use nostr_social_graph_server::{ServerConfig, load_graph_read_only, run};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    if std::env::args().any(|argument| argument == "--check-checkpoint") {
        match load_graph_read_only(
            &config.root,
            &config.graph_db_dir,
            config.legacy_graph_binary_path.as_deref(),
        ) {
            Ok(graph) => {
                let stats = graph.size();
                let revisions = graph.get_list_revisions();
                println!(
                    "{}",
                    serde_json::json!({"verified": true, "root": graph.get_root(),
                    "users": stats.users, "follows": stats.follows, "mutes": stats.mutes,
                    "follow_revisions": revisions.iter().filter(|(_, kind, _)| *kind == 3).count(),
                    "mute_revisions": revisions.iter().filter(|(_, kind, _)| *kind == 10000).count()})
                );
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if let Err(error) = run(config).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
