use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexCredentialError {
    MissingCodexHome,
    Io(String),
    InvalidJson(String),
    MissingAccessToken,
}

impl std::fmt::Display for CodexCredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodexCredentialError::MissingCodexHome => {
                f.write_str("could not locate CODEX_HOME or user profile")
            }
            CodexCredentialError::Io(message) | CodexCredentialError::InvalidJson(message) => {
                f.write_str(message)
            }
            CodexCredentialError::MissingAccessToken => {
                f.write_str("Codex ChatGPT access token was not found")
            }
        }
    }
}

impl std::error::Error for CodexCredentialError {}

pub fn load_codex_access_token() -> Result<String, CodexCredentialError> {
    let auth_path = default_codex_auth_path()?;
    load_codex_access_token_from_auth_file(&auth_path)
}

pub fn load_codex_access_token_from_auth_file(
    auth_path: &Path,
) -> Result<String, CodexCredentialError> {
    let text = fs::read_to_string(auth_path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            CodexCredentialError::MissingAccessToken
        } else {
            CodexCredentialError::Io(format!("could not read Codex auth file: {error}"))
        }
    })?;
    let parsed: Value = serde_json::from_str(&text).map_err(|error| {
        CodexCredentialError::InvalidJson(format!("invalid Codex auth JSON: {error}"))
    })?;

    parsed
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(CodexCredentialError::MissingAccessToken)
}

pub fn codex_auth_path_from_home(codex_home: Option<&Path>) -> PathBuf {
    codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_fallback)
        .join("auth.json")
}

pub fn user_facing_codex_credential_error(error: &CodexCredentialError) -> String {
    match error {
        CodexCredentialError::MissingCodexHome | CodexCredentialError::MissingAccessToken => {
            "未找到本地 Codex ChatGPT 登录态。请先在终端运行 codex login，并选择 ChatGPT 登录。".to_owned()
        }
        CodexCredentialError::InvalidJson(_) => {
            "本地 Codex 登录文件格式无法识别。请运行 codex login 重新登录。".to_owned()
        }
        CodexCredentialError::Io(_) => {
            "无法读取本地 Codex 登录态。请确认当前用户可访问 ~/.codex/auth.json，或改用临时 Cookie Header。".to_owned()
        }
    }
}

fn default_codex_auth_path() -> Result<PathBuf, CodexCredentialError> {
    if let Some(path) = env::var_os("CODEX_HOME").map(PathBuf::from) {
        return Ok(codex_auth_path_from_home(Some(&path)));
    }

    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or(CodexCredentialError::MissingCodexHome)?;
    Ok(home.join(".codex").join("auth.json"))
}

fn default_codex_home_fallback() -> PathBuf {
    env::var_os("CODEX_HOME")
        .or_else(|| {
            env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join(".codex").into_os_string())
        })
        .or_else(|| {
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex").into_os_string())
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".codex"))
}
