use std::path::{Path, PathBuf};
use std::process::Command;

/// Stable repository identity. A git remote survives a path move. A path does not.
pub fn repo_key(path: &Path, overrides: &[(String, String)]) -> String {
    let canonical = canonicalize(path);
    let display = canonical.to_string_lossy();
    for (prefix, key) in overrides {
        if display == *prefix || display.starts_with(prefix) {
            return key.clone();
        }
    }
    if let Some(remote) = git_remote(&canonical) {
        return normalize_remote(&remote);
    }
    display.to_string()
}

pub fn normalize_remote(url: &str) -> String {
    let mut value = url.trim().trim_end_matches('/').to_string();
    if let Some(rest) = value.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            value = format!("{host}/{path}");
        }
    }
    for prefix in ["https://", "http://", "ssh://"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest.to_string();
            break;
        }
    }
    if let Some(rest) = value.strip_prefix("ssh://") {
        value = rest.to_string();
    }
    value = value.trim_start_matches("git@").replace(':', "/");
    if let Some(stripped) = value.strip_suffix(".git") {
        value = stripped.to_string();
    }
    if let Some((host, path)) = value.split_once('/') {
        format!("{}/{}", host.to_ascii_lowercase(), path.trim_start_matches('/'))
    } else {
        value.to_ascii_lowercase()
    }
}

fn canonicalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn git_remote(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_scp_and_https_remotes_to_the_same_key() {
        assert_eq!(
            normalize_remote("git@github.com:Acme/Memory.git"),
            "github.com/Acme/Memory"
        );
        assert_eq!(
            normalize_remote("https://github.com/Acme/Memory.git"),
            "github.com/Acme/Memory"
        );
    }
}
