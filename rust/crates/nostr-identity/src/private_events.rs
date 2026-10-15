//! Ordinary Nostr events without signatures. Admission requires authenticated private transport.
use nostr_sdk::{Event, EventId, Kind, PublicKey, Tags, Timestamp, UnsignedEvent};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivateEvent {
    pub id: EventId,
    pub pubkey: PublicKey,
    pub created_at: Timestamp,
    pub kind: Kind,
    pub tags: Tags,
    pub content: String,
}
impl PrivateEvent {
    pub fn from_unsigned(event: UnsignedEvent) -> Self {
        Self {
            id: EventId::new(
                &event.pubkey,
                &event.created_at,
                &event.kind,
                &event.tags,
                &event.content,
            ),
            pubkey: event.pubkey,
            created_at: event.created_at,
            kind: event.kind,
            tags: event.tags,
            content: event.content,
        }
    }
    /// Verify content integrity; this is not proof of authorship.
    pub fn verify(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.id
                == EventId::new(
                    &self.pubkey,
                    &self.created_at,
                    &self.kind,
                    &self.tags,
                    &self.content
                ),
            "Invalid private event ID"
        );
        Ok(())
    }
}
impl<'de> Deserialize<'de> for PrivateEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("sig").is_some() {
            let event: Event =
                serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
            event.verify().map_err(serde::de::Error::custom)?;
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            id: EventId,
            pubkey: PublicKey,
            created_at: Timestamp,
            kind: Kind,
            tags: Tags,
            content: String,
            #[serde(default)]
            sig: Option<String>,
        }
        let wire: Wire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        let _ = wire.sig;
        let event = Self {
            id: wire.id,
            pubkey: wire.pubkey,
            created_at: wire.created_at,
            kind: wire.kind,
            tags: wire.tags,
            content: wire.content,
        };
        event.verify().map_err(serde::de::Error::custom)?;
        Ok(event)
    }
}

pub fn build_private_setting(
    owner: PublicKey,
    setting: &str,
    target: &str,
    value: &serde_json::Value,
) -> anyhow::Result<PrivateEvent> {
    let mut value = value.clone();
    fn sort(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.sort_keys();
                for value in map.values_mut() {
                    sort(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    sort(value);
                }
            }
            _ => {}
        }
    }
    sort(&mut value);
    let draft = crate::build_fact_op_draft(
        &owner.to_hex(),
        [
            crate::fact("type", &["private_setting"]),
            crate::fact("setting", &[setting]),
            crate::fact("target", &[target]),
            crate::fact("value", &[&serde_json::to_string(&value)?]),
        ],
    )?;
    let tags = draft
        .tags
        .into_iter()
        .map(nostr_sdk::Tag::parse)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PrivateEvent::from_unsigned(UnsignedEvent::new(
        owner,
        Timestamp::from(0),
        Kind::Custom(draft.kind),
        tags,
        draft.content,
    )))
}
pub fn parse_private_setting(
    event: &PrivateEvent,
    owner: PublicKey,
) -> anyhow::Result<(String, String, serde_json::Value)> {
    event.verify()?;
    anyhow::ensure!(
        event.pubkey == owner
            && event.kind.as_u16() == 7368
            && event.created_at.as_secs() == 0
            && event.content.is_empty()
            && event.tags.len() <= 8,
        "Invalid private setting"
    );
    let tag = |name: &str| {
        event
            .tags
            .iter()
            .find(|tag| tag.as_slice().first().is_some_and(|key| key == name))
            .and_then(|tag| tag.as_slice().get(1))
    };
    anyhow::ensure!(
        tag("type").map(String::as_str) == Some("private_setting"),
        "Invalid private setting type"
    );
    let setting = tag("setting").ok_or_else(|| anyhow::anyhow!("Missing setting"))?;
    let target = tag("target").ok_or_else(|| anyhow::anyhow!("Missing target"))?;
    let value: serde_json::Value =
        serde_json::from_str(tag("value").ok_or_else(|| anyhow::anyhow!("Missing value"))?)?;
    anyhow::ensure!(
        build_private_setting(owner, setting, target, &value)? == *event,
        "Noncanonical private setting"
    );
    Ok((setting.clone(), target.clone(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_signature_is_checked_then_removed() {
        let keys = nostr_sdk::Keys::generate();
        let signed = nostr_sdk::EventBuilder::new(Kind::Custom(10454), "legacy")
            .sign_with_keys(&keys)
            .unwrap();
        let mut wire = serde_json::to_value(&signed).unwrap();
        let event: PrivateEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(event.id, signed.id);
        assert!(serde_json::to_value(&event).unwrap().get("sig").is_none());
        wire["sig"] = serde_json::json!("0".repeat(128));
        assert!(serde_json::from_value::<PrivateEvent>(wire).is_err());
    }
    #[test]
    fn setting_is_an_ordinary_unsigned_event() {
        let owner = nostr_sdk::Keys::generate().public_key();
        let event = build_private_setting(
            owner,
            "chatDeletion",
            "a",
            &serde_json::json!({"id":"a","deletedAt":100}),
        )
        .unwrap();
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire.as_object().unwrap().len(), 6);
        let unsigned: UnsignedEvent = serde_json::from_value(wire).unwrap();
        unsigned.verify_id().unwrap();
        assert_eq!(
            parse_private_setting(&event, owner).unwrap().0,
            "chatDeletion"
        );
        let mut changed = event.clone();
        changed.content = "tampered".into();
        assert!(parse_private_setting(&changed, owner).is_err());
    }
}
