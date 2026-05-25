use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::Connection;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCookie {
    pub host: String,
    pub name: String,
    pub value: String,
}

impl BrowserCookie {
    pub fn new(host: impl Into<String>, name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            name: name.into(),
            value: value.into(),
        }
    }
}

pub fn build_cookie_header(cookies: &[BrowserCookie]) -> Option<String> {
    let parts: Vec<String> = cookies
        .iter()
        .filter(|cookie| is_chatgpt_host(&cookie.host))
        .filter(|cookie| !cookie.name.trim().is_empty() && !cookie.value.is_empty())
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect();

    (!parts.is_empty()).then(|| parts.join("; "))
}

pub fn cookie_header_from_override(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_cookie_header_override)
        .filter(|value| !value.is_empty())
}

fn normalize_cookie_header_override(value: &str) -> String {
    let first_line = value.lines().next().unwrap_or(value).trim();
    let lower = first_line.to_ascii_lowercase();
    let without_prefix = if lower.starts_with("cookie:") {
        first_line["cookie:".len()..].trim()
    } else {
        first_line
    };

    without_prefix
        .trim_matches(|ch| ch == '"' || ch == '\'')
        .trim()
        .to_owned()
}

pub fn load_edge_cookie_header() -> Result<String, CredentialError> {
    if let Some(header) =
        cookie_header_from_override(std::env::var("CODEX_QUOTA_COOKIE_HEADER").ok().as_deref())
    {
        return Ok(header);
    }

    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| CredentialError::Io("LOCALAPPDATA is not set".to_owned()))?;
    let user_data_dir = edge_user_data_dir_from_local_app_data(&local_app_data);
    let local_state_path = user_data_dir.join("Local State");
    let local_state = fs::read_to_string(&local_state_path).map_err(|error| {
        CredentialError::Io(format!("could not read Edge Local State: {error}"))
    })?;
    let protected_key = extract_dpapi_wrapped_key(&local_state)?;
    let aes_key = decrypt_dpapi_blob(&protected_key)?;
    let db_paths = discover_cookie_db_paths(&user_data_dir).map_err(|error| {
        CredentialError::Io(format!("could not inspect Edge profiles: {error}"))
    })?;

    let mut cookies = Vec::new();
    let mut last_error = None;
    for db_path in db_paths {
        match read_cookies_from_profile_db(&db_path, &aes_key) {
            Ok(mut profile_cookies) => cookies.append(&mut profile_cookies),
            Err(error) => last_error = Some(error),
        }
    }

    build_cookie_header(&cookies).ok_or_else(|| {
        last_error.unwrap_or_else(|| {
            CredentialError::MissingCookies(
                "No usable chatgpt.com cookies were found in Edge. Sign in to ChatGPT in Edge."
                    .to_owned(),
            )
        })
    })
}

pub fn discover_cookie_db_paths(user_data_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    for entry in fs::read_dir(user_data_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }

        let cookie_db = entry.path().join("Network").join("Cookies");
        if cookie_db.is_file() {
            paths.push(cookie_db);
        }
    }

    paths.sort();
    Ok(paths)
}

pub fn edge_user_data_dir_from_local_app_data(local_app_data: &Path) -> PathBuf {
    local_app_data
        .join("Microsoft")
        .join("Edge")
        .join("User Data")
}

pub fn extract_dpapi_wrapped_key(local_state_json: &str) -> Result<Vec<u8>, CredentialError> {
    let parsed: Value = serde_json::from_str(local_state_json)
        .map_err(|error| CredentialError::LocalState(format!("invalid JSON: {error}")))?;
    let encoded = parsed
        .pointer("/os_crypt/encrypted_key")
        .and_then(Value::as_str)
        .ok_or_else(|| CredentialError::LocalState("missing os_crypt.encrypted_key".to_owned()))?;
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|error| CredentialError::LocalState(format!("invalid encrypted_key: {error}")))?;
    let Some(unwrapped) = decoded.strip_prefix(b"DPAPI") else {
        return Err(CredentialError::LocalState(
            "encrypted_key is missing DPAPI prefix".to_owned(),
        ));
    };
    Ok(unwrapped.to_vec())
}

