use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::access::Access;
use crate::model::{CONTENT_LIMIT, SUMMARY_LIMIT, TITLE_LIMIT};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub access: Access,
    pub require_trust: bool,
    pub trusted_repos: Vec<String>,
    pub repo_overrides: Vec<(String, String)>,
    pub recall_limit: usize,
    pub recall_bytes: usize,
    pub space_id: String,
    pub principal_id: String,
    pub data_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            access: Access::Learn,
            require_trust: false,
            trusted_repos: Vec::new(),
            repo_overrides: Vec::new(),
            recall_limit: 5,
            recall_bytes: 8 * 1024,
            space_id: "personal".into(),
            principal_id: "local".into(),
            data_dir: default_data_dir(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let file: FileConfig = toml::from_str(&text).map_err(|error| error.to_string())?;
        let mut config = Self::default();
        if let Some(access) = file.access {
            config.access = Access::parse(&access).ok_or_else(|| format!("unknown access {access}"))?;
        }
        if let Some(value) = file.require_trust {
            config.require_trust = value;
        }
        if let Some(repos) = file.trusted_repos {
            config.trusted_repos = repos;
        }
        if let Some(limit) = file.recall_limit {
            config.recall_limit = limit;
        }
        if let Some(bytes) = file.recall_bytes {
            config.recall_bytes = bytes;
        }
        if let Some(space) = file.space_id {
            config.space_id = space;
        }
        if let Some(principal) = file.principal_id {
            config.principal_id = principal;
        }
        if let Some(dir) = file.data_dir {
            config.data_dir = PathBuf::from(dir);
        }
        if let Some(overrides) = file.repo_overrides {
            config.repo_overrides = overrides
                .into_iter()
                .map(|item| (item.path, item.repo_key))
                .collect();
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let file = FileConfig {
            access: Some(self.access.as_str().into()),
            require_trust: Some(self.require_trust),
            trusted_repos: Some(self.trusted_repos.clone()),
            recall_limit: Some(self.recall_limit),
            recall_bytes: Some(self.recall_bytes),
            space_id: Some(self.space_id.clone()),
            principal_id: Some(self.principal_id.clone()),
            data_dir: Some(self.data_dir.display().to_string()),
            repo_overrides: Some(
                self.repo_overrides
                    .iter()
                    .map(|(path, repo_key)| RepoOverride {
                        path: path.clone(),
                        repo_key: repo_key.clone(),
                    })
                    .collect(),
            ),
        };
        let text = toml::to_string_pretty(&file).map_err(|error| error.to_string())?;
        fs::write(path, text).map_err(|error| error.to_string())
    }

    pub fn allows_repo(&self, repo_key: &str) -> bool {
        !self.require_trust || self.trusted_repos.iter().any(|item| item == repo_key)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FileConfig {
    access: Option<String>,
    require_trust: Option<bool>,
    trusted_repos: Option<Vec<String>>,
    recall_limit: Option<usize>,
    recall_bytes: Option<usize>,
    space_id: Option<String>,
    principal_id: Option<String>,
    data_dir: Option<String>,
    repo_overrides: Option<Vec<RepoOverride>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RepoOverride {
    path: String,
    repo_key: String,
}

pub fn config_path() -> PathBuf {
    default_data_dir().join("config.toml")
}

pub fn default_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MCP_MEMORY_HOME") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("mcp-memory")
}

pub fn check_bounds(title: &str, summary: &str, content: &str) -> Result<(), String> {
    if title.chars().count() > TITLE_LIMIT {
        return Err(format!("title is longer than {TITLE_LIMIT} characters"));
    }
    if summary.chars().count() > SUMMARY_LIMIT {
        return Err(format!("summary is longer than {SUMMARY_LIMIT} characters"));
    }
    if content.len() > CONTENT_LIMIT {
        return Err(format!(
            "content is longer than {CONTENT_LIMIT} bytes; split or shorten it"
        ));
    }
    if title.trim().is_empty() || content.trim().is_empty() {
        return Err("title and content are required".into());
    }
    Ok(())
}
