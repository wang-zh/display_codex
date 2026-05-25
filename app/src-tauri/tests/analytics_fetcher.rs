use app_lib::analytics::{
    classify_fetch_response, extract_access_token_from_html, summarize_analytics_body, FetchOutcome,
};

#[test]
fn classifies_redirected_login_page_as_login_required() {
    let outcome = classify_fetch_response(200, "https://chatgpt.com/auth/login");

    assert_eq!(outcome, FetchOutcome::LoginRequired);
}

#[test]
fn classifies_forbidden_as_login_required() {
    let outcome =
        classify_fetch_response(403, "https://chatgpt.com/codex/cloud/settings/analytics");

    assert_eq!(outcome, FetchOutcome::LoginRequired);
}

#[test]
fn classifies_successful_analytics_page_as_success() {
    let outcome =
        classify_fetch_response(200, "https://chatgpt.com/codex/cloud/settings/analytics");

    assert_eq!(outcome, FetchOutcome::Success);
}

#[test]
fn classifies_successful_wham_usage_endpoint_as_success() {
    let outcome = classify_fetch_response(200, "https://chatgpt.com/backend-api/wham/usage");

    assert_eq!(outcome, FetchOutcome::Success);
}

#[test]
fn response_diagnostics_describe_shell_without_body_snippet() {
    let summary = summarize_analytics_body(
        200,
        "https://chatgpt.com/codex/cloud/settings/analytics",
        Some("text/html; charset=utf-8"),
        "<html><script>self.__next_f.push([1,\"cloud/settings\"])</script></html>",
    );

    assert!(summary.contains("HTTP 200"));
    assert!(summary.contains("text/html"));
    assert!(summary.contains("next-flight"));
    assert!(!summary.contains("self.__next_f.push"));
}

#[test]
fn response_diagnostics_describe_usage_json_without_values() {
    let summary = summarize_analytics_body(
        200,
        "https://chatgpt.com/backend-api/wham/usage",
        Some("application/json"),
        r#"{"usage_limits":[{"window":"codex_5_hour","remaining_percent":0.93}]}"#,
    );

    assert!(summary.contains("usage-api"));
    assert!(summary.contains("json"));
    assert!(summary.contains("top-level-keys=usage_limits"));
    assert!(!summary.contains("0.93"));
}

#[test]
fn response_diagnostics_include_safe_rate_limit_values() {
    let summary = summarize_analytics_body(
        200,
        "https://chatgpt.com/backend-api/wham/usage",
        Some("application/json"),
        r#"{
          "email": "hidden@example.com",
          "rate_limit": {
            "primary_window": {
              "used": 2,
              "limit": 100,
              "used_percent": 2,
              "reset_after_seconds": 5300
            },
            "secondary_window": {
              "remaining_percent": 62,
              "reset_after_seconds": 432000
            }
          }
        }"#,
    );

    assert!(summary.contains(
        "rate-limit=primary_window(used=2,limit=100,used_percent=2,reset_after_seconds=5300)"
    ));
    assert!(summary.contains("secondary_window(remaining_percent=62,reset_after_seconds=432000)"));
    assert!(!summary.contains("hidden@example.com"));
}

#[test]
fn extracts_access_token_from_client_bootstrap() {
    let html = r#"
<script type="application/json" id="client-bootstrap">
{"authStatus":"logged_in","session":{"accessToken":"token-value","user":{"id":"user-1"}}}
</script>
"#;

    let token = extract_access_token_from_html(html).expect("expected access token");

    assert_eq!(token, "token-value");
}

#[test]
fn extract_access_token_ignores_html_without_bootstrap_token() {
    let token = extract_access_token_from_html("<html></html>");

    assert_eq!(token, None);
}
