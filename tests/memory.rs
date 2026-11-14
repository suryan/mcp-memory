use std::path::PathBuf;

use mcp_memory::access::Actor;
use mcp_memory::config::{self, Config};
use mcp_memory::model::EntityInput;
use mcp_memory::service::Service;
use mcp_memory::store::Graph;

fn config_for(dir: &std::path::Path, access: mcp_memory::access::Access) -> (Config, PathBuf) {
    let mut config = Config::default();
    config.access = access;
    config.data_dir = dir.to_path_buf();
    config.space_id = "personal".into();
    let path = dir.join("config.toml");
    config.save(&path).unwrap();
    (config, path)
}

async fn service(dir: &std::path::Path, access: mcp_memory::access::Access) -> Service {
    let (_config, path) = config_for(dir, access);
    let graph = Graph::open_memory(&format!("test-{}", uuid::Uuid::new_v4())).await.unwrap();
    Service::open_graph(graph, path).await
}

#[tokio::test]
async fn recall_uses_a_stored_fact_and_skips_another_repo() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let created = service
        .remember(
            Actor::Agent,
            false,
            "Seed the database".into(),
            "Integration tests fail until the local database is seeded".into(),
            "Run the seed command before integration tests.".into(),
            "fact",
            "repository",
            Some("/tmp/repo-a"),
            None,
            None,
            vec![EntityInput {
                name: "integration tests".into(),
                kind: "command".into(),
            }],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(created["action"], "created");

    let hit = service
        .recall(
            Actor::Agent,
            "integration tests",
            Some("/tmp/repo-a"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(hit["action"], "recalled");
    assert!(!hit["hits"].as_array().unwrap().is_empty());

    let other = service
        .recall(
            Actor::Agent,
            "integration tests",
            Some("/tmp/repo-b"),
            &[],
        )
        .await
        .unwrap();
    assert!(other["hits"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn read_only_rejects_agent_writes_and_human_delete_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let created = service
        .remember(
            Actor::Agent,
            true,
            "Prefer small diffs".into(),
            "Keep edits local to the change".into(),
            "Do not refactor unrelated code.".into(),
            "preference",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let path = service.config_path().to_path_buf();
    let mut config = Config::load(&path).unwrap();
    config.access = mcp_memory::access::Access::ReadOnly;
    config.save(&path).unwrap();

    let rejected = service
        .remember(
            Actor::Agent,
            true,
            "Another preference".into(),
            "summary".into(),
            "content".into(),
            "preference",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");

    let deleted = service.forget(Actor::Human, &id).await.unwrap();
    assert_eq!(deleted["action"], "deleted");
    let missing = service.show(Actor::Human, &id).await.unwrap();
    assert_eq!(missing["action"], "rejected");
}

#[tokio::test]
async fn secret_is_rejected_and_absent() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let rejected = service
        .remember(
            Actor::Agent,
            true,
            "token".into(),
            "do not keep".into(),
            "sk-live-abcdefghijklmnopqrstuvwxyz".into(),
            "fact",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");
    let listed = service
        .list(Actor::Human, None, None, Some("token"), None)
        .await
        .unwrap();
    assert!(listed["memories"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn second_statement_supersedes_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let first = service
        .remember(
            Actor::Agent,
            true,
            "Formatter".into(),
            "Use the old formatter".into(),
            "Run the legacy formatter.".into(),
            "correction",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    let previous = first["id"].as_str().unwrap().to_string();
    let second = service
        .remember(
            Actor::Agent,
            true,
            "Formatter".into(),
            "Use rustfmt".into(),
            "Run rustfmt before committing.".into(),
            "correction",
            "global",
            None,
            None,
            Some(previous),
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(second["action"], "superseded");
    let hits = service
        .recall(Actor::Agent, "rustfmt", None, &[])
        .await
        .unwrap();
    let titles: Vec<&str> = hits["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles.iter().filter(|title| **title == "Formatter").count(), 1);
    let _ = config::config_path();
}

#[tokio::test]
async fn automatic_write_is_rejected_in_read_write_mode() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::ReadWrite).await;
    let rejected = service
        .remember(
            Actor::Agent,
            false,
            "Auto".into(),
            "summary".into(),
            "content".into(),
            "fact",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");
    let created = service
        .remember(
            Actor::Agent,
            true,
            "Explicit".into(),
            "summary".into(),
            "The user asked to remember this.".into(),
            "fact",
            "global",
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(created["action"], "created");
}
