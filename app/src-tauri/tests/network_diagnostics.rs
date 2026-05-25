use app_lib::network_diagnostics::{
    build_connection_summary, parse_proxy_socket_candidate, EndpointDiagnostic, ProxyDiagnostics,
};

#[test]
fn parses_common_windows_proxy_server_formats() {
    assert_eq!(
        parse_proxy_socket_candidate("127.0.0.1:7897"),
        Some("127.0.0.1:7897".to_owned())
    );
    assert_eq!(
        parse_proxy_socket_candidate("http=127.0.0.1:7897;https=127.0.0.1:7897"),
        Some("127.0.0.1:7897".to_owned())
    );
    assert_eq!(
        parse_proxy_socket_candidate("https=http://localhost:7897"),
        Some("localhost:7897".to_owned())
    );
}

#[test]
fn connection_summary_reports_proxy_and_endpoint_health() {
    let proxy = ProxyDiagnostics {
        enabled: true,
        source: "Windows 系统代理".to_owned(),
        server: Some("127.0.0.1:7897".to_owned()),
        local_probe: Some(true),
    };
    let endpoints = vec![
        EndpointDiagnostic {
            label: "session".to_owned(),
            url: "https://chatgpt.com/api/auth/session".to_owned(),
            reachable: true,
            status_code: Some(403),
            error: None,
        },
        EndpointDiagnostic {
            label: "usage".to_owned(),
            url: "https://chatgpt.com/backend-api/wham/usage".to_owned(),
            reachable: false,
            status_code: None,
            error: Some("connection refused".to_owned()),
        },
    ];

    let summary = build_connection_summary(&proxy, &endpoints);

    assert!(summary.contains("系统代理 127.0.0.1:7897 可连接"));
    assert!(summary.contains("1/2 个 ChatGPT 接口可达"));
}
