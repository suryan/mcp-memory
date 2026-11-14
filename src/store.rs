use std::path::Path;

use helix_db::dsl::prelude::*;
use helix_db::{Client, HelixDbSource};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::embed::{self, EMBED_DIM};
use crate::model::{
    EntityInput, EntityRecord, ExportDocument, MemoryRecord, MemoryType, RelationInput, Scope,
    SCHEMA_VERSION,
};
use crate::rank::{self, Candidate, RankedHit};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("graph: {0}")]
    Graph(String),
    #[error("{0}")]
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct RememberRequest {
    pub id: Option<String>,
    pub supersedes: Option<String>,
    pub title: String,
    pub summary: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub scope: Scope,
    pub repo_key: String,
    pub space_id: String,
    pub principal_id: String,
    pub source: String,
    pub entities: Vec<EntityInput>,
    pub relations: Vec<RelationInput>,
}

#[derive(Debug, Clone)]
pub enum Remembered {
    Created { id: String },
    Updated { id: String },
    Superseded { id: String },
    Duplicate { id: String },
}

pub struct Graph {
    client: Client,
}

impl Graph {
    pub async fn open_disk(root: &Path) -> Result<Self, StoreError> {
        silence_telemetry();
        std::fs::create_dir_all(root).map_err(|error| StoreError::Graph(error.to_string()))?;
        let client = Client::open(HelixDbSource::Disk {
            root: root.to_path_buf(),
            database: "memory".into(),
        })
        .await
        .map_err(graph_err)?;
        let graph = Self { client };
        graph.ensure_schema().await?;
        Ok(graph)
    }

    pub async fn open_memory(name: &str) -> Result<Self, StoreError> {
        silence_telemetry();
        let client = Client::open(HelixDbSource::InMemory {
            database: name.into(),
        })
        .await
        .map_err(graph_err)?;
        let graph = Self { client };
        graph.ensure_schema().await?;
        Ok(graph)
    }

    pub async fn ensure_schema(&self) -> Result<(), StoreError> {
        let dimension = std::num::NonZeroUsize::new(EMBED_DIM).expect("dimension");
        self.build_index(
            "vector",
            g().create_vector_index_nodes(
                "Memory",
                "embedding",
                dimension,
                VectorDistanceMetric::Cosine,
                Some("space_id"),
            ),
        )
        .await?;
        self.build_index(
            "title",
            g().create_text_index_nodes("Memory", "title", Some("space_id")),
        )
        .await?;
        self.build_index(
            "summary",
            g().create_text_index_nodes("Memory", "summary", Some("space_id")),
        )
        .await?;
        self.build_index(
            "content",
            g().create_text_index_nodes("Memory", "content", Some("space_id")),
        )
        .await?;
        self.build_index(
            "entity_name",
            g().create_text_index_nodes("Entity", "name", Some("space_id")),
        )
        .await?;
        Ok(())
    }

