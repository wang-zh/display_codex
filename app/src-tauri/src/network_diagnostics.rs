use serde::Serialize;
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

const ENDPOINTS: [(&str, &str); 3] = [
    ("session", "https://chatgpt.com/api/auth/session"),
    (
        "analytics",
        "https://chatgpt.com/codex/cloud/settings/analytics",
    ),
    ("usage", "https://chatgpt.com/backend-api/wham/usage"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDiagnostics {
    pub proxy: ProxyDiagnostics,
    pub endpoints: Vec<EndpointDiagnostic>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyDiagnostics {
    pub enabled: bool,
    pub source: String,
    pub server: Option<String>,
    pub local_probe: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointDiagnostic {
    pub label: String,
    pub url: String,
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub error: Option<String>,
}

pub fn collect_connection_diagnostics() -> ConnectionDiagnostics {
    let proxy = detect_proxy();
    let endpoints = probe_chatgpt_endpoints();
    let summary = build_connection_summary(&proxy, &endpoints);

    ConnectionDiagnostics {
        proxy,
        endpoints,
        summary,
    }
}

pub fn build_connection_summary(
    proxy: &ProxyDiagnostics,
    endpoints: &[EndpointDiagnostic],
) -> String {
    let reachable = endpoints
        .iter()
        .filter(|endpoint| endpoint.reachable)
        .count();
    let proxy_summary = if !proxy.enabled {
        proxy.source.clone()
    } else {
        match (&proxy.server, proxy.local_probe) {
            (Some(server), Some(true)) => format!("系统代理 {server} 可连接"),
            (Some(server), Some(false)) => format!("系统代理 {server} 无法连接"),
            (Some(server), None) => format!("{} {server}", proxy.source),
            (None, _) if proxy.enabled => proxy.source.clone(),
            _ => "未检测到系统代理".to_owned(),
        }
    };

    format!(
        "{proxy_summary}；{reachable}/{} 个 ChatGPT 接口可达",
        endpoints.len()
    )
}

pub fn parse_proxy_socket_candidate(proxy_server: &str) -> Option<String> {
    let trimmed = proxy_server.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if trimmed.contains('=') {
        trimmed
            .split(';')
            .find_map(|part| {
                let (scheme, value) = part.split_once('=')?;
                (scheme.trim().eq_ignore_ascii_case("https")
                    || scheme.trim().eq_ignore_ascii_case("http"))
                .then(|| value.trim())
            })
            .or_else(|| {
                trimmed
                    .split(';')
                    .next()
                    .and_then(|part| part.split_once('='))
                    .map(|(_, value)| value.trim())
            })?
    } else {
        trimmed
    };

    let without_scheme = candidate
        .strip_prefix("http://")
        .or_else(|| candidate.strip_prefix("https://"))
        .unwrap_or(candidate);
    let host_port = without_scheme.split('/').next()?.trim();

    (!host_port.is_empty()).then(|| host_port.to_owned())
}

fn detect_proxy() -> ProxyDiagnostics {
    let mut proxy = platform_proxy_settings().unwrap_or_else(env_proxy_settings);
    if proxy.enabled {
        if let Some(server) = proxy.server.as_deref() {
            proxy.local_probe = parse_proxy_socket_candidate(server)
                .filter(|socket| is_local_proxy_socket(socket))
                .map(|socket| probe_tcp_socket(&socket));
        }
    }
    proxy
}

fn env_proxy_settings() -> ProxyDiagnostics {
    let server = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty());

    ProxyDiagnostics {
        enabled: server.is_some(),
        source: server
            .as_ref()
            .map(|_| "环境变量代理".to_owned())
            .unwrap_or_else(|| "未检测到系统代理".to_owned()),
        server,
        local_probe: None,
    }
}

#[cfg(windows)]
fn platform_proxy_settings() -> Option<ProxyDiagnostics> {
    windows_proxy_settings()
}

#[cfg(not(windows))]
fn platform_proxy_settings() -> Option<ProxyDiagnostics> {
    None
}

fn is_local_proxy_socket(socket: &str) -> bool {
    socket.starts_with("127.0.0.1:")
        || socket.starts_with("localhost:")
        || socket.starts_with("[::1]:")
}

fn probe_tcp_socket(socket: &str) -> bool {
    let Ok(mut addresses) = socket.to_socket_addrs() else {
        return false;
    };
    let Some(address) = addresses.next() else {
        return false;
    };

    TcpStream::connect_timeout(&address, Duration::from_millis(700)).is_ok()
}

fn probe_chatgpt_endpoints() -> Vec<EndpointDiagnostic> {
    let client = match reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) CodexQuotaWidget/0.1")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ENDPOINTS
                .iter()
                .map(|(label, url)| EndpointDiagnostic {
                    label: (*label).to_owned(),
                    url: (*url).to_owned(),
                    reachable: false,
                    status_code: None,
                    error: Some(format!("HTTP client build failed: {error}")),
                })
                .collect();
        }
    };

    ENDPOINTS
        .iter()
        .map(|(label, url)| probe_endpoint(&client, label, url))
        .collect()
}