pub fn read_cookies_from_db(
    db_path: &Path,
    aes_key: Option<&[u8]>,
) -> Result<Vec<BrowserCookie>, CredentialError> {
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| CredentialError::Io(error.to_string()))?;
    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, value, encrypted_value
             FROM cookies
             WHERE host_key = 'chatgpt.com' OR host_key = '.chatgpt.com'",
        )
        .map_err(|error| CredentialError::Io(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let host: String = row.get(0)?;
            let name: String = row.get(1)?;
            let value: String = row.get(2)?;
            let encrypted_value: Vec<u8> = row.get(3)?;
            Ok((host, name, value, encrypted_value))
        })
        .map_err(|error| CredentialError::Io(error.to_string()))?;

    let mut cookies = Vec::new();
    for row in rows {
        let (host, name, value, encrypted_value) =
            row.map_err(|error| CredentialError::Io(error.to_string()))?;
        let value = if !value.is_empty() {
            value
        } else if !encrypted_value.is_empty() {
            let Some(key) = aes_key else {
                continue;
            };
            decrypt_chromium_cookie_value(&encrypted_value, key)?
        } else {
            String::new()
        };

        if !value.is_empty() {
            cookies.push(BrowserCookie::new(host, name, value));
        }
    }

    Ok(cookies)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    LocalState(String),
    UnsupportedEncryption(String),
    Io(String),
    Decryption(String),
    MissingCookies(String),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialError::LocalState(message)
            | CredentialError::UnsupportedEncryption(message)
            | CredentialError::Io(message)
            | CredentialError::Decryption(message)
            | CredentialError::MissingCookies(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CredentialError {}

pub fn user_facing_credential_error(error: &CredentialError) -> String {
    match error {
        CredentialError::Io(message)
            if message.contains("could not copy or read Edge cookie DB")
                || message.contains("另一个程序正在使用此文件")
                || message.contains("unable to open database file") =>
        {
            "Edge 正在锁定 Cookie 数据库，当前无法自动读取登录态。请关闭 Edge 后重试，或在设置里临时粘贴 ChatGPT 请求的 Cookie Header。".to_owned()
        }
        CredentialError::UnsupportedEncryption(message) if message.contains("v20") => {
            "Edge 使用了 Chromium v20 app-bound cookie 加密，当前版本无法直接解密。请改用设置里的临时 Cookie Header，或后续接浏览器扩展桥接。".to_owned()
        }
        CredentialError::MissingCookies(_) => {
            "没有在 Edge 中找到可用的 chatgpt.com 登录态。请确认 Edge 已登录 ChatGPT，或在设置里临时粘贴 Cookie Header。".to_owned()
        }
        CredentialError::Io(message)
            if message.contains("could not read Edge Local State")
                || message.contains("LOCALAPPDATA") =>
        {
            "无法读取 Edge 本地配置。请确认本机安装并登录 Microsoft Edge，或在设置里临时粘贴 Cookie Header。".to_owned()
        }
        _ => error.to_string(),
    }
}

#[cfg(windows)]
pub fn decrypt_dpapi_blob(protected: &[u8]) -> Result<Vec<u8>, CredentialError> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok =
        unsafe { CryptUnprotectData(&input, null_mut(), null(), null(), null(), 0, &mut output) };
    if ok == 0 {
        return Err(CredentialError::Decryption(
            "Windows DPAPI could not decrypt the Edge state key".to_owned(),
        ));
    }

    let decrypted =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut _);
    }
    Ok(decrypted)
}

