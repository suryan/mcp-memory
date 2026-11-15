//! MCP tools and the digest resource for one harness connection.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    Implementation, ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::access::Actor;
use crate::instructions::INSTRUCTIONS;
use crate::model::{EntityInput, RelationInput};
use crate::service::{Draft, Service};

/// One harness session over the shared service.
#[derive(Clone)]
pub struct MemoryServer {
    service: Arc<Service>,
    /// The harness's working directory. The shared process runs elsewhere, so
    /// its own directory says nothing about the repository in play.
    default_path: Option<String>,
}

impl MemoryServer {
    /// A session whose recall and writes default to `default_path`.
    pub fn new(service: Arc<Service>, default_path: Option<String>) -> Self {
        Self {
            service,
            default_path,
        }
    }

    fn project_path(&self, requested: Option<String>) -> Option<String> {
        requested.or_else(|| self.default_path.clone())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecallArgs {
    /// The task, in the agent's own words.
    query: String,
    /// Repository path. Defaults to the harness working directory.
    #[serde(default)]
    project_path: Option<String>,
    /// Optional memory types to keep.
    #[serde(default)]
    types: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RememberArgs {
    title: String,
    #[serde(default)]
    summary: String,
    content: String,
    #[serde(rename = "type")]
    memory_type: String,
    /// `global` or `repository`.
    scope: String,
    /// True when the user asked to store or change this memory.
    #[serde(default)]
    explicit: bool,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    supersedes: Option<String>,
    #[serde(default)]
    entities: Vec<EntityArg>,
    /// Typed links between entities. Each kind must be one of the listed relation kinds.
    #[serde(default)]
    relations: Vec<RelationArg>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EntityArg {
    name: String,
    #[serde(default = "other_kind")]
    kind: String,
}

fn other_kind() -> String {
    "other".into()
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RelationArg {
    /// Source entity name.
    from: String,
    /// Target entity name.
    to: String,
    /// One of `REQUIRES`, `PART_OF`, `RUNS_WITH`, `PREFERS`, `CORRECTS`, `RULED_OUT_FOR` (case-insensitive). The link reads `from` KIND `to`, for example `tests` `REQUIRES` `seeded database`.
    kind: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ForgetArgs {
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListArgs {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default, rename = "type")]
    memory_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GraphArgs {
    entity: String,
}

#[tool_router]
impl MemoryServer {
    #[tool(description = "Recall active memories for the current task. Returns no hits when nothing is relevant.")]
    async fn memory_recall(&self, Parameters(args): Parameters<RecallArgs>) -> String {
        let project_path = self.project_path(args.project_path);
        match self
            .service
            .recall(Actor::Agent, &args.query, project_path.as_deref(), &args.types)
            .await
        {
            Ok(value) => value.to_string(),
            Err(error) => json!({"action": "rejected", "reason": error.to_string()}).to_string(),
        }
    }

    #[tool(description = "Create, update, or supersede one memory. Set explicit=true only when the user asked to store it.")]
    async fn memory_remember(&self, Parameters(args): Parameters<RememberArgs>) -> String {
        let entities = args
            .entities
            .into_iter()
            .map(|entity| EntityInput {
                name: entity.name,
                kind: entity.kind,
            })
            .collect();
        let relations = args
            .relations
            .into_iter()
            .map(|relation| RelationInput {
                from: relation.from,
                to: relation.to,
                kind: relation.kind,
            })
            .collect();
        match self
            .service
            .remember(
                Actor::Agent,
                Draft {
                    explicit: args.explicit,
                    title: args.title,
                    summary: args.summary,
                    content: args.content,
                    memory_type: args.memory_type,
                    scope: args.scope,
                    project_path: self.project_path(args.project_path),
                    id: args.id,
                    supersedes: args.supersedes,
                    entities,
                    relations,
                },
            )
            .await
        {
            Ok(value) => value.to_string(),
            Err(error) => json!({"action": "rejected", "reason": error.to_string()}).to_string(),
        }
    }

    #[tool(description = "Delete one memory by id. There is no delete-by-query.")]
    async fn memory_forget(&self, Parameters(args): Parameters<ForgetArgs>) -> String {
        match self.service.forget(Actor::Agent, &args.id).await {
            Ok(value) => value.to_string(),
            Err(error) => json!({"action": "rejected", "reason": error.to_string()}).to_string(),
        }
    }

    #[tool(description = "List memories for the user. This is not task recall.")]
    async fn memory_list(&self, Parameters(args): Parameters<ListArgs>) -> String {
        match self
            .service
            .list(
                Actor::Agent,
                args.scope.as_deref(),
                args.memory_type.as_deref(),
                args.text.as_deref(),
                args.project_path.as_deref(),
            )
            .await
        {
            Ok(value) => value.to_string(),
            Err(error) => json!({"action": "rejected", "reason": error.to_string()}).to_string(),
        }
    }

    #[tool(description = "Inspect entities linked to a name, and the memories that cite them.")]
    async fn memory_graph(&self, Parameters(args): Parameters<GraphArgs>) -> String {
        match self.service.graph(Actor::Agent, &args.entity).await {
            Ok(value) => value.to_string(),
            Err(error) => json!({"action": "rejected", "reason": error.to_string()}).to_string(),
        }
    }
}

#[tool_handler]
#[allow(clippy::unused_async_trait_impl, reason = "rmcp's tool_handler generates async trait methods")]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(INSTRUCTIONS)
        .with_server_info(Implementation::new("mcp-memory", env!("CARGO_PKG_VERSION")))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, ErrorData>> + Send {
        std::future::ready(Ok(ListResourcesResult::with_all_items(vec![{
            let mut resource = Resource::new("memory://digest", "digest");
            resource.description = Some(
                "Titles and summaries for global memories and the current repository".into(),
            );
            resource
        }])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ReadResourceResponse, ErrorData>> + Send
    {
        let service = Arc::clone(&self.service);
        let default_path = self.default_path.clone();
        let uri = request.uri;
        async move {
            if uri != "memory://digest" {
                return Err(ErrorData::invalid_params("unknown memory resource", None));
            }
            let text = service
                .digest(default_path.as_deref())
                .await
                .unwrap_or_else(|error| error.to_string());
            Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri)]).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RelationArg;
    use crate::model::RELATION_KINDS;

    #[test]
    fn relation_kind_schema_lists_every_allowed_kind() {
        let schema = serde_json::to_value(schemars::schema_for!(RelationArg)).unwrap();
        let description = schema["properties"]["kind"]["description"].as_str().unwrap();
        for kind in RELATION_KINDS {
            assert!(description.contains(kind), "{kind} missing from {description}");
        }
    }
}
