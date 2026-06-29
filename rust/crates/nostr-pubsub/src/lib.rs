//! Minimal in-process pubsub primitives for Nostr event routing.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use nostr::Event;
pub use nostr::{Filter, PublicKey};

pub const CAP_HASHTREE_FETCH: &str = "hashtree.fetch";

pub type Result<T> = std::result::Result<T, PubsubError>;

#[derive(Debug, thiserror::Error)]
pub enum PubsubError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct VerifiedEvent {
    event: Event,
}

impl VerifiedEvent {
    pub fn as_event(&self) -> &Event {
        &self.event
    }
}

impl TryFrom<Event> for VerifiedEvent {
    type Error = PubsubError;

    fn try_from(event: Event) -> Result<Self> {
        Ok(Self { event })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub String);

impl SourceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventSourceKind {
    Peer,
    FipsEndpoint,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventSource {
    pub id: SourceId,
    pub kind: EventSourceKind,
    pub url: Option<String>,
}

impl EventSource {
    pub fn peer(id: impl Into<String>) -> Self {
        Self {
            id: SourceId::new(id),
            kind: EventSourceKind::Peer,
            url: None,
        }
    }

    pub fn relay(url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            id: SourceId::new(url.clone()),
            kind: EventSourceKind::Relay,
            url: Some(url),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow { priority: i32 },
    Throttle { priority: i32, reason: String },
    Drop { reason: String },
}

impl PolicyDecision {
    pub fn allow_with_priority(priority: i32) -> Self {
        Self::Allow { priority }
    }

    pub fn throttle(priority: i32, reason: impl Into<String>) -> Self {
        Self::Throttle {
            priority,
            reason: reason.into(),
        }
    }

