use std::{error::Error, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOutcome {
    Success,
    LoginRequired,
    NetworkError,
}

#[derive(Debug)]
pub enum AnalyticsError {
    LoginRequired(String),
    Network(String),
}

impl std::fmt::Display for AnalyticsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyticsError::LoginRequired(message) | AnalyticsError::Network(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for AnalyticsError {}

const ANALYTICS_PAGE_URL: &str = "https://chatgpt.com/codex/cloud/settings/analytics";
const AUTH_SESSION_URL: &str = "https://chatgpt.com/api/auth/session";
const ACCESSIBLE_LINKS_URL: &str =
    "https://chatgpt.com/backend-api/aip/connectors/links/list_accessible";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub struct AnalyticsPage {
    pub body: String,
    pub diagnostic_summary: String,
}

pub fn fetch_analytics_page(cookie_header: &str) -> Result<AnalyticsPage, AnalyticsError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Edg/136.0.0.0")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| {
            AnalyticsError::Network(format!(
                "failed to build HTTP client: {}",
                describe_error_chain(&error)
            ))
        })?;
    let access_token = fetch_access_token(&client, cookie_header)?;
    warm_up_accessible_links(&client, cookie_header, &access_token);
    let response = client
        .get(USAGE_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .bearer_auth(&access_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .header(reqwest::header::REFERER, ANALYTICS_PAGE_URL)
        .header(
            "sec-ch-ua",
            r#""Chromium";v="136", "Microsoft Edge";v="136", "Not.A/Brand";v="99""#,
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .send()
        .map_err(|error| network_request_error("usage request", USAGE_URL, error))?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    match classify_fetch_response(status, &final_url) {
        FetchOutcome::Success => {
            let body = response
                .text()
                .map_err(|error| network_request_error("usage response body", USAGE_URL, error))?;
            let diagnostic_summary =
                summarize_analytics_body(status, &final_url, content_type.as_deref(), &body);
            Ok(AnalyticsPage {
                body,
                diagnostic_summary,
            })
        }
        FetchOutcome::LoginRequired => Err(AnalyticsError::LoginRequired(login_required_message())),
        FetchOutcome::NetworkError => Err(AnalyticsError::Network(format!(
            "analytics request failed with HTTP {status}"
        ))),
    }
}

pub fn fetch_analytics_text(cookie_header: &str) -> Result<String, AnalyticsError> {
    fetch_analytics_page(cookie_header).map(|page| page.body)
}

pub fn classify_fetch_response(status: u16, final_url: &str) -> FetchOutcome {
    if status == 401 || status == 403 {
        return FetchOutcome::LoginRequired;
    }

    if final_url.contains("/auth/login")
        || final_url.contains("/login")
        || (!final_url.contains("/codex/cloud/settings/analytics")
            && !final_url.contains("/backend-api/wham/usage"))
    {
        return FetchOutcome::LoginRequired;
    }

    if (200..300).contains(&status) {
        FetchOutcome::Success
    } else {
        FetchOutcome::NetworkError
    }
}

pub fn summarize_analytics_body(
    status: u16,
    final_url: &str,
    content_type: Option<&str>,
    body: &str,
) -> String {
    let lower = body.to_ascii_lowercase();
    let mut markers = Vec::new();
    let mut top_level_keys = None;

    if body.contains("5 小时") || body.contains("5小时") || lower.contains("5-hour") {
        markers.push("quota-5h");
    }
    if lower.contains("5_hour") || lower.contains("five_hour") {
        markers.push("quota-5h-json");
    }
    if body.contains("每周") || lower.contains("weekly") {
        markers.push("quota-weekly");
    }
    if body.contains("self.__next_f") || body.contains("__next_f") {
        markers.push("next-flight");
    }
    if body.contains("__NEXT_DATA__") || lower.contains("next.js") {
        markers.push("next-app");
    }
    if lower.contains("cloudflare") || lower.contains("__cf_chl") {
        markers.push("cloudflare");
    }
    if lower.contains("/auth/login") || lower.contains("sign in") || lower.contains("login") {
        markers.push("login-page");
    }
    if lower.contains("codex/cloud/settings/analytics") {
        markers.push("codex-route");
    }
    if final_url.contains("/backend-api/wham/usage") {
        markers.push("usage-api");
    }
    if content_type.is_some_and(|content_type| content_type.to_ascii_lowercase().contains("json"))
        || serde_json::from_str::<serde_json::Value>(body).is_ok()
    {
        markers.push("json");
    }
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) {
        let keys = map.keys().take(8).cloned().collect::<Vec<_>>();
        if !keys.is_empty() {
            top_level_keys = Some(keys.join(","));
        }
    }
    if !markers.iter().any(|marker| marker.starts_with("quota-")) {
        markers.push("no-quota-labels");
    }

    let top_level_keys = top_level_keys
        .map(|keys| format!("; top-level-keys={keys}"))
        .unwrap_or_default();
    let rate_limit = summarize_rate_limit_windows(body)
        .map(|summary| format!("; rate-limit={summary}"))
        .unwrap_or_default();

    format!(
        "HTTP {status}; content-type={}; bytes={}; url={}; markers={}{}{}",
        content_type.unwrap_or("unknown"),
        body.len(),
        final_url,
        markers.join(","),
        top_level_keys,
        rate_limit
    )
}

fn summarize_rate_limit_windows(body: &str) -> Option<String> {
    const WINDOW_NAMES: [&str; 2] = ["primary_window", "secondary_window"];
    const FIELD_NAMES: [&str; 6] = [
        "used",
        "limit",
        "used_percent",
        "remaining",
        "remaining_percent",
        "reset_after_seconds",
    ];

    let parsed = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let rate_limit = parsed.get("rate_limit")?.as_object()?;
    let windows = WINDOW_NAMES
        .iter()
        .filter_map(|window_name| {
            let window = rate_limit.get(*window_name)?.as_object()?;
            let fields = FIELD_NAMES
                .iter()
                .filter_map(|field_name| {
                    let value = window.get(*field_name)?;
                    value
                        .as_i64()
                        .map(|number| format!("{field_name}={number}"))
                        .or_else(|| {
                            value
                                .as_f64()
                                .map(|number| format!("{field_name}={number}"))
                        })
                })
                .collect::<Vec<_>>();

            (!fields.is_empty()).then(|| format!("{window_name}({})", fields.join(",")))
        })
        .collect::<Vec<_>>();

    (!windows.is_empty()).then(|| windows.join(";"))
}

pub fn extract_access_token_from_html(html: &str) -> Option<String> {
    client_bootstrap_json(html)
        .and_then(extract_access_token_from_json)
        .or_else(|| find_json_string_value(html, "accessToken"))
}

fn fetch_access_token(
    client: &reqwest::blocking::Client,
    cookie_header: &str,
) -> Result<String, AnalyticsError> {
    resolve_access_token(
        fetch_access_token_from_session(client, cookie_header),
        || fetch_access_token_from_analytics_page(client, cookie_header),
    )
}

fn resolve_access_token(
    session_result: Result<Option<String>, AnalyticsError>,
    analytics_page_result: impl FnOnce() -> Result<Option<String>, AnalyticsError>,
) -> Result<String, AnalyticsError> {
    match session_result {
        Ok(Some(token)) => Ok(token),
        Ok(None) => missing_access_token_error(analytics_page_result()?),
        Err(session_error) => match analytics_page_result() {
            Ok(Some(token)) => Ok(token),
            Ok(None) => Err(AnalyticsError::LoginRequired(format!(
                "ChatGPT 页面登录态可用，但没有拿到 backend-api 访问令牌。请重新打开 analytics 页面并复制同一会话的 Cookie Header。session 检查先失败：{session_error}"
            ))),
            Err(page_error) => Err(combine_access_token_errors(session_error, page_error)),
        },
    }
}

fn missing_access_token_error(token: Option<String>) -> Result<String, AnalyticsError> {
    token.ok_or_else(|| {
        AnalyticsError::LoginRequired(
            "ChatGPT 页面登录态可用，但没有拿到 backend-api 访问令牌。请重新打开 analytics 页面并复制同一会话的 Cookie Header。"
                .to_owned(),
        )
    })
}

fn combine_access_token_errors(
    session_error: AnalyticsError,
    page_error: AnalyticsError,
) -> AnalyticsError {
    match (session_error, page_error) {
        (AnalyticsError::Network(session), AnalyticsError::Network(page)) => {
            AnalyticsError::Network(format!(
                "session request failed: {session}; analytics page fallback failed: {page}"
            ))
        }
        (AnalyticsError::Network(session), AnalyticsError::LoginRequired(page)) => {
            AnalyticsError::LoginRequired(format!("{page} session request failed first: {session}"))
        }
        (AnalyticsError::LoginRequired(session), AnalyticsError::Network(page)) => {
            AnalyticsError::Network(format!(
                "session login check failed: {session}; analytics page fallback failed: {page}"
            ))
        }
        (AnalyticsError::LoginRequired(session), AnalyticsError::LoginRequired(page)) => {
            AnalyticsError::LoginRequired(format!(
                "{page} session login check also failed: {session}"
            ))
        }
    }
}

fn fetch_access_token_from_session(
    client: &reqwest::blocking::Client,
    cookie_header: &str,
) -> Result<Option<String>, AnalyticsError> {
    let response = client
        .get(AUTH_SESSION_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .header(reqwest::header::REFERER, ANALYTICS_PAGE_URL)
        .send()
        .map_err(|error| network_request_error("session request", AUTH_SESSION_URL, error))?;

    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AnalyticsError::Network(format!(
            "session request failed with HTTP {}",
            response.status().as_u16()
        )));
    }

    let body = response
        .text()
        .map_err(|error| network_request_error("session response body", AUTH_SESSION_URL, error))?;
    Ok(extract_access_token_from_json(&body))
}

