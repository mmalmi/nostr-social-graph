use std::{fs,path::Path};

#[test]
fn packaged_fixtures_match_shared_repository_fixtures_when_available() {
    let package=Path::new(env!("CARGO_MANIFEST_DIR"));
    let shared=package.join("../../../fixtures");
    // A registry package intentionally has no parent repository fixture tree.
    if !shared.is_dir() { return; }
    for name in ["contact-memory.json","private-contact-sync.json","private-contact-sync-v2.json"] {
        assert_eq!(fs::read(package.join("fixtures").join(name)).unwrap(),fs::read(shared.join(name)).unwrap(),"fixture drift: {name}");
    }
}
