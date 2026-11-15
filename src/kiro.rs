//! Read Kiro's local SQLite memory database into an [`ExportDocument`].

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::config::Config;
use crate::model::{content_hash, ExportDocument, MemoryRecord, MemoryType, Scope, SCHEMA_VERSION};

/// Default Kiro memory database: `$KIRO_HOME/memories/memories.db`, else `~/.kiro/memories/memories.db`.
pub fn default_db_path() -> PathBuf {
    if let Ok(home) = std::env::var("KIRO_HOME") {
        return PathBuf::from(home).join("memories").join("memories.db");
    }
    dirs_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kiro")
        .join("memories")
        .join("memories.db")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Open Kiro's memory database read-only and map every row into an export document.
///
/// # Errors
///
/// Fails when the file is missing, is not a readable SQLite database, or the
/// `memories` table is absent.
pub fn load_export(path: &Path, config: &Config) -> Result<ExportDocument, String> {
    if !path.is_file() {
        return Err(format!(
            "Kiro memory database not found at {}; pass --path if it lives elsewhere",
            path.display()
        ));
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut statement = connection
        .prepare(
            "SELECT id, memory_type, scopes, title, summary, content,
                    why, how_to_apply, created_at, updated_at
             FROM memories
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|error| {
            format!(
                "read memories from {}: {error}; is this a Kiro memories.db?",
                path.display()
            )
        })?;
    let memories = statement
        .query_map([], |row| {
            Ok(KiroRow {
                id: row.get(0)?,
                memory_type: row.get(1)?,
                scopes: row.get(2)?,
                title: row.get(3)?,
                summary: row.get(4)?,
                content: row.get(5)?,
                why: row.get(6)?,
                how_to_apply: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })
        .map_err(|error| format!("query memories: {error}"))?
        .map(|row| {
            row.map_err(|error| format!("read memory row: {error}"))
                .map(|row| row.into_record(config))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExportDocument {
        version: SCHEMA_VERSION,
        memories,
        entities: Vec::new(),
        edges: Vec::new(),
    })
}

struct KiroRow {
    id: String,
    memory_type: String,
    scopes: String,
    title: String,
    summary: String,
    content: String,
    why: Option<String>,
    how_to_apply: Option<String>,
    created_at: String,
    updated_at: String,
}

impl KiroRow {
    fn into_record(self, config: &Config) -> MemoryRecord {
        let content = append_sections(self.content, self.why.as_deref(), self.how_to_apply.as_deref());
        let (scope, repo_key) = map_scope(&self.scopes);
        let memory_type = map_type(&self.memory_type);
        let hash = content_hash(&self.title, &self.summary, &content);
        MemoryRecord {
            id: self.id,
            title: self.title,
            summary: self.summary,
            content,
            memory_type: memory_type.as_str().into(),
            scope: scope.as_str().into(),
            repo_key,
            space_id: config.space_id.clone(),
            principal_id: config.principal_id.clone(),
            status: "active".into(),
            content_hash: hash,
            created_at: self.created_at,
            updated_at: self.updated_at,
            source: "import".into(),
        }
    }
}

fn map_type(value: &str) -> MemoryType {
    match value.trim().to_ascii_uppercase().as_str() {
        "USER_PREFERENCE" => MemoryType::Preference,
        _ => MemoryType::Fact,
    }
}

fn map_scope(scopes_json: &str) -> (Scope, String) {
    let scopes: Vec<String> = serde_json::from_str(scopes_json).unwrap_or_default();
    for scope in scopes {
        let trimmed = scope.trim();
        if trimmed.eq_ignore_ascii_case("Global") {
            return (Scope::Global, String::new());
        }
        if let Some(rest) = trimmed.strip_prefix("repo:") {
            return (Scope::Repository, percent_decode(rest));
        }
    }
    (Scope::Global, String::new())
}

fn append_sections(mut content: String, why: Option<&str>, how: Option<&str>) -> String {
    if let Some(why) = why.map(str::trim).filter(|value| !value.is_empty()) {
        content.push_str("\n\nWhy: ");
        content.push_str(why);
    }
    if let Some(how) = how.map(str::trim).filter(|value| !value.is_empty()) {
        content.push_str("\n\nHow to apply: ");
        content.push_str(how);
    }
    content
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (
                from_hex(bytes[index + 1]),
                from_hex(bytes[index + 2]),
            ) {
                out.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{append_sections, map_scope, map_type, percent_decode};
    use crate::model::{MemoryType, Scope};

    #[test]
    fn preference_and_semantic_map() {
        assert_eq!(map_type("USER_PREFERENCE"), MemoryType::Preference);
        assert_eq!(map_type("SEMANTIC"), MemoryType::Fact);
        assert_eq!(map_type("other"), MemoryType::Fact);
    }

    #[test]
    fn scopes_map_global_and_repo() {
        assert_eq!(
            map_scope(r#"["Global"]"#),
            (Scope::Global, String::new())
        );
        assert_eq!(
            map_scope(r#"["repo:github/suryan/mcp-guard"]"#),
            (Scope::Repository, "github/suryan/mcp-guard".into())
        );
        assert_eq!(
            map_scope(r#"["repo:azure/blue-current/AU%20Metering/DeEnReEnService"]"#),
            (
                Scope::Repository,
                "azure/blue-current/AU Metering/DeEnReEnService".into()
            )
        );
    }

    #[test]
    fn percent_decode_handles_spaces() {
        assert_eq!(percent_decode("AU%20Metering"), "AU Metering");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn why_and_how_append() {
        let content = append_sections(
            "Body.".into(),
            Some(" because "),
            Some(" use it when X "),
        );
        assert!(content.contains("Body."));
        assert!(content.contains("Why: because"));
        assert!(content.contains("How to apply: use it when X"));
    }

    #[test]
    fn load_export_maps_fixture_rows() {
        use rusqlite::Connection;

        use crate::config::Config;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE memories (
                   id TEXT PRIMARY KEY,
                   memory_type TEXT NOT NULL,
                   scopes TEXT NOT NULL,
                   title TEXT NOT NULL,
                   summary TEXT NOT NULL,
                   content TEXT NOT NULL,
                   why TEXT,
                   how_to_apply TEXT,
                   session_id TEXT NOT NULL,
                   metadata TEXT,
                   created_at TEXT NOT NULL,
                   updated_at TEXT NOT NULL
                 );
                INSERT INTO memories
                 (id, memory_type, scopes, title, summary, content, why, how_to_apply,
                  session_id, created_at, updated_at)
                 VALUES
                 ('pref-1', 'USER_PREFERENCE', '["Global"]', 'Prefer tabs', 'Use tabs',
                  'Use tabs in Makefiles.', 'Keeps Makefiles consistent.', NULL,
                  'sess', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                 ('fact-1', 'SEMANTIC', '["repo:github/suryan/demo"]', 'Seed db',
                  'Seed before tests', 'Run seed before integration tests.', NULL, NULL,
                  'sess', '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z');
                "#,
            )
            .unwrap();

        let config = Config {
            space_id: "personal".into(),
            principal_id: "local".into(),
            ..Config::default()
        };
        let document = super::load_export(&path, &config).unwrap();
        assert_eq!(document.memories.len(), 2);
        let global = &document.memories[0];
        assert_eq!(global.id, "pref-1");
        assert_eq!(global.memory_type, "preference");
        assert_eq!(global.scope, "global");
        assert_eq!(global.repo_key, "");
        assert!(global.content.contains("Why: Keeps Makefiles consistent."));
        assert_eq!(global.space_id, "personal");
        assert_eq!(global.source, "import");
        let repo = &document.memories[1];
        assert_eq!(repo.memory_type, "fact");
        assert_eq!(repo.scope, "repository");
        assert_eq!(repo.repo_key, "github/suryan/demo");
    }
}