fn fetch_access_token_from_analytics_page(
    client: &reqwest::blocking::Client,
    cookie_header: &str,
) -> Result<Option<String>, AnalyticsError> {
    let response = client
        .get(ANALYTICS_PAGE_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .map_err(|error| {
            network_request_error("analytics page request", ANALYTICS_PAGE_URL, error)
        })?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    match classify_fetch_response(status, &final_url) {
        FetchOutcome::Success => {
            let body = response.text().map_err(|error| {
                network_request_error("analytics page response body", ANALYTICS_PAGE_URL, error)
            })?;
            Ok(extract_access_token_from_html(&body))
        }
        FetchOutcome::LoginRequired => Err(AnalyticsError::LoginRequired(login_required_message())),
        FetchOutcome::NetworkError => Err(AnalyticsError::Network(format!(
            "analytics page request failed with HTTP {status}"
        ))),
    }
}

fn login_required_message() -> String {
    "ChatGPT 登录态未被接受。请在 Edge 打开 ChatGPT analytics 页面确认已登录，或在设置里粘贴同一会话的 Cookie Header。"
        .to_owned()
}

fn extract_access_token_from_json(json: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(json).ok()?;
    parsed
        .pointer("/accessToken")
        .or_else(|| parsed.pointer("/session/accessToken"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn client_bootstrap_json(html: &str) -> Option<&str> {
    let id_at = html.find("client-bootstrap")?;
    let before = &html[..id_at];
    let script_start = before.rfind("<script")?;
    let after_start = &html[script_start..];
    let content_start = after_start.find('>')? + script_start + 1;
    let content_end = html[content_start..].find("</script>")? + content_start;
    Some(html[content_start..content_end].trim())
}

fn find_json_string_value(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = text.find(&needle)? + needle.len();
    let mut value = String::new();
    let mut escaped = false;
    for ch in text[start..].chars() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => break,
            _ => value.push(ch),
        }
    }

    (!value.trim().is_empty()).then(|| value)
}

fn warm_up_accessible_links(
    client: &reqwest::blocking::Client,
    cookie_header: &str,
    access_token: &str,
) {
    let _ = client
        .get(ACCESSIBLE_LINKS_URL)
        .header(reqwest::header::COOKIE, cookie_header)
        .bearer_auth(access_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.7")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .header(reqwest::header::REFERER, ANALYTICS_PAGE_URL)
        .header(
            "sec-ch-ua",
            r#""Chromium";v="136", "Microsoft Edge";v="136", "Not.A/Brand";v="99""#,
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", r#""Windows""#)
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .send();
}

fn network_request_error(stage: &str, url: &str, error: reqwest::Error) -> AnalyticsError {
    let summary = describe_error_chain(&error);
    AnalyticsError::Network(format!(
        "{stage} failed for {url}: {}",
        add_network_hint(&summary)
    ))
}

fn describe_error_chain(error: &(dyn Error + 'static)) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(error) = source {
        let message = error.to_string();
        if !messages.iter().any(|existing| existing == &message) {
            messages.push(message);
        }
        source = error.source();
    }
    messages.join("; caused by: ")
}

fn add_network_hint(summary: &str) -> String {
    if summary.contains("os error 10061") || summary.contains("积极拒绝") {
        format!(
            "{summary}。提示：检测到连接被拒绝；如果 Windows 系统代理指向 127.0.0.1 本地端口，请确认 Clash Verge/mihomo 等代理程序已启动，且系统代理端口正在监听。"
        )
    } else {
        summary.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{error::Error, fmt};

    #[test]
    fn session_network_error_falls_back_to_analytics_page_token() {
        let token = resolve_access_token(
            Err(AnalyticsError::Network(
                "session request failed for https://chatgpt.com/api/auth/session".to_owned(),
            )),
            || Ok(Some("page-token".to_owned())),
        )
        .expect("expected page fallback token");

        assert_eq!(token, "page-token");
    }

    #[test]
    fn combined_network_error_reports_session_and_page_failures() {
        let error = resolve_access_token(
            Err(AnalyticsError::Network(
                "session request failed for https://chatgpt.com/api/auth/session".to_owned(),
            )),
            || {
                Err(AnalyticsError::Network(
                    "analytics page request failed for https://chatgpt.com/codex/cloud/settings/analytics"
                        .to_owned(),
                ))
            },
        )
        .expect_err("expected combined network error");

        let AnalyticsError::Network(message) = error else {
            panic!("expected network error");
        };
        assert!(message.contains("session request failed"));
        assert!(message.contains("analytics page fallback failed"));
    }

    #[test]
    fn network_error_summary_includes_nested_sources() {
        let error = OuterError;

        let summary = describe_error_chain(&error);

        assert!(summary.contains("outer send failed"));
        assert!(summary.contains("inner connection reset"));
    }

    #[test]
    fn network_error_summary_hints_at_proxy_for_refused_connection() {
        let summary = add_network_hint("由于目标计算机积极拒绝，无法连接。 (os error 10061)");

        assert!(summary.contains("系统代理"));
        assert!(summary.contains("127.0.0.1"));
    }

    #[derive(Debug)]
    struct OuterError;

    impl fmt::Display for OuterError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer send failed")
        }
    }

    impl Error for OuterError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&InnerError)
        }
    }

    #[derive(Debug)]
    struct InnerError;

    impl fmt::Display for InnerError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner connection reset")
        }
    }

    impl Error for InnerError {}
}
