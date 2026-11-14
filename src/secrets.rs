/// Reject values that look like credentials. The stored text is not logged.
pub fn looks_like_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        return true;
    }
    if lower.contains("aws_secret_access_key") || lower.contains("aws_session_token") {
        return true;
    }
    for token in text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',') {
        if token.starts_with("AKIA") && token.len() >= 20 {
            return true;
        }
        if token.starts_with("ghp_")
            || token.starts_with("gho_")
            || token.starts_with("github_pat_")
            || token.starts_with("sk-")
            || token.starts_with("xoxb-")
            || token.starts_with("xoxp-")
        {
            return true;
        }
        if token.len() > 20 && token.starts_with("eyJ") && token.matches('.').count() == 2 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_key_material_and_keeps_ordinary_text() {
        assert!(looks_like_secret(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
        ));
        assert!(looks_like_secret("token sk-live-abcdefghijklmnopqrstuvwxyz"));
        assert!(looks_like_secret("AKIAIOSFODNN7EXAMPLE"));
        assert!(!looks_like_secret(
            "seed the local database before the integration tests"
        ));
    }
}
