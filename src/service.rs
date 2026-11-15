//! Access checks, validation, and scoping in front of the graph. MCP tools and the CLI call this.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use thiserror::Error;

use crate::access::{self, Actor, Intent};
use crate::config::{self, Config};
use crate::embed::MODEL_ID;
use crate::identity::repo_key;
use crate::model::{EntityInput, MemoryType, RelationInput, Scope, SCHEMA_VERSION};
use crate::secrets;
use crate::store::{Graph, RememberRequest, Remembered, StoreError};

/// A request could not be served.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// Invalid input or configuration.
    #[error("{0}")]
    Message(String),
    /// The graph failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A write as the caller states it, before access checks and scoping.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    /// The user asked for this write. Required in `read_write` mode.
    pub explicit: bool,
    /// One-line title.
    pub title: String,
    /// One-line summary.
    pub summary: String,
    /// Statement body.
    pub content: String,
    /// Memory type name, see [`MemoryType`].
    pub memory_type: String,
    /// `global` or `repository`.
    pub scope: String,
    /// Repository path. Defaults to the current directory.
    pub project_path: Option<String>,
    /// Update this memory in place.
    pub id: Option<String>,
    /// Mark this memory superseded by the new one.
    pub supersedes: Option<String>,
    /// Entities the statement is about.
    pub entities: Vec<EntityInput>,
    /// Typed links between those entities.
    pub relations: Vec<RelationInput>,
}

/// The memory service over one graph.
pub struct Service {
    graph: Graph,
    config_path: PathBuf,
    /// Duplicate detection reads before it writes, so concurrent writers would
    /// both miss each other. Writes take this lock; reads do not.
    writes: tokio::sync::Mutex<()>,
}

impl Service {
    /// Load config (writing defaults on first run) and open the on-disk graph.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or written, or the graph cannot be opened.
    pub async fn open(config_path: PathBuf) -> Result<Self, ServiceError> {
        let config = Config::load(&config_path).map_err(ServiceError::Message)?;
        std::fs::create_dir_all(&config.data_dir).map_err(|error| ServiceError::Message(error.to_string()))?;
        if !config_path.exists() {
            config.save(&config_path).map_err(ServiceError::Message)?;
        }
        let graph = Box::pin(Graph::open_disk(&config.data_dir.join("helix"))).await?;
        Ok(Self::open_graph(graph, config_path))
    }

