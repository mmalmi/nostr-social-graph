//! Nostr pubsub policy adapter for `nostr-social-graph`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use nostr_pubsub::{
    EventPolicyContext, EventSource, EventSourceKind, PolicyDecision, PublicKey, PubsubError,
    PubsubPolicy, Result, SourcePolicyContext,
};
use nostr_social_graph::SocialGraphBackend;

pub const DEFAULT_SOCIAL_GRAPH_ENTRYPOINT_NPUB: &str =
    "npub1g53mukxnjkcmr94fhryzkqutdz2ukq4ks0gvy5af25rgmwsl4ngq43drvk";
pub const DEFAULT_UNKNOWN_FOLLOW_DISTANCE: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphDistanceAction {
    Allow,
    Throttle,
    Drop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SocialGraphPolicyConfig {
    pub trusted_distance: u32,
    pub max_follow_distance: Option<u32>,
    pub unknown_follow_distance: u32,
    pub trusted_priority: i32,
    pub neutral_priority: i32,
    pub distance_priority_step: i32,
    pub outside_graph_priority: i32,
    pub missing_author_priority: i32,
    pub outside_graph_action: GraphDistanceAction,
    pub missing_author_action: GraphDistanceAction,
    pub drop_overmuted: bool,
    pub overmute_threshold: f64,
}

impl Default for SocialGraphPolicyConfig {
    fn default() -> Self {
        Self {
            trusted_distance: 2,
            max_follow_distance: None,
            unknown_follow_distance: DEFAULT_UNKNOWN_FOLLOW_DISTANCE,
            trusted_priority: 100,
            neutral_priority: 0,
            distance_priority_step: 10,
            outside_graph_priority: -100,
            missing_author_priority: 0,
            outside_graph_action: GraphDistanceAction::Throttle,
            missing_author_action: GraphDistanceAction::Allow,
            drop_overmuted: true,
            overmute_threshold: 1.0,
        }
    }
}

#[derive(Clone)]
pub struct SocialGraphPolicy<B> {
    graph: Arc<RwLock<B>>,
    config: SocialGraphPolicyConfig,
    service_reputation: Option<Arc<dyn ServiceReputation>>,
}

impl<B> SocialGraphPolicy<B> {
    pub fn new(graph: Arc<RwLock<B>>, config: SocialGraphPolicyConfig) -> Self {
        Self {
            graph,
            config,
            service_reputation: None,
        }
    }

    pub fn with_service_reputation(mut self, reputation: Arc<dyn ServiceReputation>) -> Self {
        self.service_reputation = Some(reputation);
        self
    }

    pub fn graph(&self) -> Arc<RwLock<B>> {
        self.graph.clone()
    }

    pub fn config(&self) -> &SocialGraphPolicyConfig {
        &self.config
    }
}

impl<B> SocialGraphPolicy<B>
where
    B: SocialGraphBackend + Send + Sync + 'static,
{
    fn decision_for_author(&self, author_pubkey: &str) -> Result<PolicyDecision> {
        let graph = self
            .graph
            .read()
            .map_err(|_| PubsubError::Validation("social graph lock poisoned".to_string()))?;

        if self.config.drop_overmuted
            && graph
                .is_overmuted(author_pubkey, self.config.overmute_threshold)
                .map_err(graph_policy_error)?
        {
            return Ok(PolicyDecision::drop("author overmuted by social graph"));
        }

        let distance = graph
            .get_follow_distance(author_pubkey)
            .map_err(graph_policy_error)?;
        let outside = distance >= self.config.unknown_follow_distance
            || self
                .config
                .max_follow_distance
                .is_some_and(|max_distance| distance > max_distance);

        if outside {
            return Ok(self.decision_from_action(
                self.config.outside_graph_action,
                self.config.outside_graph_priority,
                outside_reason(distance, &self.config),
            ));
        }

        Ok(PolicyDecision::allow_with_priority(
            self.priority_for_distance(distance),
        ))
    }

    fn decision_for_missing_author(&self) -> PolicyDecision {
        self.decision_from_action(
            self.config.missing_author_action,
            self.config.missing_author_priority,
            "source has no author pubkey",
        )
    }

    fn decision_from_action(
        &self,
        action: GraphDistanceAction,
        priority: i32,
        reason: impl Into<String>,
    ) -> PolicyDecision {
        match action {
            GraphDistanceAction::Allow => PolicyDecision::allow_with_priority(priority),
            GraphDistanceAction::Throttle => PolicyDecision::throttle(priority, reason),
            GraphDistanceAction::Drop => PolicyDecision::drop(reason),
        }
    }

    fn priority_for_distance(&self, distance: u32) -> i32 {
        if distance <= self.config.trusted_distance {
            let distance_penalty = i32::try_from(distance)
                .unwrap_or(i32::MAX)
                .saturating_mul(self.config.distance_priority_step);
            return self
                .config
                .trusted_priority
                .saturating_sub(distance_penalty);
        }

        self.config
            .neutral_priority
            .saturating_sub(i32::try_from(distance).unwrap_or(i32::MAX))
    }

    fn apply_service_reputation(
        &self,
        source: &EventSource,
        graph_decision: PolicyDecision,
    ) -> PolicyDecision {
        let Some(reputation) = &self.service_reputation else {
            return graph_decision;
        };
        let Some(reputation_decision) = reputation.decision_for_source(source, None) else {
            return graph_decision;
        };
        combine_source_decisions(graph_decision, reputation_decision)
    }
}

#[async_trait]
impl<B> PubsubPolicy for SocialGraphPolicy<B>
where
    B: SocialGraphBackend + Send + Sync + 'static,
{
    async fn check_event(&self, context: EventPolicyContext<'_>) -> Result<PolicyDecision> {
        self.decision_for_author(&context.event.as_event().pubkey.to_hex())
    }

    async fn check_source(&self, context: SourcePolicyContext<'_>) -> Result<PolicyDecision> {
        let graph_decision = match author_pubkey_for_source_policy(&context) {
            Some(author_pubkey) => self.decision_for_author(&author_pubkey),
            None => Ok(self.decision_for_missing_author()),
        }?;

        Ok(self.apply_service_reputation(&context.candidate.source, graph_decision))
    }
}

pub trait ServiceReputation: Send + Sync {
    fn decision_for_source(
        &self,
        source: &EventSource,
        capability: Option<&str>,
    ) -> Option<PolicyDecision>;
}

#[derive(Default)]
pub struct InMemoryServiceReputation {
    records: RwLock<BTreeMap<ServiceReputationKey, PolicyDecision>>,
}

impl InMemoryServiceReputation {
    pub fn boost_source(
        &self,
        source_id: impl Into<String>,
        capability: Option<&str>,
        priority: i32,
    ) {
        self.set_source_decision(
            source_id,
            capability,
            PolicyDecision::allow_with_priority(priority),
        );
    }

    pub fn throttle_source(
        &self,
        source_id: impl Into<String>,
        capability: Option<&str>,
        priority: i32,
        reason: impl Into<String>,
    ) {
        self.set_source_decision(
            source_id,
            capability,
            PolicyDecision::throttle(priority, reason),
        );
    }

    pub fn drop_source(
        &self,
        source_id: impl Into<String>,
        capability: Option<&str>,
        reason: impl Into<String>,
    ) {
        self.set_source_decision(source_id, capability, PolicyDecision::drop(reason));
    }

    fn set_source_decision(
        &self,
        source_id: impl Into<String>,
        capability: Option<&str>,
        decision: PolicyDecision,
    ) {
        let mut records = self
            .records
            .write()
            .expect("service reputation lock poisoned");
        records.insert(ServiceReputationKey::new(source_id, capability), decision);
    }
}

impl ServiceReputation for InMemoryServiceReputation {
    fn decision_for_source(
        &self,
        source: &EventSource,
        capability: Option<&str>,
    ) -> Option<PolicyDecision> {
        let records = self.records.read().ok()?;
        records
            .get(&ServiceReputationKey::new(source.id.0.clone(), capability))
            .or_else(|| records.get(&ServiceReputationKey::new(source.id.0.clone(), None)))
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ServiceReputationKey {
    source_id: String,
    capability: Option<String>,
}

impl ServiceReputationKey {
    fn new(source_id: impl Into<String>, capability: Option<&str>) -> Self {
        let source_id = source_id.into();
        Self {
            source_id: parse_pubkey(&source_id).unwrap_or(source_id),
            capability: capability.map(ToOwned::to_owned),
        }
    }
}

fn combine_source_decisions(
    graph_decision: PolicyDecision,
    reputation_decision: PolicyDecision,
) -> PolicyDecision {
    match (&graph_decision, &reputation_decision) {
        (PolicyDecision::Drop { reason }, _) => return PolicyDecision::drop(reason.clone()),
        (_, PolicyDecision::Drop { reason }) => return PolicyDecision::drop(reason.clone()),
        _ => {}
    }

    let priority =
        decision_priority(&graph_decision).saturating_add(decision_priority(&reputation_decision));
    let reason = decision_reason(&reputation_decision)
        .or_else(|| decision_reason(&graph_decision))
        .map(ToOwned::to_owned);

    match reputation_decision {
        PolicyDecision::Throttle { reason, .. } => PolicyDecision::throttle(priority, reason),
        PolicyDecision::Allow { .. } if priority > 0 => {
            PolicyDecision::allow_with_priority(priority)
        }
        _ => match graph_decision {
            PolicyDecision::Throttle { reason, .. } => PolicyDecision::throttle(priority, reason),
            PolicyDecision::Allow { .. } => match reason {
                Some(reason) => PolicyDecision::throttle(priority, reason),
                None => PolicyDecision::allow_with_priority(priority),
            },
            PolicyDecision::Drop { reason } => PolicyDecision::drop(reason),
        },
    }
}

fn decision_priority(decision: &PolicyDecision) -> i32 {
    match decision {
        PolicyDecision::Allow { priority } | PolicyDecision::Throttle { priority, .. } => *priority,
        PolicyDecision::Drop { .. } => 0,
    }
}

fn decision_reason(decision: &PolicyDecision) -> Option<&str> {
    match decision {
        PolicyDecision::Throttle { reason, .. } | PolicyDecision::Drop { reason } => Some(reason),
        PolicyDecision::Allow { .. } => None,
    }
}

fn author_pubkey_for_source_policy(context: &SourcePolicyContext<'_>) -> Option<String> {
    if let Some(author_pubkey) = context.author_pubkey {
        return Some(author_pubkey.to_owned());
    }

    match context.candidate.source.kind {
        EventSourceKind::Peer | EventSourceKind::FipsEndpoint => {
            parse_pubkey(&context.candidate.source.id.0)
        }
        _ => None,
    }
}

fn parse_pubkey(value: &str) -> Option<String> {
    PublicKey::parse(value).ok().map(|pubkey| pubkey.to_hex())
}

fn outside_reason(distance: u32, config: &SocialGraphPolicyConfig) -> String {
    if distance >= config.unknown_follow_distance {
        return "author outside social graph".to_string();
    }
    match config.max_follow_distance {
        Some(max_distance) => format!("author beyond allowed social graph distance {max_distance}"),
        None => "author outside social graph".to_string(),
    }
}

fn graph_policy_error(error: impl std::fmt::Display) -> PubsubError {
    PubsubError::Validation(format!("social graph policy error: {error}"))
}
