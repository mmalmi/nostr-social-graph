#[allow(dead_code)]
mod external_social_graph {
    include!(env!("NOTIFIER_PARSER_SOURCE"));
}

use std::{env, error::Error, fs};

use external_social_graph::ExternalSocialGraph;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let [path, root, current, other, stable]: [String; 5] = env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "expected snapshot, root, current friend, other friend, stable friend")?;
    let bytes = fs::read(path)?;
    assert_eq!(bytes.first(), Some(&3), "expected v3 snapshot");
    let graph = ExternalSocialGraph::from_social_graph_binary(&root, &bytes)?;

    assert!(graph.is_pubkey_in_graph(&root)?, "missing root");
    assert!(
        graph.is_pubkey_in_graph(&current)?,
        "missing current follow"
    );
    assert!(
        graph.recipient_has_muted_author(&root, &current)?,
        "missing current mute"
    );
    assert!(
        !graph.is_author_visible(&root, &current)?,
        "muted author is visible"
    );
    assert!(
        !graph.is_pubkey_in_graph(&other)?,
        "unexpected reachable friend"
    );
    assert!(
        !graph.recipient_has_muted_author(&root, &other)?,
        "unexpected mute"
    );
    assert!(
        !graph.is_author_visible(&root, &other)?,
        "unexpected visible friend"
    );
    assert!(graph.is_pubkey_in_graph(&stable)?, "missing stable follow");
    assert!(
        !graph.recipient_has_muted_author(&root, &stable)?,
        "stable follow is muted"
    );
    assert!(
        graph.is_author_visible(&root, &stable)?,
        "stable follow is hidden"
    );

    println!(
        r#"{{"passed":true,"version":3,"follow":true,"mute":true,"visibility":true,"otherAbsent":true}}"#
    );
    Ok(())
}
