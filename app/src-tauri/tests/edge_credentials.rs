use app_lib::edge_credentials::{
    build_cookie_header, cookie_header_from_override, decrypt_chromium_cookie_value,
    decrypt_dpapi_blob, discover_cookie_db_paths, edge_user_data_dir_from_local_app_data,
    extract_dpapi_wrapped_key, load_edge_cookie_header, read_cookies_from_db,
    user_facing_credential_error, BrowserCookie, CredentialError,
};
use std::{fs, path::Path};

#[test]
fn cookie_header_includes_only_chatgpt_cookies() {
    let cookies = vec![
        BrowserCookie::new(
            ".chatgpt.com",
            "__Secure-next-auth.session-token",
            "session",
        ),
        BrowserCookie::new("chatgpt.com", "oai-sc", "csrf"),
        BrowserCookie::new(".openai.com", "other", "skip"),
        BrowserCookie::new(".chatgpt.com", "empty", ""),
    ];

    let header = build_cookie_header(&cookies).expect("expected usable cookies");

    assert_eq!(
        header,
        "__Secure-next-auth.session-token=session; oai-sc=csrf"
    );
}

#[test]
fn cookie_header_is_none_without_chatgpt_cookies() {
    let cookies = vec![BrowserCookie::new(".openai.com", "other", "skip")];

    assert_eq!(build_cookie_header(&cookies), None);
}

#[test]
fn cookie_header_override_trims_and_rejects_empty_values() {
    assert_eq!(
        cookie_header_from_override(Some("  a=b; c=d  ")),
        Some("a=b; c=d".to_owned())
    );
    assert_eq!(cookie_header_from_override(Some("   ")), None);
    assert_eq!(cookie_header_from_override(None), None);
}

#[test]
fn cookie_header_override_accepts_copied_cookie_header_line() {
    assert_eq!(
        cookie_header_from_override(Some("Cookie: a=b; c=d")),
        Some("a=b; c=d".to_owned())
    );
    assert_eq!(
        cookie_header_from_override(Some("  cookie: a=b; c=d\r\n")),
        Some("a=b; c=d".to_owned())
    );
    assert_eq!(
        cookie_header_from_override(Some("COOKIE: a=b; c=d")),
        Some("a=b; c=d".to_owned())
    );
}

#[test]
fn locked_edge_cookie_db_gets_actionable_user_message() {
    let err = CredentialError::Io(
        "could not copy or read Edge cookie DB: copy failed (另一个程序正在使用此文件); read failed (unable to open database file: C:\\Users\\TR\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Network\\Cookies)"
            .to_owned(),
    );

    let message = user_facing_credential_error(&err);

    assert!(message.contains("Edge 正在锁定 Cookie 数据库"));
    assert!(message.contains("关闭 Edge"));
    assert!(message.contains("Cookie Header"));
}