    /// Wrap an already open graph. Config is still read from `config_path` on each request.
    pub fn open_graph(graph: Graph, config_path: PathBuf) -> Self {
        Self {
            graph,
            config_path,
            writes: tokio::sync::Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Config, ServiceError> {
        Config::load(&self.config_path).map_err(ServiceError::Message)
    }

    /// Where this service reads its config.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Model, schema, and access summary. The graph answered if the service exists.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read.
    pub fn health(&self) -> Result<Value, ServiceError> {
        let config = self.load()?;
        Ok(json!({
            "helix": "ok",
            "schema_version": SCHEMA_VERSION,
            "embedding_model": MODEL_ID,
            "embedding_dim": crate::embed::EMBED_DIM,
            "access": config.access.as_str(),
            "space_id": config.space_id,
            "vector_leg": "token-hash",
        }))
    }

    /// Check access, bounds, secrets, and trust, then write one memory.
    /// A refusal is an `Ok` result with `action: rejected` so the agent can read the reason.
    ///
    /// # Errors
    ///
    /// Fails on an unknown type or scope, unreadable config, or a graph failure.
    pub async fn remember(&self, actor: Actor, draft: Draft) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let intent = if draft.explicit {
            Intent::ExplicitWrite
        } else {
            Intent::AutomaticWrite
        };
        if let Err(denied) = access::authorize(config.access, actor, intent) {
            return Ok(rejected(denied.reason));
        }
        if let Err(reason) = config::check_bounds(&draft.title, &draft.summary, &draft.content) {
            return Ok(rejected(&reason));
        }
        if secrets::looks_like_secret(&format!("{}\n{}\n{}", draft.title, draft.summary, draft.content)) {
            tracing::info!("rejected secret-shaped memory");
            return Ok(rejected(
                "content looks like a secret and was not stored",
            ));
        }
        let memory_type = MemoryType::parse(&draft.memory_type)
            .ok_or_else(|| ServiceError::Message(format!("unknown type {}", draft.memory_type)))?;
        let scope = Scope::parse(&draft.scope)
            .ok_or_else(|| ServiceError::Message(format!("unknown scope {}", draft.scope)))?;
        let repo = Self::repo(&config, draft.project_path.as_deref());
        if config.require_trust && !config.allows_repo(&repo) && actor == Actor::Agent {
            return Ok(rejected("repository is not trusted"));
        }
        if draft
            .relations
            .iter()
            .any(|relation| crate::model::relation_kind(&relation.kind).is_none())
        {
            return Ok(rejected("unknown relation kind"));
        }
        if draft.entities.is_empty() && draft.relations.is_empty() && memory_type.expects_entities() {
            tracing::info!("memory stored without entities");
        }
        let _write = self.writes.lock().await;
        let remembered = self
            .graph
            .remember(RememberRequest {
                id: draft.id,
                supersedes: draft.supersedes,
                title: draft.title,
                summary: draft.summary,
                content: draft.content,
                memory_type,
                scope,
                repo_key: repo,
                space_id: config.space_id,
                principal_id: config.principal_id,
                source: if draft.explicit { "user" } else { "automatic" }.into(),
                entities: draft.entities,
                relations: draft.relations,
            })
            .await?;
        Ok(match remembered {
            Remembered::Created { id } => json!({"action": "created", "id": id, "titles": []}),
            Remembered::Updated { id } => json!({"action": "updated", "id": id}),
            Remembered::Superseded { id } => json!({"action": "superseded", "id": id}),
            Remembered::Duplicate { id } => json!({"action": "duplicate", "id": id}),
        })
    }

    /// Recall active memories for a query: global plus the repository at `project_path`.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn recall(
        &self,
        actor: Actor,
        query: &str,
        project_path: Option<&str>,
        types: &[String],
    ) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::Read) {
            return Ok(json!({"action": "disabled", "reason": denied.reason, "hits": []}));
        }
        let repo = Self::repo(&config, project_path);
        let hits = self
            .graph
            .recall(
                &config.space_id,
                &[repo],
                query,
                types,
                config.recall_limit,
                config.recall_bytes,
            )
            .await?;
        let titles: Vec<&str> = hits.iter().map(|hit| hit.title.as_str()).collect();
        Ok(json!({
            "action": "recalled",
            "ranking": "keyword+vector+graph",
            "vector_leg": "token-hash",
            "titles": titles,
            "hits": hits,
        }))
    }

    /// Delete one memory by id. Agents need a write-capable mode; the CLI always may.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn forget(&self, actor: Actor, id: &str) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::ExplicitWrite) {
            return Ok(rejected(denied.reason));
        }
        let _write = self.writes.lock().await;
        let removed = self
            .graph
            .forget(&config.space_id, &config.principal_id, id)
            .await?;
        if removed {
            Ok(json!({"action": "deleted", "id": id}))
        } else {
            Ok(json!({"action": "rejected", "reason": "memory not found", "id": id}))
        }
    }

    /// Browse stored memories with optional filters. This is not task recall.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn list(
        &self,
        actor: Actor,
        scope: Option<&str>,
        memory_type: Option<&str>,
        text: Option<&str>,
        project_path: Option<&str>,
    ) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::Read) {
            return Ok(json!({"action": "disabled", "reason": denied.reason, "memories": []}));
        }
        let repo = project_path.map(|path| Self::repo(&config, Some(path)));
        let memories = self
            .graph
            .list(
                &config.space_id,
                scope,
                repo.as_deref(),
                memory_type,
                text,
            )
            .await?;
        Ok(json!({"action": "listed", "memories": memories}))
    }

    /// One memory by id.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn show(&self, actor: Actor, id: &str) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::Read) {
            return Ok(json!({"action": "disabled", "reason": denied.reason}));
        }
        match self.graph.get(&config.space_id, id).await? {
            Some(memory) => Ok(json!({"action": "listed", "memory": memory})),
            None => Ok(json!({"action": "rejected", "reason": "memory not found"})),
        }
    }

    /// Entities and memories linked to an entity name.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn graph(&self, actor: Actor, entity: &str) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::Read) {
            return Ok(json!({"action": "disabled", "reason": denied.reason, "items": []}));
        }
        let items = self.graph.neighbors(&config.space_id, entity).await?;
        Ok(json!({"action": "recalled", "entity": entity, "items": items}))
    }

    /// Titles and summaries for global memories and the repository, about 2 KB at most.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn digest(&self, project_path: Option<&str>) -> Result<String, ServiceError> {
        let config = self.load()?;
        if access::authorize(config.access, Actor::Agent, Intent::Read).is_err() {
            return Ok("memory is off".into());
        }
        let repo = Self::repo(&config, project_path);
        let memories = self
            .graph
            .list(&config.space_id, None, Some(&repo), None, None)
            .await?;
        let mut lines = Vec::new();
        let mut used = 0usize;
        for memory in memories.into_iter().filter(|memory| memory.status == "active") {
            let line = format!("- {} ({}) {}", memory.title, memory.memory_type, memory.summary);
            if used + line.len() > 2000 {
                break;
            }
            used += line.len();
            lines.push(line);
        }
        if lines.is_empty() {
            Ok("No memories yet.".into())
        } else {
            Ok(lines.join("\n"))
        }
    }

    /// The space as an export document. Embeddings are omitted.
    ///
    /// # Errors
    ///
    /// Fails when config cannot be read or the graph fails.
    pub async fn export_json(&self) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let document = self.graph.export_space(&config.space_id).await?;
        serde_json::to_value(document).map_err(|error| ServiceError::Message(error.to_string()))
    }

    /// Import an export document into the space, rebuilding embeddings.
    ///
    /// # Errors
    ///
    /// Fails when the document does not parse, config cannot be read, or the graph fails.
    pub async fn import_json(&self, value: Value) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let document = serde_json::from_value(value).map_err(|error| ServiceError::Message(error.to_string()))?;
        let _write = self.writes.lock().await;
        let count = self
            .graph
            .import_space(&config.space_id, &config.principal_id, document)
            .await?;
        Ok(json!({"action": "imported", "count": count}))
    }

    fn repo(config: &Config, project_path: Option<&str>) -> String {
        let path = project_path.map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            PathBuf::from,
        );
        repo_key(&path, &config.repo_overrides)
    }
}

fn rejected(reason: &str) -> Value {
    json!({"action": "rejected", "reason": reason})
}
