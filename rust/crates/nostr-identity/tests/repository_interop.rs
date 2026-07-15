use nostr_identity::*;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, EventBuilder, JsonUtil, Keys, Kind, SecretKey, Tag};
use uuid::Uuid;

const SUBJECT: &str = "6b7f5df4-1d2d-43a7-9b87-873e41a2d99a";

fn subject() -> Uuid {
    Uuid::parse_str(SUBJECT).unwrap()
}

fn fixed_keys(byte: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[byte; 32]).unwrap())
}

#[test]
fn parses_shared_ts_rust_identity_link_request_fixture() {
    let device_keys = fixed_keys(1);
    let invite_keys = fixed_keys(2);
    let device_pubkey = device_keys.public_key().to_hex();
    let invite_pubkey = invite_keys.public_key().to_hex();
    let event = Event::from_json(include_str!(
        "../../../../testdata/identity-link-request.json"
    ))
    .unwrap();
    let parsed = parse_identity_link_request_event(&event, &invite_keys).unwrap();

    assert_eq!(parsed.signer_pubkey, device_pubkey);
    assert_eq!(parsed.content.identity, subject());
    assert_eq!(
        parsed.content.admin_pubkey,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(parsed.content.invite_pubkey, invite_pubkey);
    assert_eq!(parsed.content.joining_pubkey, device_pubkey);
    assert_eq!(parsed.content.client_nonce, "fixture-link-request");
    assert_eq!(parsed.content.requested_at, 1_720_000_021);
    assert_eq!(parsed.content.label, Some("Fixture Phone".to_owned()));
}

#[test]
fn fips_transport_identity_fixture_matches_rust_identity() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/fips-transport-identity-v1.json"
    ))
    .unwrap();
    let transport = Keys::new(
        SecretKey::from_hex(fixture["keys"]["transportSecretKey"].as_str().unwrap()).unwrap(),
    );

    assert_eq!(fixture["version"], 1);
    assert_eq!(fixture["identity"], SUBJECT);
    assert_eq!(
        fixture["purpose"].as_str(),
        Some(IDENTITY_PURPOSE_FIPS_TRANSPORT)
    );
    assert_eq!(
        fixture["keys"]["transportPubkey"],
        transport.public_key().to_hex()
    );
}

