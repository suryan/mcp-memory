use serde::{Deserialize, Serialize};

pub const CONTENT_LIMIT: usize = 4 * 1024;
pub const TITLE_LIMIT: usize = 160;
pub const SUMMARY_LIMIT: usize = 240;
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Preference,
    Fact,
    Convention,
    Correction,
    RuledOut,
}

impl MemoryType {
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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Convention => "convention",
            Self::Correction => "correction",
            Self::RuledOut => "ruled_out",
        }
    }

    pub fn expects_entities(self) -> bool {
        matches!(
            self,
            Self::Fact | Self::Convention | Self::Correction | Self::RuledOut
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Repository,
}

impl Scope {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "global" => Some(Self::Global),
            "repository" | "repo" => Some(Self::Repository),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Repository => "repository",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Superseded,
    Conflicted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Conflicted => "conflicted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "conflicted" => Some(Self::Conflicted),
            _ => None,
        }
    }
}

pub const RELATION_KINDS: &[&str] = &[
    "REQUIRES",
    "PART_OF",
    "RUNS_WITH",
    "PREFERS",
    "CORRECTS",
    "RULED_OUT_FOR",
];

pub fn relation_kind(value: &str) -> Option<&'static str> {
    RELATION_KINDS
        .iter()
        .copied()
        .find(|kind| kind.eq_ignore_ascii_case(value.trim()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityInput {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "other".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationInput {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub scope: String,
    pub repo_key: String,
    pub space_id: String,
    pub principal_id: String,
    pub status: String,
    pub content_hash: String,
    pub created_at: String,
    pub updated_at: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub space_id: String,
    pub repo_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub kind: String,
    pub from_id: String,
    pub to_id: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDocument {
    pub version: u32,
    pub memories: Vec<MemoryRecord>,
    pub entities: Vec<EntityRecord>,
    pub edges: Vec<EdgeRecord>,
}