    async fn build_index(
        &self,
        name: &str,
        traversal: Traversal<Terminal, WriteEnabled>,
    ) -> Result<(), StoreError> {
        let value = self
            .execute(QueryRequest::write(
                write_batch().var_as(name, traversal).returning([name]),
            ))
            .await?;
        let receipt = value.get(name).cloned().unwrap_or(Value::Null);
        if receipt.get("kind").and_then(Value::as_str) == Some("already_active") {
            return Ok(());
        }
        let Some(operation_id) = receipt
            .get("operation_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return Err(StoreError::Graph(format!("index {name} receipt: {receipt}")));
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let status = self
                .execute(QueryRequest::read(
                    read_batch()
                        .var_as("status", g().get_index_operation(operation_id.clone()))
                        .returning(["status"]),
                ))
                .await?;
            let state = status
                .pointer("/status/status")
                .and_then(Value::as_str)
                .unwrap_or("");
            match state {
                "succeeded" => return Ok(()),
                "aborted" | "blocked" => {
                    return Err(StoreError::Graph(format!("index {name} {state}: {status}")));
                }
                _ => {
                    if std::time::Instant::now() > deadline {
                        return Err(StoreError::Graph(format!("index {name} timed out: {status}")));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            }
        }
    }

    pub async fn remember(&self, request: RememberRequest) -> Result<Remembered, StoreError> {
        let hash = content_hash(&request.title, &request.summary, &request.content);
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(id) = &request.id {
            self.update_memory(id, &request, &hash, &now).await?;
            self.link_entities(id, &request).await?;
            return Ok(Remembered::Updated { id: id.clone() });
        }
        if request.supersedes.is_none() {
            if let Some(id) = self.find_duplicate(&request).await? {
                return Ok(Remembered::Duplicate { id });
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let vector = embed::embed(&format!(
            "{} {} {}",
            request.title, request.summary, request.content
        ));
        let repo_key = if request.scope == Scope::Global {
            String::new()
        } else {
            request.repo_key.clone()
        };
        let query = QueryRequest::write(
            write_batch()
                .var_as(
                    "created",
                    g().add_n(
                        "Memory",
                        vec![
                            ("memory_id", PropertyInput::from(id.clone())),
                            ("title", PropertyInput::from(request.title.clone())),
                            ("summary", PropertyInput::from(request.summary.clone())),
                            ("content", PropertyInput::from(request.content.clone())),
                            ("memory_type", PropertyInput::from(request.memory_type.as_str())),
                            ("scope", PropertyInput::from(request.scope.as_str())),
                            ("repo_key", PropertyInput::from(repo_key)),
                            ("space_id", PropertyInput::from(request.space_id.clone())),
                            ("principal_id", PropertyInput::from(request.principal_id.clone())),
                            ("status", PropertyInput::from("active")),
                            ("content_hash", PropertyInput::from(hash)),
                            ("created_at", PropertyInput::from(now.clone())),
                            ("updated_at", PropertyInput::from(now)),
                            ("source", PropertyInput::from(request.source.clone())),
                            ("embedding", PropertyInput::from(vector)),
                        ],
                    )
                    .value_map(Some(vec!["memory_id"])),
                )
                .returning(["created"]),
        );
        self.execute(query).await?;
        if let Some(previous) = &request.supersedes {
            self.mark_superseded(previous, &id, &request.space_id).await?;
        }
        self.link_entities(&id, &request).await?;
        self.audit("created", &id, &request.space_id, &request.principal_id)
            .await?;
        if request.supersedes.is_some() {
            Ok(Remembered::Superseded { id })
        } else {
            Ok(Remembered::Created { id })
        }
    }

    pub async fn recall(
        &self,
        space_id: &str,
        repos: &[String],
        query: &str,
        type_filter: &[String],
        limit: usize,
        byte_budget: usize,
    ) -> Result<Vec<RankedHit>, StoreError> {
        let vector = embed::embed(query);
        let filter = memory_filter(space_id, repos, type_filter);
        let tenant = Some(PropertyInput::from(space_id.to_string()));
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "vector_hits",
                    g().n_with_label_where("Memory", filter.clone())
                        .vector_search_with(
                            "Memory",
                            "embedding",
                            vector,
                            limit.saturating_mul(3).max(8),
                            tenant.clone(),
                        )
                        .value_map(Some(memory_fields())),
                )
                .var_as(
                    "title_hits",
                    g().n_with_label_where("Memory", filter.clone())
                        .text_search_with(
                            "Memory",
                            "title",
                            query.to_string(),
                            limit.saturating_mul(3).max(8),
                            tenant.clone(),
                        )
                        .value_map(Some(memory_fields())),
                )
                .var_as(
                    "summary_hits",
                    g().n_with_label_where("Memory", filter.clone())
                        .text_search_with(
                            "Memory",
                            "summary",
                            query.to_string(),
                            limit.saturating_mul(3).max(8),
                            tenant.clone(),
                        )
                        .value_map(Some(memory_fields())),
                )
                .var_as(
                    "content_hits",
                    g().n_with_label_where("Memory", filter)
                        .text_search_with(
                            "Memory",
                            "content",
                            query.to_string(),
                            limit.saturating_mul(3).max(8),
                            tenant.clone(),
                        )
                        .value_map(Some(memory_fields())),
                )
                .var_as(
                    "graph_hits",
                    g().text_search_nodes(
                        "Entity",
                        "name",
                        query.to_string(),
                        8,
                        Some(PropertyValue::from(space_id.to_string())),
                    )
                    .in_(Some("ABOUT"))
                    .value_map(Some(memory_fields())),
                )
                .var_as(
                    "graph_hops",
                    g().text_search_nodes(
                        "Entity",
                        "name",
                        query.to_string(),
                        8,
                        Some(PropertyValue::from(space_id.to_string())),
                    )
                    .out(None::<&str>)
                    .in_(Some("ABOUT"))
                    .value_map(Some(memory_fields())),
                )
                .returning([
                    "vector_hits",
                    "title_hits",
                    "summary_hits",
                    "content_hits",
                    "graph_hits",
                    "graph_hops",
                ]),
        );
        let value = self.execute(request).await?;
        let mut candidates = Vec::new();
        candidates.extend(candidates_from(&value, "vector_hits", true, false, space_id, repos));
        for key in ["title_hits", "summary_hits", "content_hits"] {
            candidates.extend(candidates_from(&value, key, false, false, space_id, repos));
        }
        candidates.extend(candidates_from(&value, "graph_hits", false, true, space_id, repos));
        candidates.extend(candidates_from(&value, "graph_hops", false, true, space_id, repos));
        Ok(rank::rank(candidates, limit, byte_budget))
    }

    pub async fn list(
        &self,
        space_id: &str,
        scope: Option<&str>,
        repo_key: Option<&str>,
        memory_type: Option<&str>,
        text: Option<&str>,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "rows",
                    g().n_with_label_where("Memory", Predicate::eq("space_id", space_id.to_string()))
                        .value_map(Some(memory_fields())),
                )
                .returning(["rows"]),
        );
        let value = self.execute(request).await?;
        let needle = text.unwrap_or("").to_ascii_lowercase();
        let mut rows = Vec::new();
        for row in array(&value, "rows") {
            let record = record_from(&row);
            if record.space_id != space_id {
                continue;
            }
            if let Some(scope) = scope {
                if record.scope != scope {
                    continue;
                }
            }
            if let Some(repo_key) = repo_key {
                if record.repo_key != repo_key && record.scope != "global" {
                    continue;
                }
            }
            if let Some(memory_type) = memory_type {
                if record.memory_type != memory_type {
                    continue;
                }
            }
            if !needle.is_empty() {
                let haystack = format!("{} {} {}", record.title, record.summary, record.content)
                    .to_ascii_lowercase();
                if !haystack.contains(&needle) {
                    continue;
                }
            }
            rows.push(record);
        }
        rows.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        rows.truncate(200);
        Ok(rows)
    }

