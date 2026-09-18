//! **Layer: Domain/Core** (shared utilities)
//!
//! Best-effort redaction of secrets (API keys, bearer tokens, URLs with
//! embedded credentials) from human-readable strings.
//!
//! This is defense-in-depth for display/log paths, not a security boundary:
//! the real guarantee is never placing secrets into these strings. Callers
//! in higher layers ([`crate::auto_sync::status`], [`crate::logging`],
//! [`crate::error`]) share this single definition so redaction cannot drift.

use std::sync::LazyLock;

static SECRET_ASSIGNMENT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)((?:api[_-]?key|token|password|passwd|secret|credential|authorization)\s*=\s*)(?:"[^"]*"|'[^']*'|[^\s&;]+)"#,
    )
    .expect("secret assignment pattern is valid")
});
static BEARER_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)\b(Bearer)\s+[^\s"']+"#).expect("bearer pattern is valid")
});
static URL_CREDENTIALS_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)(https?://)[^/\s:@]+(?::[^/\s@]*)?@"#)
        .expect("URL credential pattern is valid")
});

/// Redact potential secrets from a message string.
///
/// Strips API keys, bearer tokens, and URLs with embedded credentials.
/// Uses simple pattern matching — this is best-effort redaction, not a
/// security boundary.
pub(crate) fn redact_secrets(msg: &str) -> String {
    let result = SECRET_ASSIGNMENT_RE.replace_all(msg, "$1[REDACTED]");
    let result = BEARER_RE.replace_all(&result, "$1 [REDACTED]");
    URL_CREDENTIALS_RE
        .replace_all(&result, "$1[REDACTED]@")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_assignment() {
        assert_eq!(
            redact_secrets(r#"api_key = "hunter2""#),
            "api_key = [REDACTED]"
        );
    }

    #[test]
    fn redacts_bearer_token() {
        assert_eq!(
            redact_secrets("Authorization: Bearer hunter2"),
            "Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn redacts_url_credentials() {
        assert_eq!(
            redact_secrets("https://user:s3cret@example.com/x"),
            "https://[REDACTED]@example.com/x"
        );
    }

    #[test]
    fn leaves_plain_text_untouched() {
        assert_eq!(redact_secrets("git status"), "git status");
    }
}
