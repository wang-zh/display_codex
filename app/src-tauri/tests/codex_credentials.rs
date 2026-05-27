use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use app_lib::codex_credentials::{
    codex_auth_path_from_home, load_codex_access_token_from_auth_file,
    user_facing_codex_credential_error, CodexCredentialError,
};

#[test]
fn loads_access_token_from_codex_auth_file() {
    let root = unique_temp_dir("codex-auth-token");
    let auth_path = root.join("auth.json");
    fs::write(
        &auth_path,
        r#"{
          "auth_mode": "chatgpt",
          "tokens": {
            "access_token": "access-token-value",
            "refresh_token": "refresh-token-value"
          }
        }"#,
    )
    .expect("expected auth file write");

    let token = load_codex_access_token_from_auth_file(&auth_path)
        .expect("expected access token from auth file");

    assert_eq!(token, "access-token-value");
}

#[test]
fn rejects_api_key_auth_without_chatgpt_tokens() {
    let root = unique_temp_dir("codex-auth-api-key");
    let auth_path = root.join("auth.json");
    fs::write(
        &auth_path,
        r#"{
          "auth_mode": "api_key",
          "OPENAI_API_KEY": "sk-redacted"
        }"#,
    )
    .expect("expected auth file write");

    let error = load_codex_access_token_from_auth_file(&auth_path)
        .expect_err("expected missing ChatGPT token");

    assert_eq!(error, CodexCredentialError::MissingAccessToken);
    let message = user_facing_codex_credential_error(&error);
    assert!(message.contains("codex login"));
    assert!(!message.contains("sk-redacted"));
}

#[test]
fn codex_auth_path_uses_codex_home_when_present() {
    let path = codex_auth_path_from_home(Some(Path::new(r"C:\Users\TR\.codex")));

    assert_eq!(path, PathBuf::from(r"C:\Users\TR\.codex\auth.json"));
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{name}-{}-{timestamp}", std::process::id()));
    fs::create_dir_all(&path).expect("expected temp dir creation");
    path
}
