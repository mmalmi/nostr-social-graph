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
    // Startup resumes crawling from the store's mtime. Import time is newer
    // than the captured data; preserve its checkpoint to cover that gap.
    preserve_capture_time(target, fs::metadata(source)?.modified()?)?;
    let size = reopened.size();
    println!(
        "Verified import and reopen: {} users, {} follows, {} mutes",
        size.users, size.follows, size.mutes
    );
    Ok(())
}

fn preserve_capture_time(path: &Path, captured: std::time::SystemTime) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            preserve_capture_time(&entry?.path(), captured)?;
        }
    }
    fs::File::open(path)?.set_times(fs::FileTimes::new().set_modified(captured))
}
