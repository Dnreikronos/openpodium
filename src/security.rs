//! Shared security policy helpers for diagnostic and process boundaries.

use std::borrow::Cow;

const SENSITIVE_NAMES: &[&str] = &[
    "accesstoken",
    "apikey",
    "authorization",
    "clientsecret",
    "cookie",
    "credential",
    "password",
    "passwd",
    "privatekey",
    "refreshtoken",
    "secret",
    "sessiontoken",
    "token",
];

const CREDENTIAL_PREFIXES: &[(&str, usize)] = &[
    ("github_pat_", 20),
    ("ghp_", 20),
    ("xoxb-", 16),
    ("akia", 16),
    ("sk-", 16),
];

/// Returns whether an environment or settings name conventionally carries a secret.
pub fn is_sensitive_name(name: &str) -> bool {
    let normalized = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    SENSITIVE_NAMES
        .iter()
        .any(|candidate| normalized.contains(candidate))
}

/// Returns whether a URL authority contains user information before the host.
pub fn url_has_userinfo(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    remainder
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

/// Redacts common credential shapes before untrusted text reaches diagnostics.
pub fn redact_secrets(value: &str) -> Cow<'_, str> {
    let lower = value.to_ascii_lowercase();
    if lower.contains("-----begin ") && lower.contains("private key-----") {
        return Cow::Borrowed("[redacted private key]");
    }

    let mut redacted = value.to_owned();
    redact_url_passwords(&mut redacted);
    redact_authorization(&mut redacted, "bearer ");
    redact_authorization(&mut redacted, "basic ");
    for marker in SENSITIVE_NAMES {
        redact_assignment(&mut redacted, marker);
    }
    for (prefix, minimum) in CREDENTIAL_PREFIXES {
        redact_prefixed_credential(&mut redacted, prefix, *minimum);
    }

    if redacted == value {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(redacted)
    }
}

fn redact_assignment(value: &mut String, marker: &str) {
    let mut offset = 0;
    loop {
        let lower = value.to_ascii_lowercase();
        let Some(relative) = lower[offset..].find(marker) else {
            break;
        };
        let start = offset + relative;
        let end = start + marker.len();
        if !token_boundary(lower.as_bytes(), start, end) {
            offset = end;
            continue;
        }

        let bytes = value.as_bytes();
        let mut cursor = end;
        if bytes.get(cursor) == Some(&b'"') || bytes.get(cursor) == Some(&b'\'') {
            cursor += 1;
        }
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if !matches!(bytes.get(cursor), Some(b'=') | Some(b':')) {
            offset = end;
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let quote = bytes
            .get(cursor)
            .copied()
            .filter(|byte| matches!(byte, b'"' | b'\''));
        if quote.is_some() {
            cursor += 1;
        }
        let value_start = cursor;
        while let Some(byte) = bytes.get(cursor) {
            if quote.map_or_else(
                || byte.is_ascii_whitespace() || matches!(byte, b',' | b';' | b'}' | b']'),
                |quote| *byte == quote,
            ) {
                break;
            }
            cursor += 1;
        }
        if cursor > value_start {
            value.replace_range(value_start..cursor, "[redacted]");
            offset = value_start + "[redacted]".len();
        } else {
            offset = end;
        }
    }
}

fn redact_authorization(value: &mut String, marker: &str) {
    let mut offset = 0;
    loop {
        let lower = value.to_ascii_lowercase();
        let Some(relative) = lower[offset..].find(marker) else {
            break;
        };
        let start = offset + relative;
        if start > 0 && lower.as_bytes()[start - 1].is_ascii_alphanumeric() {
            offset = start + marker.len();
            continue;
        }
        let credential_start = start + marker.len();
        let credential_end = value.as_bytes()[credential_start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b',' | b';'))
            .map_or(value.len(), |relative| credential_start + relative);
        if credential_end > credential_start {
            value.replace_range(credential_start..credential_end, "[redacted]");
            offset = credential_start + "[redacted]".len();
        } else {
            offset = credential_start;
        }
    }
}

fn redact_prefixed_credential(value: &mut String, prefix: &str, minimum: usize) {
    let mut offset = 0;
    loop {
        let lower = value.to_ascii_lowercase();
        let Some(relative) = lower[offset..].find(prefix) else {
            break;
        };
        let start = offset + relative;
        if start > 0
            && (lower.as_bytes()[start - 1].is_ascii_alphanumeric()
                || lower.as_bytes()[start - 1] == b'_')
        {
            offset = start + prefix.len();
            continue;
        }
        let end = lower.as_bytes()[start + prefix.len()..]
            .iter()
            .position(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
            .map_or(value.len(), |relative| start + prefix.len() + relative);
        if end - start >= minimum {
            value.replace_range(start..end, "[redacted]");
            offset = start + "[redacted]".len();
        } else {
            offset = start + prefix.len();
        }
    }
}

fn redact_url_passwords(value: &mut String) {
    let mut offset = 0;
    while let Some(relative) = value[offset..].find("://") {
        let authority_start = offset + relative + 3;
        let authority_end = value.as_bytes()[authority_start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'?' | b'#'))
            .map_or(value.len(), |relative| authority_start + relative);
        let authority = &value[authority_start..authority_end];
        let Some(at) = authority.rfind('@') else {
            offset = authority_end;
            continue;
        };
        let Some(colon) = authority[..at].find(':') else {
            offset = authority_end;
            continue;
        };
        let password_start = authority_start + colon + 1;
        let password_end = authority_start + at;
        value.replace_range(password_start..password_end, "[redacted]");
        offset = password_start + "[redacted]".len() + 1;
    }
}

fn token_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
    let starts_cleanly =
        start == 0 || (!bytes[start - 1].is_ascii_alphanumeric() && bytes[start - 1] != b'_');
    let ends_cleanly =
        end == bytes.len() || (!bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_');
    starts_cleanly && ends_cleanly
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_names_cover_common_environment_conventions() {
        for name in [
            "OPENPODIUM_IPC_TOKEN",
            "API_KEY",
            "client-secret",
            "Authorization",
            "DATABASE_PASSWORD",
        ] {
            assert!(is_sensitive_name(name), "{name} should be sensitive");
        }
        for name in ["PATH", "TERM", "MONKEY_PATCH", "PUBLIC_ID"] {
            assert!(!is_sensitive_name(name), "{name} should not be sensitive");
        }
    }

    #[test]
    fn diagnostics_redact_assignments_headers_tokens_urls_and_private_keys() {
        let diagnostic = concat!(
            "token=top-secret-value Authorization: Bearer abcdefghijklmnop ",
            "https://alice:hunter2@example.test/ ghp_abcdefghijklmnopqrstuvwxyz"
        );
        let redacted = redact_secrets(diagnostic);
        assert!(!redacted.contains("top-secret-value"));
        assert!(!redacted.contains("abcdefghijklmnop"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
        assert!(redacted.contains("alice:[redacted]@example.test"));
        assert_eq!(
            redact_secrets("-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----"),
            "[redacted private key]"
        );
    }

    #[test]
    fn ordinary_diagnostics_are_left_unchanged() {
        let message = "connection refused while opening project tokenization.rs";
        assert!(matches!(redact_secrets(message), Cow::Borrowed(_)));
        assert_eq!(redact_secrets(message), message);
    }

    #[test]
    fn url_user_information_is_detected_without_confusing_paths() {
        assert!(url_has_userinfo("https://alice:secret@example.test/path"));
        assert!(url_has_userinfo("https://alice@example.test"));
        assert!(!url_has_userinfo(
            "https://example.test/users/alice@example.test"
        ));
    }
}