fn probe_endpoint(
    client: &reqwest::blocking::Client,
    label: &str,
    url: &str,
) -> EndpointDiagnostic {
    match client
        .get(url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
    {
        Ok(response) => EndpointDiagnostic {
            label: label.to_owned(),
            url: url.to_owned(),
            reachable: true,
            status_code: Some(response.status().as_u16()),
            error: None,
        },
        Err(error) => EndpointDiagnostic {
            label: label.to_owned(),
            url: url.to_owned(),
            reachable: false,
            status_code: None,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(windows)]
fn windows_proxy_settings() -> Option<ProxyDiagnostics> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
            REG_DWORD, REG_EXPAND_SZ, REG_SZ,
        },
    };

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    unsafe fn query_dword(hkey: HKEY, name: &str) -> Option<u32> {
        let mut value_type = 0;
        let mut data = [0_u8; 4];
        let mut data_len = data.len() as u32;
        let name = wide(name);
        let status = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            data.as_mut_ptr(),
            &mut data_len,
        );

        (status == ERROR_SUCCESS && value_type == REG_DWORD && data_len == 4)
            .then(|| u32::from_le_bytes(data))
    }

    unsafe fn query_string(hkey: HKEY, name: &str) -> Option<String> {
        let mut value_type = 0;
        let mut buffer = vec![0_u16; 1024];
        let mut data_len = (buffer.len() * 2) as u32;
        let name = wide(name);
        let status = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            buffer.as_mut_ptr().cast::<u8>(),
            &mut data_len,
        );

        if status != ERROR_SUCCESS || (value_type != REG_SZ && value_type != REG_EXPAND_SZ) {
            return None;
        }

        let units = (data_len as usize / 2).min(buffer.len());
        if units == 0 {
            return None;
        }
        let without_nul = buffer
            .get(..units)
            .unwrap_or_default()
            .iter()
            .copied()
            .take_while(|unit| *unit != 0)
            .collect::<Vec<_>>();

        String::from_utf16(&without_nul)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
    let mut hkey: HKEY = ptr::null_mut();
    let opened =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) };
    if opened != ERROR_SUCCESS {
        return None;
    }

    let enabled = unsafe { query_dword(hkey, "ProxyEnable") }.unwrap_or(0) != 0;
    let server = unsafe { query_string(hkey, "ProxyServer") };
    unsafe {
        RegCloseKey(hkey);
    }

    Some(ProxyDiagnostics {
        enabled: enabled && server.is_some(),
        source: if enabled {
            "Windows 系统代理".to_owned()
        } else {
            "Windows 系统代理未启用".to_owned()
        },
        server,
        local_probe: None,
    })
}