    pub async fn get(&self, space_id: &str, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "rows",
                    g().n_with_label_where("Memory", Predicate::eq("memory_id", id.to_string()))
                        .value_map(Some(memory_fields())),
                )
                .returning(["rows"]),
        );
        let value = self.execute(request).await?;
        Ok(array(&value, "rows")
            .into_iter()
            .map(|row| record_from(&row))
            .find(|record| record.space_id == space_id))
    }

    pub async fn forget(&self, space_id: &str, principal_id: &str, id: &str) -> Result<bool, StoreError> {
        let Some(_) = self.get(space_id, id).await? else {
            return Ok(false);
        };
        let request = QueryRequest::write(
            write_batch()
                .var_as(
                    "removed",
                    g().n_with_label_where(
                        "Memory",
                        Predicate::and(vec![
                            Predicate::eq("memory_id", id.to_string()),
                            Predicate::eq("space_id", space_id.to_string()),
                        ]),
                    )
                    .drop(),
                )
                .returning(["removed"]),
        );
        self.execute(request).await?;
        self.audit("deleted", id, space_id, principal_id).await?;
        Ok(true)
    }

    pub async fn neighbors(
        &self,
        space_id: &str,
        name: &str,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "related",
                    g().n_with_label_where(
                        "Entity",
                        Predicate::and(vec![
                            Predicate::eq("space_id", space_id.to_string()),
                            Predicate::eq("name_key", name_key(name)),
                        ]),
                    )
                    .out(None::<&str>)
                    .value_map(Some(vec!["name", "kind", "entity_id"])),
                )
                .var_as(
                    "memories",
                    g().n_with_label_where(
                        "Entity",
                        Predicate::and(vec![
                            Predicate::eq("space_id", space_id.to_string()),
                            Predicate::eq("name_key", name_key(name)),
                        ]),
                    )
                    .in_(Some("ABOUT"))
                    .value_map(Some(vec!["memory_id", "title", "summary", "status"])),
                )
                .returning(["related", "memories"]),
        );
        let value = self.execute(request).await?;
        let mut items = array(&value, "related");
        items.extend(array(&value, "memories"));
        Ok(items)
    }

    pub async fn export_space(&self, space_id: &str) -> Result<ExportDocument, StoreError> {
        let memories = self.list(space_id, None, None, None, None).await?;
        let entities = self.list_entities(space_id).await?;
        Ok(ExportDocument {
            version: SCHEMA_VERSION,
            memories,
            entities,
            edges: Vec::new(),
        })
    }

    pub async fn import_space(
        &self,
        space_id: &str,
        principal_id: &str,
        document: ExportDocument,
    ) -> Result<usize, StoreError> {
        let mut imported = 0usize;
        for memory in document.memories {
            if memory.space_id != space_id {
                continue;
            }
            if let Some(existing) = self.get(space_id, &memory.id).await? {
                if existing.content_hash != memory.content_hash {
                    self.set_status(space_id, &memory.id, "conflicted").await?;
                }
                continue;
            }
            let memory_type = MemoryType::parse(&memory.memory_type).unwrap_or(MemoryType::Fact);
            let scope = Scope::parse(&memory.scope).unwrap_or(Scope::Global);
            let request = RememberRequest {
                id: Some(memory.id.clone()),
                supersedes: None,
                title: memory.title,
                summary: memory.summary,
                content: memory.content,
                memory_type,
                scope,
                repo_key: memory.repo_key,
                space_id: space_id.into(),
                principal_id: principal_id.into(),
                source: "import".into(),
                entities: Vec::new(),
                relations: Vec::new(),
            };
            // id is set, so remember updates. Create when missing.
            self.insert_with_id(&memory.id, &request, &memory.content_hash, &memory.created_at)
                .await?;
            imported += 1;
        }
        let _ = document.entities;
        Ok(imported)
    }

    async fn insert_with_id(
        &self,
        id: &str,
        request: &RememberRequest,
        hash: &str,
        created_at: &str,
    ) -> Result<(), StoreError> {
        let vector = embed::embed(&format!(
            "{} {} {}",
            request.title, request.summary, request.content
        ));
        let now = chrono::Utc::now().to_rfc3339();
        let query = QueryRequest::write(
            write_batch()
                .var_as(
                    "created",
                    g().add_n(
                        "Memory",
                        vec![
                            ("memory_id", PropertyInput::from(id.to_string())),
                            ("title", PropertyInput::from(request.title.clone())),
                            ("summary", PropertyInput::from(request.summary.clone())),
                            ("content", PropertyInput::from(request.content.clone())),
                            ("memory_type", PropertyInput::from(request.memory_type.as_str())),
                            ("scope", PropertyInput::from(request.scope.as_str())),
                            ("repo_key", PropertyInput::from(request.repo_key.clone())),
                            ("space_id", PropertyInput::from(request.space_id.clone())),
                            ("principal_id", PropertyInput::from(request.principal_id.clone())),
                            ("status", PropertyInput::from("active")),
                            ("content_hash", PropertyInput::from(hash.to_string())),
                            ("created_at", PropertyInput::from(created_at.to_string())),
                            ("updated_at", PropertyInput::from(now)),
                            ("source", PropertyInput::from(request.source.clone())),
                            ("embedding", PropertyInput::from(vector)),
                        ],
                    )
                    .count(),
                )
                .returning(["created"]),
        );
        self.execute(query).await?;
        Ok(())
    }

    async fn update_memory(
        &self,
        id: &str,
        request: &RememberRequest,
        hash: &str,
        now: &str,
    ) -> Result<(), StoreError> {
        let vector = embed::embed(&format!(
            "{} {} {}",
            request.title, request.summary, request.content
        ));
        let repo_key = if request.scope == Scope::Global {
            String::new()
        } else {
            request.repo_key.clone()
        };
        let query = QueryRequest::write(
            write_batch()
                .var_as(
                    "updated",
                    g().n_with_label_where(
                        "Memory",
                        Predicate::and(vec![
                            Predicate::eq("memory_id", id.to_string()),
                            Predicate::eq("space_id", request.space_id.clone()),
                        ]),
                    )
                    .set_property("title", request.title.clone())
                    .set_property("summary", request.summary.clone())
                    .set_property("content", request.content.clone())
                    .set_property("memory_type", request.memory_type.as_str())
                    .set_property("scope", request.scope.as_str())
                    .set_property("repo_key", repo_key)
                    .set_property("content_hash", hash.to_string())
                    .set_property("updated_at", now.to_string())
                    .set_property("embedding", vector)
                    .set_property("status", "active")
                    .count(),
                )
                .returning(["updated"]),
        );
        self.execute(query).await?;
        Ok(())
    }

    async fn mark_superseded(
        &self,
        previous: &str,
        id: &str,
        space_id: &str,
    ) -> Result<(), StoreError> {
        let query = QueryRequest::write(
            write_batch()
                .var_as(
                    "old",
                    g().n_with_label_where(
                        "Memory",
                        Predicate::and(vec![
                            Predicate::eq("memory_id", previous.to_string()),
                            Predicate::eq("space_id", space_id.to_string()),
                        ]),
                    )
                    .set_property("status", "superseded"),
                )
                .var_as(
                    "edge",
                    g().n_with_label_where("Memory", Predicate::eq("memory_id", id.to_string()))
                        .add_e(
                            "SUPERSEDES",
                            NodeRef::var("old"),
                            vec![("evidence_id", PropertyInput::from(id.to_string()))],
                        )
                        .count(),
                )
                .returning(["edge"]),
        );
        self.execute(query).await?;
        Ok(())
    }

    async fn find_duplicate(&self, request: &RememberRequest) -> Result<Option<String>, StoreError> {
        let vector = embed::embed(&format!(
            "{} {} {}",
            request.title, request.summary, request.content
        ));
        let tenant = Some(PropertyValue::from(request.space_id.clone()));
        let query = QueryRequest::read(
            read_batch()
                .var_as(
                    "hits",
                    g().vector_search_nodes("Memory", "embedding", vector, 3, tenant)
                        .value_map(Some(vec![
                            "memory_id",
                            "memory_type",
                            "scope",
                            "repo_key",
                            "status",
                            "space_id",
                            "$distance",
                        ])),
                )
                .returning(["hits"]),
        );
        let value = self.execute(query).await?;
        for row in array(&value, "hits") {
            if field(&row, "space_id") != request.space_id || field(&row, "status") != "active" {
                continue;
            }
            if field(&row, "memory_type") != request.memory_type.as_str() {
                continue;
            }
            if field(&row, "scope") != request.scope.as_str() {
                continue;
            }
            let distance = row
                .get("$distance")
                .and_then(|value| value.as_f64())
                .unwrap_or(1.0);
            if distance < 0.08 {
                return Ok(Some(field(&row, "memory_id")));
            }
        }
        Ok(None)
    }

    async fn link_entities(&self, memory_id: &str, request: &RememberRequest) -> Result<(), StoreError> {
        let repo_key = if request.scope == Scope::Global {
            String::new()
        } else {
            request.repo_key.clone()
        };
        for entity in &request.entities {
            let entity_id = self
                .ensure_entity(&request.space_id, &repo_key, entity)
                .await?;
            let ordered = QueryRequest::write(
                write_batch()
                    .var_as(
                        "target",
                        g().n_with_label_where("Entity", Predicate::eq("entity_id", entity_id)),
                    )
                    .var_as(
                        "link",
                        g().n_with_label_where(
                            "Memory",
                            Predicate::eq("memory_id", memory_id.to_string()),
                        )
                        .add_e(
                            "ABOUT",
                            NodeRef::var("target"),
                            vec![("evidence_id", PropertyInput::from(memory_id.to_string()))],
                        )
                        .count(),
                    )
                    .returning(["link"]),
            );
            self.execute(ordered).await?;
        }
        for relation in &request.relations {
            let kind = crate::model::relation_kind(&relation.kind).unwrap_or("REQUIRES");
            let from = self
                .ensure_entity(
                    &request.space_id,
                    &repo_key,
                    &EntityInput {
                        name: relation.from.clone(),
                        kind: "other".into(),
                    },
                )
                .await?;
            let to = self
                .ensure_entity(
                    &request.space_id,
                    &repo_key,
                    &EntityInput {
                        name: relation.to.clone(),
                        kind: "other".into(),
                    },
                )
                .await?;
            let query = QueryRequest::write(
                write_batch()
                    .var_as(
                        "from_node",
                        g().n_with_label_where("Entity", Predicate::eq("entity_id", from)),
                    )
                    .var_as(
                        "to_node",
                        g().n_with_label_where("Entity", Predicate::eq("entity_id", to)),
                    )
                    .var_as(
                        "edge",
                        g().n(NodeRef::var("from_node"))
                            .add_e(
                                kind,
                                NodeRef::var("to_node"),
                                vec![("evidence_id", PropertyInput::from(memory_id.to_string()))],
                            )
                            .count(),
                    )
                    .returning(["edge"]),
            );
            self.execute(query).await?;
        }
        Ok(())
    }

    async fn ensure_entity(
        &self,
        space_id: &str,
        repo_key: &str,
        entity: &EntityInput,
    ) -> Result<String, StoreError> {
        let key = name_key(&entity.name);
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "rows",
                    g().n_with_label_where(
                        "Entity",
                        Predicate::and(vec![
                            Predicate::eq("space_id", space_id.to_string()),
                            Predicate::eq("repo_key", repo_key.to_string()),
                            Predicate::eq("name_key", key.clone()),
                        ]),
                    )
                    .value_map(Some(vec!["entity_id"])),
                )
                .returning(["rows"]),
        );
        let value = self.execute(request).await?;
        if let Some(row) = array(&value, "rows").into_iter().next() {
            let id = field(&row, "entity_id");
            if !id.is_empty() {
                return Ok(id);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let request = QueryRequest::write(
            write_batch()
                .var_as(
                    "created",
                    g().add_n(
                        "Entity",
                        vec![
                            ("entity_id", PropertyInput::from(id.clone())),
                            ("name", PropertyInput::from(entity.name.trim().to_string())),
                            ("name_key", PropertyInput::from(key)),
                            ("kind", PropertyInput::from(entity.kind.clone())),
                            ("space_id", PropertyInput::from(space_id.to_string())),
                            ("repo_key", PropertyInput::from(repo_key.to_string())),
                        ],
                    )
                    .count(),
                )
                .returning(["created"]),
        );
        self.execute(request).await?;
        Ok(id)
    }

    async fn list_entities(&self, space_id: &str) -> Result<Vec<EntityRecord>, StoreError> {
        let request = QueryRequest::read(
            read_batch()
                .var_as(
                    "rows",
                    g().n_with_label_where("Entity", Predicate::eq("space_id", space_id.to_string()))
                        .value_map(Some(vec![
                            "entity_id",
                            "name",
                            "kind",
                            "space_id",
                            "repo_key",
                        ])),
                )
                .returning(["rows"]),
        );
        let value = self.execute(request).await?;
        Ok(array(&value, "rows")
            .into_iter()
            .map(|row| EntityRecord {
                id: field(&row, "entity_id"),
                name: field(&row, "name"),
                kind: field(&row, "kind"),
                space_id: field(&row, "space_id"),
                repo_key: field(&row, "repo_key"),
            })
            .collect())
    }

    async fn set_status(&self, space_id: &str, id: &str, status: &str) -> Result<(), StoreError> {
        let request = QueryRequest::write(
            write_batch()
                .var_as(
                    "updated",
                    g().n_with_label_where(
                        "Memory",
                        Predicate::and(vec![
                            Predicate::eq("memory_id", id.to_string()),
                            Predicate::eq("space_id", space_id.to_string()),
                        ]),
                    )
                    .set_property("status", status.to_string())
                    .count(),
                )
                .returning(["updated"]),
        );
        self.execute(request).await?;
        Ok(())
    }

    async fn audit(
        &self,
        action: &str,
        memory_id: &str,
        space_id: &str,
        principal_id: &str,
    ) -> Result<(), StoreError> {
        let request = QueryRequest::write(
            write_batch()
                .var_as(
                    "audit",
                    g().add_n(
                        "Audit",
                        vec![
                            ("action", PropertyInput::from(action.to_string())),
                            ("memory_id", PropertyInput::from(memory_id.to_string())),
                            ("space_id", PropertyInput::from(space_id.to_string())),
                            ("principal_id", PropertyInput::from(principal_id.to_string())),
                            ("at", PropertyInput::from(chrono::Utc::now().to_rfc3339())),
                        ],
                    )
                    .count(),
                )
                .returning(["audit"]),
        );
        self.execute(request).await?;
        tracing::info!(action, memory_id, space_id, "memory audit");
        Ok(())
    }

    async fn execute(&self, request: QueryRequest) -> Result<Value, StoreError> {
        self.client
            .query(request)
            .send()
            .await
            .map_err(graph_err)
    }
}

