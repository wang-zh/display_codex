use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_LOG_BYTES: u64 = 256 * 1024;

pub fn append_log_line(path: &Path, event: &str, message: impl AsRef<str>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_log_if_needed(path)?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let event = sanitize_log_token(event);
    let message = sanitize_log_message(message.as_ref());
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "ts={timestamp} event={event} message={message}")
}

pub fn sanitize_log_message(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let lower = sanitized.to_ascii_lowercase();
    for marker in [
        "cookie:",
        "authorization:",
        "bearer ",
        "__secure-next-auth.session-token=",
        "cf_clearance=",
        "oai-sc=",
    ] {
        if let Some(index) = lower.find(marker) {
            sanitized.truncate(index);
            sanitized.push_str("[redacted sensitive HTTP credential]");
            break;
        }
    }

    const MAX_MESSAGE_CHARS: usize = 2_000;
    if sanitized.chars().count() > MAX_MESSAGE_CHARS {
        sanitized = sanitized.chars().take(MAX_MESSAGE_CHARS).collect();
        sanitized.push_str("...");
    }

    sanitized
}

fn rotate_log_if_needed(path: &Path) -> io::Result<()> {
    if !path.exists() || path.metadata()?.len() <= MAX_LOG_BYTES {
        return Ok(());
    }

    let old_path = path.with_extension("log.old");
    let _ = fs::remove_file(&old_path);
    fs::rename(path, old_path)
}

fn sanitize_log_token(value: &str) -> String {
    let token: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .take(64)
        .collect();
    if token.is_empty() {
        "event".to_owned()
    } else {
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn append_log_line_creates_parent_and_records_event() {
        let root = unique_temp_dir("codex-quota-log-create");
        let log_path = root.join("nested").join("codex-quota.log");

        append_log_line(&log_path, "refresh_start", "source=edge").unwrap();

        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("event=refresh_start"));
        assert!(content.contains("source=edge"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sanitize_log_message_keeps_entries_single_line() {
        assert_eq!(
            sanitize_log_message("first line\r\nsecond line\tthird"),
            "first line second line third"
        );
    }

    #[test]
    fn sanitize_log_message_redacts_http_credentials() {
        let sanitized = sanitize_log_message(
            "request failed Cookie: a=b; __Secure-next-auth.session-token=secret Bearer token",
        );

        assert!(sanitized.contains("[redacted sensitive HTTP credential]"));
        assert!(!sanitized.contains("secret"));
        assert!(!sanitized.contains("Bearer token"));
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