#[test]
fn shared_ts_rust_device_approval_vectors_reject_tampering() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../testdata/nostr-identity-device-approval-v1.json"
    ))
    .unwrap();
    let request = NostrIdentityDeviceApprovalRequest {
        request_pubkey: fixture["keys"]["requestPubkey"]
            .as_str()
            .unwrap()
            .to_string(),
        device_app_key_pubkey: fixture["keys"]["deviceAppKeyPubkey"]
            .as_str()
            .unwrap()
            .to_string(),
        request_secret: fixture["requestSecret"].as_str().unwrap().to_string(),
        device_app_key_proof: fixture["proofEvent"].to_string(),
        requested_at: fixture["requestedAt"].as_i64().unwrap(),
        request_type: fixture["requestType"].as_str().map(str::to_string),
        resources: serde_json::from_value(fixture["resources"].clone()).unwrap(),
        expires_at: fixture["expiresAt"].as_i64(),
        profile_id: Some(fixture["profileId"].as_str().unwrap().parse().unwrap()),
        admin_app_key_pubkey: Some(fixture["keys"]["adminPubkey"].as_str().unwrap().to_string()),
        label: fixture["label"].as_str().map(str::to_string),
    };
    assert_eq!(request.request_pubkey, fixture["keys"]["requestPubkey"]);
    assert_eq!(
        request.device_app_key_pubkey,
        fixture["keys"]["deviceAppKeyPubkey"]
    );
    assert_eq!(request.request_secret, fixture["requestSecret"]);
    assert_eq!(
        request.profile_id.unwrap().to_string(),
        fixture["profileId"]
    );
    assert_eq!(
        nostr_identity_device_approval_request_relays(&request).unwrap(),
        vec!["wss://temp.iris.to".to_string()]
    );

    let proof = Event::from_json(&request.device_app_key_proof).unwrap();
    assert_eq!(proof.id.to_hex(), fixture["proofEvent"]["id"]);
    assert!(proof.content.is_empty());

    let request_keys = Keys::new(
        SecretKey::from_hex(fixture["keys"]["requestSecretKey"].as_str().unwrap()).unwrap(),
    );
    let bootstrap = nostr_identity_device_approval_bootstrap(&request).unwrap();
    let prefix = fixture["prefix"].as_str().unwrap();
    let bootstrap_uri =
        encode_nostr_identity_device_approval_bootstrap(&bootstrap, Some(prefix)).unwrap();
    assert_eq!(
        parse_nostr_identity_device_approval_bootstrap(&bootstrap_uri, &[])
            .unwrap()
            .unwrap(),
        bootstrap
    );
    assert!(
        parse_nostr_identity_device_approval_bootstrap(
            fixture["fullRequest"].as_str().unwrap(),
            &[],
        )
        .is_err(),
        "legacy full request URI was accepted"
    );
    let request_event =
        build_nostr_identity_device_approval_request_event(&request_keys, &request).unwrap();
    let request_event_content: serde_json::Value =
        serde_json::from_str(&request_event.content).unwrap();
    assert_eq!(
        request_event_content["requestSecretCommitment"],
        "55aeef52b9f3641a8546bd60f0861a5b52f0952df10fecf000d6651b68c3a3ba"
    );
    assert_eq!(
        parse_nostr_identity_device_approval_request_event(&request_event, &bootstrap).unwrap(),
        request
    );
    assert!(!request_event.as_json().contains(&request.request_secret));

    let receipt_event = Event::from_json(fixture["receiptEvent"].to_string()).unwrap();
    let receipt = parse_nostr_identity_device_approval_receipt_event_for_request(
        &receipt_event,
        &request_keys,
        &request,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(receipt).unwrap(),
        fixture["expectedReceipt"]
    );

    let mut request_payload: serde_json::Value =
        serde_json::from_str(&request_event.content).unwrap();
    request_payload["unexpected"] = true.into();
    let unknown_content = EventBuilder::new(Kind::from(FACT_OP_KIND), request_payload.to_string())
        .tags(request_event.tags.iter().cloned())
        .custom_created_at(request_event.created_at)
        .sign_with_keys(&request_keys)
        .unwrap();
    assert!(
        parse_nostr_identity_device_approval_request_event(&unknown_content, &bootstrap,).is_err()
    );

    let mut request_payload: serde_json::Value =
        serde_json::from_str(&request_event.content).unwrap();
    request_payload["resources"][0]["scopes"][0] = "admin".into();
    let changed_resource = EventBuilder::new(Kind::from(FACT_OP_KIND), request_payload.to_string())
        .tags(request_event.tags.iter().cloned())
        .custom_created_at(request_event.created_at)
        .sign_with_keys(&request_keys)
        .unwrap();
    assert!(
        parse_nostr_identity_device_approval_request_event(&changed_resource, &bootstrap,).is_err()
    );

    let device_keys = Keys::new(
        SecretKey::from_hex(fixture["keys"]["deviceAppKeySecretKey"].as_str().unwrap()).unwrap(),
    );
    let proof_fixture = Event::from_json(fixture["proofEvent"].to_string()).unwrap();
    for tamper in fixture["tamperCases"]["proof"].as_array().unwrap() {
        let mut tags = proof_fixture.tags.iter().cloned().collect::<Vec<_>>();
        if let Some(tag) = tamper["tag"].as_array() {
            tags.push(Tag::parse([tag[0].as_str().unwrap(), tag[1].as_str().unwrap()]).unwrap());
        }
        let proof = EventBuilder::new(
            Kind::from(FACT_OP_KIND),
            tamper["content"].as_str().unwrap_or(""),
        )
        .tags(tags)
        .custom_created_at(proof_fixture.created_at)
        .sign_with_keys(&device_keys)
        .unwrap();
        let mut tampered = request.clone();
        tampered.device_app_key_proof = proof.as_json();
        assert!(
            build_nostr_identity_device_approval_request_event(&request_keys, &tampered).is_err(),
            "accepted proof tamper {}",
            tamper["name"]
        );
    }

    let admin_keys = Keys::new(
        SecretKey::from_hex(fixture["keys"]["adminSecretKey"].as_str().unwrap()).unwrap(),
    );
    for tamper in fixture["tamperCases"]["receipt"].as_array().unwrap() {
        let event = if let Some(field) = tamper["eventField"].as_str() {
            let mut event = fixture["receiptEvent"].clone();
            event
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), tamper["value"].clone());
            Event::from_json(event.to_string()).unwrap()
        } else if let Some(tag) = tamper["tag"].as_array() {
            let mut tags = receipt_event.tags.iter().cloned().collect::<Vec<_>>();
            tags.push(Tag::parse([tag[0].as_str().unwrap(), tag[1].as_str().unwrap()]).unwrap());
            EventBuilder::new(Kind::from(FACT_OP_KIND), receipt_event.content.clone())
                .tags(tags)
                .custom_created_at(receipt_event.created_at)
                .sign_with_keys(&admin_keys)
                .unwrap()
        } else {
            let field = tamper["field"].as_str().unwrap();
            let mut receipt = fixture["expectedReceipt"].clone();
            receipt
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), tamper["value"].clone());
            let encrypted = nip44::encrypt(
                admin_keys.secret_key(),
                &request_keys.public_key(),
                receipt.to_string(),
                Nip44Version::V2,
            )
            .unwrap();
            let profile_id = fixture["profileId"].as_str().unwrap();
            EventBuilder::new(Kind::from(FACT_OP_KIND), encrypted)
                .tag(Tag::parse(["type", NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_TYPE]).unwrap())
                .tag(Tag::parse(["p", request.request_pubkey.as_str()]).unwrap())
                .tag(Tag::parse(["i", profile_id, "subject"]).unwrap())
                .custom_created_at(receipt_event.created_at)
                .sign_with_keys(&admin_keys)
                .unwrap()
        };
        assert!(
            parse_nostr_identity_device_approval_receipt_event_for_request(
                &event,
                &request_keys,
                &request,
            )
            .is_err(),
            "accepted receipt tamper {}",
            tamper["name"]
        );
    }
}
