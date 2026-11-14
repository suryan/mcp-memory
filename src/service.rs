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

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub struct Service {
    graph: Graph,
    config_path: PathBuf,
}

impl Service {
    pub async fn open(config_path: PathBuf) -> Result<Self, ServiceError> {
        let config = Config::load(&config_path).map_err(ServiceError::Message)?;
        std::fs::create_dir_all(&config.data_dir).map_err(|error| ServiceError::Message(error.to_string()))?;
        if !config_path.exists() {
            config.save(&config_path).map_err(ServiceError::Message)?;
        }
        let graph = Graph::open_disk(&config.data_dir.join("helix")).await?;
        Ok(Self { graph, config_path })
    }

    pub async fn open_graph(graph: Graph, config_path: PathBuf) -> Self {
        Self { graph, config_path }
    }

    fn load(&self) -> Result<Config, ServiceError> {
        Config::load(&self.config_path).map_err(ServiceError::Message)
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub async fn health(&self) -> Result<Value, ServiceError> {
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

    pub async fn remember(
        &self,
        actor: Actor,
        explicit: bool,
        title: String,
        summary: String,
        content: String,
        memory_type: &str,
        scope: &str,
        project_path: Option<&str>,
        id: Option<String>,
        supersedes: Option<String>,
        entities: Vec<EntityInput>,
        relations: Vec<RelationInput>,
    ) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let intent = if explicit {
            Intent::ExplicitWrite
        } else {
            Intent::AutomaticWrite
        };
        if let Err(denied) = access::authorize(config.access, actor, intent) {
            return Ok(rejected(denied.reason));
        }
        if let Err(reason) = config::check_bounds(&title, &summary, &content) {
            return Ok(rejected(&reason));
        }
        if secrets::looks_like_secret(&format!("{title}\n{summary}\n{content}")) {
            tracing::info!("rejected secret-shaped memory");
            return Ok(rejected(
                "content looks like a secret and was not stored",
            ));
        }
        let memory_type = MemoryType::parse(memory_type)
            .ok_or_else(|| ServiceError::Message(format!("unknown type {memory_type}")))?;
        let scope = Scope::parse(scope)
            .ok_or_else(|| ServiceError::Message(format!("unknown scope {scope}")))?;
        let repo = self.repo(&config, project_path);
        if config.require_trust && !config.allows_repo(&repo) && actor == Actor::Agent {
            return Ok(rejected("repository is not trusted"));
        }
        for relation in &relations {
            if crate::model::relation_kind(&relation.kind).is_none() {
                return Ok(rejected("unknown relation kind"));
            }
        }
        let unlinked = entities.is_empty() && relations.is_empty() && memory_type.expects_entities();
        if unlinked {
            tracing::info!("memory stored without entities");
        }
        let remembered = self
            .graph
            .remember(RememberRequest {
                id,
                supersedes,
                title,
                summary,
                content,
                memory_type,
                scope,
                repo_key: repo,
                space_id: config.space_id,
                principal_id: config.principal_id,
                source: if explicit { "user" } else { "automatic" }.into(),
                entities,
                relations,
            })
            .await?;
        Ok(match remembered {
            Remembered::Created { id } => json!({"action": "created", "id": id, "titles": []}),
            Remembered::Updated { id } => json!({"action": "updated", "id": id}),
            Remembered::Superseded { id } => json!({"action": "superseded", "id": id}),
            Remembered::Duplicate { id } => json!({"action": "duplicate", "id": id}),
        })
    }

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
        let repo = self.repo(&config, project_path);
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

    pub async fn forget(&self, actor: Actor, id: &str) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::ExplicitWrite) {
            return Ok(rejected(denied.reason));
        }
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
        let repo = project_path.map(|path| self.repo(&config, Some(path)));
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

    pub async fn graph(&self, actor: Actor, entity: &str) -> Result<Value, ServiceError> {
        let config = self.load()?;
        if let Err(denied) = access::authorize(config.access, actor, Intent::Read) {
            return Ok(json!({"action": "disabled", "reason": denied.reason, "items": []}));
        }
        let items = self.graph.neighbors(&config.space_id, entity).await?;
        Ok(json!({"action": "recalled", "entity": entity, "items": items}))
    }

    pub async fn digest(&self, project_path: Option<&str>) -> Result<String, ServiceError> {
        let config = self.load()?;
        if access::authorize(config.access, Actor::Agent, Intent::Read).is_err() {
            return Ok("memory is off".into());
        }
        let repo = self.repo(&config, project_path);
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

    pub async fn export_json(&self) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let document = self.graph.export_space(&config.space_id).await?;
        Ok(serde_json::to_value(document).map_err(|error| ServiceError::Message(error.to_string()))?)
    }

    pub async fn import_json(&self, value: Value) -> Result<Value, ServiceError> {
        let config = self.load()?;
        let document = serde_json::from_value(value).map_err(|error| ServiceError::Message(error.to_string()))?;
        let count = self
            .graph
            .import_space(&config.space_id, &config.principal_id, document)
            .await?;
        Ok(json!({"action": "imported", "count": count}))
    }

    fn repo(&self, config: &Config, project_path: Option<&str>) -> String {
        let path = project_path
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        repo_key(&path, &config.repo_overrides)
    }
}

fn rejected(reason: &str) -> Value {
    json!({"action": "rejected", "reason": reason})
}
