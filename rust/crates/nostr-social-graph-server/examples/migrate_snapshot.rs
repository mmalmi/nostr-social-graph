//! Import a binary snapshot and verify it survives reopening the Hashtree store.
use std::{env, fs, path::Path};

use nostr_sdk::PublicKey;
use nostr_social_graph::SocialGraph;
use nostr_social_graph_server::load_or_bootstrap_graph;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: migrate_snapshot <snapshot.bin> <target-directory> <root-pubkey>".into(),
        );
    }
    let source = Path::new(&args[1]);
    let target = Path::new(&args[2]);
    let root = PublicKey::parse(&args[3])?.to_hex();
    let original = SocialGraph::from_binary(&root, &fs::read(source)?)?;
    let expected = original.to_binary()?;
    let imported = load_or_bootstrap_graph(&root, target, Some(source))?;
    if imported.to_binary()? != expected {
        return Err("imported graph differs from the source snapshot".into());
    }
    drop(imported);
    let reopened = load_or_bootstrap_graph(&root, target, None)?;
    if reopened.to_binary()? != expected {
        return Err("persisted graph differs after reopening".into());
    }
    let size = reopened.size();
    println!(
        "Verified import and reopen: {} users, {} follows, {} mutes",
        size.users, size.follows, size.mutes
    );
    Ok(())
}