#[cfg(not(windows))]
pub fn decrypt_dpapi_blob(_protected: &[u8]) -> Result<Vec<u8>, CredentialError> {
    Err(CredentialError::UnsupportedEncryption(
        "DPAPI is only available on Windows".to_owned(),
    ))
}

pub fn decrypt_chromium_cookie_value(
    encrypted_value: &[u8],
    key: &[u8],
) -> Result<String, CredentialError> {
    if encrypted_value.starts_with(b"v20") {
        return Err(CredentialError::UnsupportedEncryption(
            "v20 app-bound encrypted cookies are not supported".to_owned(),
        ));
    }

    if encrypted_value.starts_with(b"v10") || encrypted_value.starts_with(b"v11") {
        if key.len() != 32 {
            return Err(CredentialError::Decryption(
                "Chromium AES-GCM key must be 32 bytes".to_owned(),
            ));
        }
        if encrypted_value.len() < 3 + 12 + 16 {
            return Err(CredentialError::Decryption(
                "encrypted cookie is too short".to_owned(),
            ));
        }

        let nonce = Nonce::from_slice(&encrypted_value[3..15]);
        let ciphertext = &encrypted_value[15..];
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|error| CredentialError::Decryption(error.to_string()))?;
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|error| CredentialError::Decryption(error.to_string()))?;
        return String::from_utf8(plaintext)
            .map_err(|error| CredentialError::Decryption(error.to_string()));
    }

    Err(CredentialError::UnsupportedEncryption(
        "legacy DPAPI cookie values are not implemented".to_owned(),
    ))
}

fn copy_cookie_db_to_temp(db_path: &Path) -> Result<PathBuf, CredentialError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CredentialError::Io(error.to_string()))?
        .as_nanos();
    let temp_path = std::env::temp_dir().join(format!(
        "codex-quota-edge-cookies-{}-{timestamp}.sqlite",
        std::process::id()
    ));
    copy_locked_file(db_path, &temp_path)
        .map_err(|error| CredentialError::Io(format!("could not copy Edge cookie DB: {error}")))?;
    copy_sidecar_if_present(db_path, &temp_path, "-wal")?;
    copy_sidecar_if_present(db_path, &temp_path, "-shm")?;
    Ok(temp_path)
}

fn read_cookies_from_profile_db(
    db_path: &Path,
    aes_key: &[u8],
) -> Result<Vec<BrowserCookie>, CredentialError> {
    match copy_cookie_db_to_temp(db_path) {
        Ok(temp_path) => {
            let result = read_cookies_from_db(&temp_path, Some(aes_key));
            let _ = fs::remove_file(temp_path);
            result
        }
        Err(copy_error) => read_cookies_from_db(db_path, Some(aes_key)).map_err(|read_error| {
            CredentialError::Io(format!(
                "could not copy or read Edge cookie DB: copy failed ({copy_error}); read failed ({read_error})"
            ))
        }),
    }
}

fn copy_sidecar_if_present(
    source_db: &Path,
    temp_db: &Path,
    suffix: &str,
) -> Result<(), CredentialError> {
    let source = path_with_suffix(source_db, suffix);
    if !source.exists() {
        return Ok(());
    }
    let target = path_with_suffix(temp_db, suffix);
    copy_locked_file(&source, &target)
        .map(|_| ())
        .map_err(|error| {
            CredentialError::Io(format!(
                "could not copy Edge cookie sidecar {suffix}: {error}"
            ))
        })
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(windows)]
fn copy_locked_file(source: &Path, target: &Path) -> io::Result<u64> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(source)?;
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)?;
    io::copy(&mut input, &mut output)
}

#[cfg(not(windows))]
fn copy_locked_file(source: &Path, target: &Path) -> io::Result<u64> {
    fs::copy(source, target)
}

fn is_chatgpt_host(host: &str) -> bool {
    let normalized = host.trim_start_matches('.').to_ascii_lowercase();
    normalized == "chatgpt.com" || normalized.ends_with(".chatgpt.com")
}