fn silence_telemetry() {
    if std::env::var_os("HELIX_TELEMETRY_LEVEL").is_none() {
        // Embedded Helix sends basic telemetry unless this is set. Memory text stays local.
        std::env::set_var("HELIX_TELEMETRY_LEVEL", "off");
    }
}

fn graph_err(error: impl std::fmt::Display) -> StoreError {
    StoreError::Graph(error.to_string())
}

fn content_hash(title: &str, summary: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(summary.as_bytes());
    hasher.update([0]);
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn memory_fields() -> Vec<&'static str> {
    vec![
        "memory_id",
        "title",
        "summary",
        "content",
        "memory_type",
        "scope",
        "repo_key",
        "space_id",
        "principal_id",
        "status",
        "content_hash",
        "created_at",
        "updated_at",
        "source",
        "$distance",
        "$score",
    ]
}

fn memory_filter(space_id: &str, repos: &[String], type_filter: &[String]) -> Predicate {
    let mut scope = vec![Predicate::eq("scope", "global")];
    for repo in repos {
        scope.push(Predicate::eq("repo_key", repo.clone()));
    }
    let mut parts = vec![
        Predicate::eq("space_id", space_id.to_string()),
        Predicate::eq("status", "active"),
        Predicate::or(scope),
    ];
    if !type_filter.is_empty() {
        let types = type_filter
            .iter()
            .map(|value| Predicate::eq("memory_type", value.clone()))
            .collect();
        parts.push(Predicate::or(types));
    }
    Predicate::and(parts)
}

