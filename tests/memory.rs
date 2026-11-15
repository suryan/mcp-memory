//! Service behaviour over an in-memory graph: recall, access modes, secrets, and supersession.

#![allow(clippy::unwrap_used, reason = "helpers set up fixtures; a failure there fails the test")]

use std::path::PathBuf;

use serde_json::json;

use mcp_memory::access::Actor;
use mcp_memory::config::{self, Config};
use mcp_memory::model::{EntityInput, RelationInput};
use mcp_memory::service::{Draft, Service};
use mcp_memory::store::Graph;

fn config_for(dir: &std::path::Path, access: mcp_memory::access::Access) -> (Config, PathBuf) {
    let config = Config {
        access,
        data_dir: dir.to_path_buf(),
        space_id: "personal".into(),
        ..Config::default()
    };
    let path = dir.join("config.toml");
    config.save(&path).unwrap();
    (config, path)
}

async fn service(dir: &std::path::Path, access: mcp_memory::access::Access) -> Service {
    let (_config, path) = config_for(dir, access);
    let graph = Graph::open_memory(&format!("test-{}", uuid::Uuid::new_v4())).await.unwrap();
    Service::open_graph(graph, path)
}

#[tokio::test]
async fn recall_uses_a_stored_fact_and_skips_another_repo() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let created = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: false,
                title: "Seed the database".into(),
                summary: "Integration tests fail until the local database is seeded".into(),
                content: "Run the seed command before integration tests.".into(),
                memory_type: "fact".into(),
                scope: "repository".into(),
                project_path: Some("/tmp/repo-a".into()),
                entities: vec![EntityInput {
                    name: "integration tests".into(),
                    kind: "command".into(),
                }],
                ..Draft::default()
            },
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
    assert_ne!(hit["hits"], json!([]));

    let other = service
        .recall(
            Actor::Agent,
            "integration tests",
            Some("/tmp/repo-b"),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(other["hits"], json!([]));
}

#[tokio::test]
async fn read_only_rejects_agent_writes_and_human_delete_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let created = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Prefer small diffs".into(),
                summary: "Keep edits local to the change".into(),
                content: "Do not refactor unrelated code.".into(),
                memory_type: "preference".into(),
                scope: "global".into(),
                ..Draft::default()
            },
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
            Draft {
                explicit: true,
                title: "Another preference".into(),
                summary: "summary".into(),
                content: "content".into(),
                memory_type: "preference".into(),
                scope: "global".into(),
                ..Draft::default()
            },
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
            Draft {
                explicit: true,
                title: "token".into(),
                summary: "do not keep".into(),
                content: "sk-live-abcdefghijklmnopqrstuvwxyz".into(),
                memory_type: "fact".into(),
                scope: "global".into(),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");
    let listed = service
        .list(Actor::Human, None, None, Some("token"), None)
        .await
        .unwrap();
    assert_eq!(listed["memories"], json!([]));
}

#[tokio::test]
async fn second_statement_supersedes_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let first = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Formatter".into(),
                summary: "Use the old formatter".into(),
                content: "Run the legacy formatter.".into(),
                memory_type: "correction".into(),
                scope: "global".into(),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    let previous = first["id"].as_str().unwrap().to_string();
    let second = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Formatter".into(),
                summary: "Use rustfmt".into(),
                content: "Run rustfmt before committing.".into(),
                memory_type: "correction".into(),
                scope: "global".into(),
                supersedes: Some(previous),
                ..Draft::default()
            },
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
            Draft {
                explicit: false,
                title: "Auto".into(),
                summary: "summary".into(),
                content: "content".into(),
                memory_type: "fact".into(),
                scope: "global".into(),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");
    let created = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Explicit".into(),
                summary: "summary".into(),
                content: "The user asked to remember this.".into(),
                memory_type: "fact".into(),
                scope: "global".into(),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(created["action"], "created");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_identical_writes_store_one_memory() {
    let dir = tempfile::tempdir().unwrap();
    let service = std::sync::Arc::new(service(dir.path(), mcp_memory::access::Access::Learn).await);
    let writes = (0..8).map(|_| {
        let service = std::sync::Arc::clone(&service);
        tokio::spawn(async move {
            service
                .remember(
                    Actor::Agent,
                    Draft {
                        explicit: true,
                        title: "Package manager".into(),
                        summary: "Use pnpm for node projects".into(),
                        content: "Install and run scripts with pnpm.".into(),
                        memory_type: "preference".into(),
                        scope: "global".into(),
                        ..Draft::default()
                    },
                )
                .await
                .unwrap()
        })
    });
    let mut created = 0;
    for write in writes {
        if write.await.unwrap()["action"] == "created" {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    let listed = service
        .list(Actor::Human, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(listed["memories"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_relation_kind_names_the_allowed_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let rejected = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Precommit runs cargo".into(),
                summary: "summary".into(),
                content: "scripts/precommit.sh runs cargo.".into(),
                memory_type: "fact".into(),
                scope: "global".into(),
                relations: vec![RelationInput {
                    from: "scripts/precommit.sh".into(),
                    to: "cargo".into(),
                    kind: "uses".into(),
                }],
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rejected["action"], "rejected");
    let reason = rejected["reason"].as_str().unwrap();
    assert!(reason.contains("\"uses\""), "{reason}");
    for kind in mcp_memory::model::RELATION_KINDS {
        assert!(reason.contains(kind), "{kind} missing from {reason}");
    }
}

#[tokio::test]
async fn without_a_path_writes_are_global_and_recall_skips_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path(), mcp_memory::access::Access::Learn).await;
    let pathless = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Lint before commit".into(),
                summary: "Run the linter before every commit".into(),
                content: "Run the linter before every commit.".into(),
                memory_type: "convention".into(),
                scope: "repository".into(),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(pathless["action"], "created");
    assert_eq!(pathless["scope"], "global");
    assert!(pathless["note"].is_string(), "{pathless}");

    let scoped = service
        .remember(
            Actor::Agent,
            Draft {
                explicit: true,
                title: "Seed before tests".into(),
                summary: "Seed the database before integration tests".into(),
                content: "Seed the database before integration tests.".into(),
                memory_type: "fact".into(),
                scope: "repository".into(),
                project_path: Some("/tmp/repo-a".into()),
                ..Draft::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(scoped["scope"], "repository");
    assert!(scoped.get("note").is_none(), "{scoped}");

    let titles = |value: &serde_json::Value| -> Vec<String> {
        value["titles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|title| title.as_str().unwrap().to_string())
            .collect()
    };
    let global_only = service
        .recall(Actor::Agent, "seed database integration tests lint commit", None, &[])
        .await
        .unwrap();
    assert_eq!(titles(&global_only), vec!["Lint before commit"]);

    let with_repo = service
        .recall(
            Actor::Agent,
            "seed database integration tests lint commit",
            Some("/tmp/repo-a"),
            &[],
        )
        .await
        .unwrap();
    let mut found = titles(&with_repo);
    found.sort();
    assert_eq!(found, vec!["Lint before commit", "Seed before tests"]);

    let digest = service.digest().await.unwrap();
    assert!(digest.contains("Lint before commit"), "{digest}");
    assert!(!digest.contains("Seed before tests"), "{digest}");
}