#[test]
fn discovers_edge_cookie_databases_under_profiles() {
    let root = unique_temp_dir("edge-profile-discovery");
    make_cookie_db(root.join("Default").join("Network").join("Cookies"));
    make_cookie_db(root.join("Profile 1").join("Network").join("Cookies"));
    fs::create_dir_all(root.join("Guest Profile").join("Network")).unwrap();

    let paths = discover_cookie_db_paths(&root).expect("expected profile discovery");

    assert_eq!(paths.len(), 2);
    assert!(paths
        .iter()
        .any(|path| path.ends_with("Default\\Network\\Cookies")));
    assert!(paths
        .iter()
        .any(|path| path.ends_with("Profile 1\\Network\\Cookies")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn extracts_dpapi_wrapped_key_from_local_state() {
    let local_state = r#"{"os_crypt":{"encrypted_key":"RFBBUElBQkM="}}"#;

    let key = extract_dpapi_wrapped_key(local_state).expect("expected encrypted key");

    assert_eq!(key, b"ABC");
}

#[test]
fn resolves_edge_user_data_dir_from_local_app_data() {
    let root = std::path::PathBuf::from(r"C:\Users\TR\AppData\Local");

    let dir = edge_user_data_dir_from_local_app_data(&root);

    assert_eq!(
        dir,
        std::path::PathBuf::from(r"C:\Users\TR\AppData\Local\Microsoft\Edge\User Data")
    );
}

#[test]
fn decrypts_v10_chromium_cookie_value() {
    let key: Vec<u8> = (0..32).collect();
    let nonce: Vec<u8> = (0..12).collect();
    let encrypted = hex_bytes("3467a568ac8aac36fb20fbfed4c63ea359859cfd22d4dcc755977f28c1");
    let mut value = b"v10".to_vec();
    value.extend_from_slice(&nonce);
    value.extend_from_slice(&encrypted);

    let decrypted = decrypt_chromium_cookie_value(&value, &key).expect("expected decryption");

    assert_eq!(decrypted, "session-value");
}

#[test]
fn reports_v20_chromium_cookie_value_as_unsupported() {
    let key: Vec<u8> = (0..32).collect();

    let err = decrypt_chromium_cookie_value(b"v20encrypted", &key).unwrap_err();

    assert_eq!(
        err,
        CredentialError::UnsupportedEncryption(
            "v20 app-bound encrypted cookies are not supported".to_owned()
        )
    );
}

#[test]
fn reads_plain_chatgpt_cookies_from_sqlite_db() {
    let root = unique_temp_dir("edge-cookie-db");
    let db_path = root.join("Cookies");
    create_minimal_cookie_db(&db_path);

    let cookies = read_cookies_from_db(&db_path, None).expect("expected cookies");

    assert_eq!(
        cookies,
        vec![
            BrowserCookie::new(
                ".chatgpt.com",
                "__Secure-next-auth.session-token",
                "session"
            ),
            BrowserCookie::new("chatgpt.com", "oai-sc", "csrf"),
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn decrypts_windows_dpapi_blob() {
    let protected = protect_for_test(b"state-key");

    let decrypted = decrypt_dpapi_blob(&protected).expect("expected DPAPI decrypt");

    assert_eq!(decrypted, b"state-key");
}

#[test]
#[ignore = "reads the local Edge profile; run manually for live credential diagnostics"]
fn live_probe_edge_cookie_header_without_printing_secret() {
    let header = load_edge_cookie_header().expect("expected Edge ChatGPT cookies");

    assert!(header.contains('='));
    println!(
        "loaded chatgpt.com cookie header without printing secrets; length={}",
        header.len()
    );
}

fn make_cookie_db(path: impl AsRef<Path>) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"sqlite").unwrap();
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn create_minimal_cookie_db(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE cookies (
            host_key TEXT NOT NULL,
            name TEXT NOT NULL,
            value TEXT NOT NULL,
            encrypted_value BLOB NOT NULL
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cookies (host_key, name, value, encrypted_value) VALUES (?1, ?2, ?3, x'')",
        (
            ".chatgpt.com",
            "__Secure-next-auth.session-token",
            "session",
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cookies (host_key, name, value, encrypted_value) VALUES (?1, ?2, ?3, x'')",
        ("chatgpt.com", "oai-sc", "csrf"),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cookies (host_key, name, value, encrypted_value) VALUES (?1, ?2, ?3, x'')",
        (".openai.com", "other", "skip"),
    )
    .unwrap();
}

fn hex_bytes(input: &str) -> Vec<u8> {
    input
        .as_bytes()
        .chunks(2)
        .map(|chunk| {
            let hex = std::str::from_utf8(chunk).unwrap();
            u8::from_str_radix(hex, 16).unwrap()
        })
        .collect()
}

#[cfg(windows)]
fn protect_for_test(data: &[u8]) -> Vec<u8> {
    use std::ptr::null;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB},
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe { CryptProtectData(&input, null(), null(), null(), null(), 0, &mut output) };
    assert_ne!(ok, 0, "CryptProtectData failed");

    let protected =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut _);
    }
    protected
}