fn array(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(|item| item.as_array())
        .cloned()
        .unwrap_or_default()
}

fn field(row: &Value, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn record_from(row: &Value) -> MemoryRecord {
    MemoryRecord {
        id: field(row, "memory_id"),
        title: field(row, "title"),
        summary: field(row, "summary"),
        content: field(row, "content"),
        memory_type: field(row, "memory_type"),
        scope: field(row, "scope"),
        repo_key: field(row, "repo_key"),
        space_id: field(row, "space_id"),
        principal_id: field(row, "principal_id"),
        status: field(row, "status"),
        content_hash: field(row, "content_hash"),
        created_at: field(row, "created_at"),
        updated_at: field(row, "updated_at"),
        source: field(row, "source"),
    }
}

fn candidates_from(
    value: &Value,
    key: &str,
    vector: bool,
    graph: bool,
    space_id: &str,
    repos: &[String],
) -> Vec<Candidate> {
    array(value, key)
        .into_iter()
        .filter_map(|row| {
            let record = record_from(&row);
            if record.space_id != space_id || record.status != "active" || record.id.is_empty() {
                return None;
            }
            if record.scope != "global" && !repos.iter().any(|repo| repo == &record.repo_key) {
                return None;
            }
            let vector_distance = if vector {
                row.get("$distance").and_then(|item| item.as_f64()).map(|item| item as f32)
            } else {
                None
            };
            let text_score = if !vector && !graph {
                row.get("$score").and_then(|item| item.as_f64()).map(|item| item as f32)
            } else {
                None
            };
            Some(Candidate {
                id: record.id,
                title: record.title,
                summary: record.summary,
                content: record.content,
                memory_type: record.memory_type,
                scope: record.scope,
                repo_key: record.repo_key,
                vector_distance,
                text_score,
                from_graph: graph,
            })
        })
        .collect()
}