    pub fn drop(reason: impl Into<String>) -> Self {
        Self::Drop {
            reason: reason.into(),
        }
    }
}

pub struct EventPolicyContext<'a> {
    pub event: &'a VerifiedEvent,
    pub source: &'a EventSource,
}

pub struct SourcePolicyContext<'a> {
    pub candidate: &'a SourceCandidate,
    pub author_pubkey: Option<&'a str>,
    pub capabilities: &'a [String],
}

#[async_trait]
pub trait PubsubPolicy: Send + Sync {
    async fn check_event(&self, context: EventPolicyContext<'_>) -> Result<PolicyDecision>;
    async fn check_source(&self, context: SourcePolicyContext<'_>) -> Result<PolicyDecision>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceHealth {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCandidate {
    pub source: EventSource,
    pub priority: i32,
    pub reason: Option<String>,
    pub freshness_hint: Option<u64>,
    pub health: SourceHealth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub accepted: bool,
    pub priority: i32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QueryEvent {
    pub event: VerifiedEvent,
    pub source: EventSource,
    pub priority: i32,
}

#[derive(Debug, Clone, Default)]
pub struct QueryReport {
    pub events: Vec<QueryEvent>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QueryOptions {
    pub limit: Option<usize>,
}

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: VerifiedEvent, source: EventSource) -> Result<PublishReport>;
    async fn query(&self, filters: Vec<Filter>, options: QueryOptions) -> Result<QueryReport>;
}

#[derive(Clone)]
struct StoredEvent {
    event: VerifiedEvent,
    source: EventSource,
    priority: i32,
}

#[derive(Clone, Default)]
pub struct InMemoryEventBus {
    events: Arc<RwLock<Vec<StoredEvent>>>,
    policy: Option<Arc<dyn PubsubPolicy>>,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_policy(policy: Arc<dyn PubsubPolicy>) -> Self {
        Self {
            events: Arc::default(),
            policy: Some(policy),
        }
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, event: VerifiedEvent, source: EventSource) -> Result<PublishReport> {
        let decision = if let Some(policy) = &self.policy {
            policy
                .check_event(EventPolicyContext {
                    event: &event,
                    source: &source,
                })
                .await?
        } else {
            PolicyDecision::allow_with_priority(0)
        };

        let (accepted, priority, reason) = report_parts(&decision);
        if accepted {
            self.events
                .write()
                .map_err(|_| PubsubError::Storage("event bus lock poisoned".to_string()))?
                .push(StoredEvent {
                    event,
                    source,
                    priority,
                });
        }

        Ok(PublishReport {
            accepted,
            priority,
            reason,
        })
    }

    async fn query(&self, filters: Vec<Filter>, options: QueryOptions) -> Result<QueryReport> {
        let limit = options.limit.or_else(|| filter_limit(&filters));
        let events = self
            .events
            .read()
            .map_err(|_| PubsubError::Storage("event bus lock poisoned".to_string()))?
            .iter()
            .filter(|stored| filters_match(&filters, stored.event.as_event()))
            .take(limit.unwrap_or(usize::MAX))
            .map(|stored| QueryEvent {
                event: stored.event.clone(),
                source: stored.source.clone(),
                priority: stored.priority,
            })
            .collect();

        Ok(QueryReport { events })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRoute {
    pub id: String,
    pub source: EventSource,
    pub priority: i32,
    pub reason: Option<String>,
    pub capabilities: Vec<String>,
}

impl SourceRoute {
    pub fn fips_peer(id: impl Into<String>, priority: i32) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            source: EventSource {
                id: SourceId::new(id.clone()),
                kind: EventSourceKind::FipsEndpoint,
                url: None,
            },
            priority,
            reason: None,
            capabilities: Vec::new(),
        }
    }
}

pub struct RouteQuerySource<'a> {
    pub route: SourceRoute,
    bus: &'a dyn EventBus,
}

impl<'a> RouteQuerySource<'a> {
    pub fn new<B>(route: SourceRoute, bus: &'a B) -> Self
    where
        B: EventBus + 'a,
    {
        Self { route, bus }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RoutedQueryOptions {
    pub query: QueryOptions,
}

#[derive(Debug, Clone)]
pub struct RouteAttempt {
    pub route: SourceRoute,
    pub decision: PolicyDecision,
}

#[derive(Debug, Clone, Default)]
pub struct RoutedQueryReport {
    pub events: Vec<QueryEvent>,
    pub attempts: Vec<RouteAttempt>,
}

pub async fn query_routes_with_policy<P>(
    routes: &[RouteQuerySource<'_>],
    filters: Vec<Filter>,
    options: RoutedQueryOptions,
    author_pubkey: Option<&str>,
    policy: &P,
    capabilities: Option<&[String]>,
) -> Result<RoutedQueryReport>
where
    P: PubsubPolicy + ?Sized,
{
    let mut candidates = Vec::new();
    for route_source in routes {
        let route = &route_source.route;
        let capabilities = capabilities.unwrap_or(&route.capabilities);
        let candidate = SourceCandidate {
            source: route.source.clone(),
            priority: route.priority,
            reason: route.reason.clone(),
            freshness_hint: None,
            health: SourceHealth::default(),
        };
        let decision = policy
            .check_source(SourcePolicyContext {
                candidate: &candidate,
                author_pubkey,
                capabilities,
            })
            .await?;
        if matches!(decision, PolicyDecision::Drop { .. }) {
            continue;
        }
        let effective_priority = route.priority.saturating_add(decision_priority(&decision));
        candidates.push((effective_priority, route_source, decision));
    }

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

    let mut report = RoutedQueryReport::default();
    let limit = options.query.limit.unwrap_or(usize::MAX);
    for (_, route_source, decision) in candidates {
        if report.events.len() >= limit {
            break;
        }
        report.attempts.push(RouteAttempt {
            route: route_source.route.clone(),
            decision,
        });
        let remaining = limit.saturating_sub(report.events.len());
        let mut route_options = options.query;
        route_options.limit = Some(route_options.limit.unwrap_or(remaining).min(remaining));
        let mut route_report = route_source
            .bus
            .query(filters.clone(), route_options)
            .await?;
        report.events.append(&mut route_report.events);
    }

    Ok(report)
}

fn report_parts(decision: &PolicyDecision) -> (bool, i32, Option<String>) {
    match decision {
        PolicyDecision::Allow { priority } => (true, *priority, None),
        PolicyDecision::Throttle { priority, reason } => (true, *priority, Some(reason.clone())),
        PolicyDecision::Drop { reason } => (false, 0, Some(reason.clone())),
    }
}

fn decision_priority(decision: &PolicyDecision) -> i32 {
    match decision {
        PolicyDecision::Allow { priority } | PolicyDecision::Throttle { priority, .. } => *priority,
        PolicyDecision::Drop { .. } => 0,
    }
}

fn filters_match(filters: &[Filter], event: &Event) -> bool {
    filters.is_empty() || filters.iter().any(|filter| filter_matches(filter, event))
}

fn filter_matches(filter: &Filter, event: &Event) -> bool {
    if filter
        .ids
        .as_ref()
        .is_some_and(|ids| !ids.contains(&event.id))
    {
        return false;
    }
    if filter
        .authors
        .as_ref()
        .is_some_and(|authors| !authors.contains(&event.pubkey))
    {
        return false;
    }
    if filter
        .kinds
        .as_ref()
        .is_some_and(|kinds| !kinds.contains(&event.kind))
    {
        return false;
    }
    if filter.since.is_some_and(|since| event.created_at < since) {
        return false;
    }
    if filter.until.is_some_and(|until| event.created_at > until) {
        return false;
    }
    true
}

fn filter_limit(filters: &[Filter]) -> Option<usize> {
    filters.iter().filter_map(|filter| filter.limit).min()
}
