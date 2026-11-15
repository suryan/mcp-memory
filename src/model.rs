//! Memory types, scopes, statuses, limits, and the records exchanged with the graph.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Largest content in bytes. Longer text belongs in project docs, not memory.
pub const CONTENT_LIMIT: usize = 20 * 1024;
/// Largest title in characters.
pub const TITLE_LIMIT: usize = 160;
/// Largest summary in characters.
pub const SUMMARY_LIMIT: usize = 240;
/// Graph schema version reported by health.
pub const SCHEMA_VERSION: u32 = 1;

/// What kind of statement a memory is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// How the user wants work done.
    Preference,
    /// Something true about a repository or tool.
    Fact,
    /// A project or team convention.
    Convention,
    /// The user corrected the agent.
    Correction,
    /// An approach that was tried and does not work.
    RuledOut,
}

impl MemoryType {
    /// Parse a type name. Accepts `ruled-out` for `ruled_out`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "preference" => Some(Self::Preference),
            "fact" => Some(Self::Fact),
            "convention" => Some(Self::Convention),
            "correction" => Some(Self::Correction),
            "ruled_out" | "ruled-out" => Some(Self::RuledOut),
            _ => None,
        }
    }

    /// The name stored on the node.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Convention => "convention",
            Self::Correction => "correction",
            Self::RuledOut => "ruled_out",
        }
    }

    /// Whether a write of this type is expected to name entities. Only logged when missing.
    pub fn expects_entities(self) -> bool {
        matches!(
            self,
            Self::Fact | Self::Convention | Self::Correction | Self::RuledOut
        )
    }
}

/// Where a memory applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Every repository.
    Global,
    /// One repository, by repository key.
    Repository,
}

impl Scope {
    /// Parse a scope name. Accepts `repo` for `repository`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "global" => Some(Self::Global),
            "repository" | "repo" => Some(Self::Repository),
            _ => None,
        }
    }

    /// The name stored on the node.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Repository => "repository",
        }
    }
}

/// Lifecycle of a memory. Recall reads only active memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Current and recallable.
    Active,
    /// Replaced by a newer memory.
    Superseded,
    /// An import disagreed with the stored content for the same id.
    Conflicted,
}

impl Status {
    /// The name stored on the node.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Conflicted => "conflicted",
        }
    }

    /// Parse a stored status name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "conflicted" => Some(Self::Conflicted),
            _ => None,
        }
    }
}

/// Typed entity links the agent may write.
pub const RELATION_KINDS: &[&str] = &[
    "REQUIRES",
    "PART_OF",
    "RUNS_WITH",
    "PREFERS",
    "CORRECTS",
    "RULED_OUT_FOR",
];

/// The canonical relation kind for a case-insensitive name, if it is allowed.
pub fn relation_kind(value: &str) -> Option<&'static str> {
    RELATION_KINDS
        .iter()
        .copied()
        .find(|kind| kind.eq_ignore_ascii_case(value.trim()))
}

/// An entity named in a write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityInput {
    /// Display name. Matched case-insensitively within the space.
    pub name: String,
    /// Free-form kind such as `command` or `service`. Defaults to `other`.
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "other".into()
}

/// A typed link between two entities in a write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationInput {
    /// Source entity name.
    pub from: String,
    /// Target entity name.
    pub to: String,
    /// One of [`RELATION_KINDS`].
    pub kind: String,
}

/// A stored memory as the CLI, list, and export see it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    /// Memory id (a UUID, separate from Helix's internal id).
    pub id: String,
    /// One-line title.
    pub title: String,
    /// One-line summary used by the digest.
    pub summary: String,
    /// The statement body.
    pub content: String,
    /// See [`MemoryType`].
    #[serde(rename = "type")]
    pub memory_type: String,
    /// See [`Scope`].
    pub scope: String,
    /// Repository key, empty for global memories.
    pub repo_key: String,
    /// Owning space.
    pub space_id: String,
    /// Principal that wrote it.
    pub principal_id: String,
    /// See [`Status`].
    pub status: String,
    /// SHA-256 of title, summary, and content. Detects divergent imports.
    pub content_hash: String,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// RFC 3339 time of the last change.
    pub updated_at: String,
    /// `user` for explicit writes, `automatic` otherwise.
    pub source: String,
}

/// A stored entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    /// Entity id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Free-form kind.
    pub kind: String,
    /// Owning space.
    pub space_id: String,
    /// Repository key, empty when global.
    pub repo_key: String,
}

/// A stored entity link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRecord {
    /// One of [`RELATION_KINDS`].
    pub kind: String,
    /// Source entity id.
    pub from_id: String,
    /// Target entity id.
    pub to_id: String,
    /// Memory id that asserted the link.
    pub evidence_id: String,
}

/// SHA-256 of title, summary, and content, with null separators. Detects divergent imports.
pub fn content_hash(title: &str, summary: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(summary.as_bytes());
    hasher.update([0]);
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The JSON written by `export` and read by `import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDocument {
    /// Format version.
    pub version: u32,
    /// Memories in the space, without embeddings.
    pub memories: Vec<MemoryRecord>,
    /// Entities. Ignored on import in this build.
    pub entities: Vec<EntityRecord>,
    /// Entity links. Empty in this build.
    pub edges: Vec<EdgeRecord>,
}
