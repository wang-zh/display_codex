use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaKind {
    FiveHour,
    Weekly,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaEntry {
    pub kind: QuotaKind,
    pub remaining_percent: u8,
    pub reset_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaState {
    pub entries: Vec<QuotaEntry>,
    pub last_refresh_at: Option<String>,
    pub next_refresh_at: Option<String>,
    pub source: QuotaSource,
    pub status: QuotaStatus,
    pub error_summary: Option<String>,
}

impl QuotaState {
    pub fn idle() -> Self {
        Self {
            entries: Vec::new(),
            last_refresh_at: None,
            next_refresh_at: None,
            source: QuotaSource::Cache,
            status: QuotaStatus::Idle,
            error_summary: None,
        }
    }

    pub fn from_entries(entries: Vec<QuotaEntry>) -> Self {
        Self {
            entries,
            last_refresh_at: None,
            next_refresh_at: None,
            source: QuotaSource::Live,
            status: QuotaStatus::Ok,
            error_summary: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    Live,
    Cache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Idle,
    Refreshing,
    Ok,
    Stale,
    LoginRequired,
    ParseError,
    NetworkError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaParseError {
    MissingCard(&'static str),
    MissingPercent(&'static str),
    MissingReset(&'static str),
    InvalidPercent(String),
}

pub fn user_facing_parse_error(error: &QuotaParseError) -> String {
    match error {
        QuotaParseError::MissingCard("5-hour") => {
            "解析失败：usage 接口响应中没有找到 5 小时额度。请确认 Cookie Header 来自同一个已登录的 chatgpt.com 会话。".to_owned()
        }
        QuotaParseError::MissingCard("weekly") => {
            "解析失败：usage 接口响应中没有找到每周额度。请确认 Cookie Header 来自同一个已登录的 chatgpt.com 会话。".to_owned()
        }
        QuotaParseError::MissingPercent(card) => {
            format!("解析失败：额度卡 {card} 中没有找到剩余百分比。")
        }
        QuotaParseError::MissingReset(card) => {
            format!("解析失败：额度卡 {card} 中没有找到重置时间。")
        }
        QuotaParseError::InvalidPercent(line) => {
            format!("解析失败：无法识别百分比：{line}")
        }
        QuotaParseError::MissingCard(card) => {
            format!("解析失败：响应中没有找到额度卡：{card}。")
        }
    }
}

const FIVE_HOUR_LABELS: &[&str] = &[
    "5 小时使用限额",
    "5小时使用限额",
    "5-hour usage limit",
    "5 hour usage limit",
    "5-hour limit",
];
const WEEKLY_LABELS: &[&str] = &["每周使用限额", "weekly usage limit", "weekly limit"];
const RESET_LABELS: &[&str] = &["重置时间", "reset time"];

pub fn parse_analytics_text(text: &str) -> Result<QuotaState, QuotaParseError> {
    if let Ok(json) = serde_json::from_str::<Value>(text) {
        if let Ok(state) = parse_usage_json_value(&json) {
            return Ok(state);
        }
    }

    let normalized = normalize_analytics_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();

    let entries = vec![
        parse_card(
            &normalized,
            &normalized_lower,
            QuotaKind::FiveHour,
            FIVE_HOUR_LABELS,
            "5-hour",
        )?,
        parse_card(
            &normalized,
            &normalized_lower,
            QuotaKind::Weekly,
            WEEKLY_LABELS,
            "weekly",
        )?,
    ];

    Ok(QuotaState::from_entries(entries))
}

fn parse_usage_json_value(value: &Value) -> Result<QuotaState, QuotaParseError> {
    if let Some(state) = parse_wham_rate_limit_windows(value)? {
        return Ok(state);
    }

    let entries = vec![
        find_json_quota_entry(value, QuotaKind::FiveHour, "5-hour")?,
        find_json_quota_entry(value, QuotaKind::Weekly, "weekly")?,
    ];

    Ok(QuotaState::from_entries(entries))
}

fn parse_wham_rate_limit_windows(value: &Value) -> Result<Option<QuotaState>, QuotaParseError> {
    let Some(rate_limit) = value.get("rate_limit") else {
        return Ok(None);
    };
    let Some(primary_window) = rate_limit.get("primary_window") else {
        return Ok(None);
    };
    let Some(secondary_window) = rate_limit.get("secondary_window") else {
        return Ok(None);
    };

    let entries = vec![
        parse_json_window_entry(primary_window, QuotaKind::FiveHour, "5-hour")?,
        parse_json_window_entry(secondary_window, QuotaKind::Weekly, "weekly")?,
    ];

    Ok(Some(QuotaState::from_entries(entries)))
}

fn parse_json_window_entry(
    value: &Value,
    kind: QuotaKind,
    error_name: &'static str,
) -> Result<QuotaEntry, QuotaParseError> {
    let remaining_percent =
        find_json_remaining_percent(value).ok_or(QuotaParseError::MissingPercent(error_name))?;
    let reset_label =
        find_json_reset_label(value, kind).ok_or(QuotaParseError::MissingReset(error_name))?;

    Ok(QuotaEntry {
        kind,
        remaining_percent,
        reset_label,
    })
}

pub fn next_retry_delay_minutes(failure_count: u32) -> u32 {
    match failure_count {
        0 => 5,
        1 => 10,
        2 => 20,
        _ => 30,
    }
}

pub fn apply_refresh_failure(
    cached: Option<QuotaState>,
    status: QuotaStatus,
    error_summary: impl Into<String>,
) -> QuotaState {
    let mut state = cached.unwrap_or_else(|| QuotaState {
        entries: Vec::new(),
        last_refresh_at: None,
        next_refresh_at: None,
        source: QuotaSource::Cache,
        status,
        error_summary: None,
    });
    state.source = QuotaSource::Cache;
    state.status = status;
    state.error_summary = Some(error_summary.into());
    state
}

fn parse_card(
    text: &str,
    text_lower: &str,
    kind: QuotaKind,
    labels: &'static [&'static str],
    error_name: &'static str,
) -> Result<QuotaEntry, QuotaParseError> {
    let (start, label) =
        find_first_label(text_lower, labels).ok_or(QuotaParseError::MissingCard(error_name))?;
    let content_start = start + label.len();
    let end = find_next_card_start(text_lower, content_start).unwrap_or(text.len());
    let section = &text[start..end];
    let section_lower = &text_lower[start..end];

    let remaining_percent = parse_percent(section)?;

    let (reset_at, reset_label) = find_first_label(section_lower, RESET_LABELS)
        .ok_or(QuotaParseError::MissingReset(error_name))?;
    let reset_value = &section[reset_at + reset_label.len()..];
    let reset_label = clean_reset_label(reset_value);
    if reset_label.is_empty() {
        return Err(QuotaParseError::MissingReset(error_name));
    }

    Ok(QuotaEntry {
        kind,
        remaining_percent,
        reset_label,
    })
}

struct JsonQuotaSearch {
    matched_kind: bool,
    missing_percent: bool,
    missing_reset: bool,
}

fn find_json_quota_entry(
    value: &Value,
    kind: QuotaKind,
    error_name: &'static str,
) -> Result<QuotaEntry, QuotaParseError> {
    let mut path = Vec::new();
    let mut search = JsonQuotaSearch {
        matched_kind: false,
        missing_percent: false,
        missing_reset: false,
    };

    if let Some(entry) = visit_json_for_quota(value, &mut path, kind, &mut search) {
        return Ok(entry);
    }

    if search.matched_kind && search.missing_percent {
        return Err(QuotaParseError::MissingPercent(error_name));
    }
    if search.matched_kind && search.missing_reset {
        return Err(QuotaParseError::MissingReset(error_name));
    }

    Err(QuotaParseError::MissingCard(error_name))
}

fn visit_json_for_quota(
    value: &Value,
    path: &mut Vec<String>,
    kind: QuotaKind,
    search: &mut JsonQuotaSearch,
) -> Option<QuotaEntry> {
    match value {
        Value::Object(map) => {
            if json_object_matches_kind(map, path, kind) {
                search.matched_kind = true;
                let Some(remaining_percent) = find_json_remaining_percent(value) else {
                    search.missing_percent = true;
                    return None;
                };
                let Some(reset_label) = find_json_reset_label(value, kind) else {
                    search.missing_reset = true;
                    return None;
                };

                return Some(QuotaEntry {
                    kind,
                    remaining_percent,
                    reset_label,
                });
            }

            for (key, child) in map {
                path.push(key.to_owned());
                if let Some(entry) = visit_json_for_quota(child, path, kind, search) {
                    return Some(entry);
                }
                path.pop();
            }
            None
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| visit_json_for_quota(item, path, kind, search)),
        _ => None,
    }
}

fn json_object_matches_kind(map: &Map<String, Value>, path: &[String], kind: QuotaKind) -> bool {
    let mut hint = path.join(" ");
    for (key, value) in map {
        if is_kind_hint_key(key) {
            hint.push(' ');
            hint.push_str(key);
        }
        if let Value::String(value) = value {
            hint.push(' ');
            hint.push_str(value);
        }
    }

    let normalized = normalize_json_hint(&hint);
    match kind {
        QuotaKind::FiveHour => {
            normalized.contains("5hour")
                || normalized.contains("fivehour")
                || normalized.contains("5小时")
                || normalized.contains("5小時")
        }
        QuotaKind::Weekly => {
            normalized.contains("weekly")
                || normalized.contains("week")
                || normalized.contains("每周")
                || normalized.contains("每週")
        }
    }
}

fn is_kind_hint_key(key: &str) -> bool {
    let normalized = normalize_key(key);
    matches!(
        normalized.as_str(),
        "kind" | "type" | "name" | "label" | "title" | "window" | "period" | "bucket"
    )
}

fn find_json_remaining_percent(value: &Value) -> Option<u8> {
    find_keyed_number(value, |key| {
        let key = normalize_key(key);
        key.contains("remaining") && (key.contains("percent") || key.contains("pct"))
    })
    .and_then(normalize_percent_value)
    .or_else(|| {
        find_keyed_number(value, |key| {
            let key = normalize_key(key);
            key.contains("used") && (key.contains("percent") || key.contains("pct"))
        })
        .and_then(normalize_percent_value)
        .map(|used_percent| 100_u8.saturating_sub(used_percent))
    })
    .or_else(|| {
        let remaining = find_keyed_number(value, is_remaining_count_key)?;
        let limit = find_keyed_number(value, is_limit_key)?;
        percent_from_ratio(remaining, limit)
    })
    .or_else(|| {
        let used = find_keyed_number(value, |key| {
            let key = normalize_key(key);
            (key.contains("used") || key.contains("usage"))
                && !key.contains("percent")
                && !key.contains("pct")
                && !key.contains("time")
        })?;
        let limit = find_keyed_number(value, is_limit_key)?;
        let used_percent = percent_from_ratio(used, limit)?;
        Some(100_u8.saturating_sub(used_percent))
    })
}

fn find_json_reset_label(value: &Value, kind: QuotaKind) -> Option<String> {
    find_keyed_reset_label(value, kind)
}

fn find_keyed_number(value: &Value, predicate: impl Fn(&str) -> bool + Copy) -> Option<f64> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if predicate(key) {
                    if let Some(number) = value_as_f64(child) {
                        return Some(number);
                    }
                }
            }
            map.values()
                .find_map(|child| find_keyed_number(child, predicate))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_keyed_number(item, predicate)),
        _ => None,
    }
}

fn find_keyed_reset_label(value: &Value, kind: QuotaKind) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_reset_key(key) {
                    if is_reset_after_seconds_key(key) {
                        if let Some(seconds) = value_as_f64(child) {
                            return Some(format_reset_after_seconds(seconds.max(0.0) as u64, kind));
                        }
                    } else if let Some(label) = value_as_label(child) {
                        return Some(label);
                    }
                }
            }
            map.values()
                .find_map(|child| find_keyed_reset_label(child, kind))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_keyed_reset_label(item, kind)),
        _ => None,
    }
}

fn is_reset_key(key: &str) -> bool {
    let key = normalize_key(key);
    key.contains("reset") || key.contains("refresh") || key.contains("renew")
}

fn is_reset_after_seconds_key(key: &str) -> bool {
    let key = normalize_key(key);
    key.contains("seconds") || key.contains("duration") || key.contains("after")
}

fn is_limit_key(key: &str) -> bool {
    let key = normalize_key(key);
    key.contains("limit") || key.contains("total") || key.contains("cap") || key.contains("max")
}

fn is_remaining_count_key(key: &str) -> bool {
    let key = normalize_key(key);
    key.contains("remaining")
        && !key.contains("percent")
        && !key.contains("pct")
        && !key.contains("time")
        && !key.contains("second")
        && !key.contains("duration")
        && !key.contains("reset")
        && !key.contains("refresh")
        && !key.contains("after")
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_as_label(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn format_reset_after_seconds(seconds: u64, kind: QuotaKind) -> String {
    let target = SystemTime::now()
        .checked_add(Duration::from_secs(seconds))
        .unwrap_or_else(SystemTime::now);
    let Some(local_time) = local_datetime_from_system_time(target) else {
        return format!("{seconds} 秒后");
    };

    match kind {
        QuotaKind::FiveHour => format!("{}:{:02}", local_time.hour, local_time.minute),
        QuotaKind::Weekly => format!(
            "{}月{}日 {}:{:02}",
            local_time.month, local_time.day, local_time.hour, local_time.minute
        ),
    }
}

struct LocalDateTime {
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
}

#[cfg(windows)]
fn local_datetime_from_system_time(time: SystemTime) -> Option<LocalDateTime> {
    use windows_sys::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        Storage::FileSystem::FileTimeToLocalFileTime,
        System::Time::FileTimeToSystemTime,
    };

    const FILETIME_UNIX_EPOCH_OFFSET: u64 = 116_444_736_000_000_000;
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    let ticks = duration
        .as_secs()
        .checked_mul(10_000_000)?
        .checked_add((duration.subsec_nanos() / 100) as u64)?
        .checked_add(FILETIME_UNIX_EPOCH_OFFSET)?;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local_file_time = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut system_time = SYSTEMTIME::default();

    let local_ok = unsafe { FileTimeToLocalFileTime(&file_time, &mut local_file_time) };
    if local_ok == 0 {
        return None;
    }
    let system_ok = unsafe { FileTimeToSystemTime(&local_file_time, &mut system_time) };
    if system_ok == 0 {
        return None;
    }

    Some(LocalDateTime {
        month: system_time.wMonth,
        day: system_time.wDay,
        hour: system_time.wHour,
        minute: system_time.wMinute,
    })
}

#[cfg(not(windows))]
fn local_datetime_from_system_time(time: SystemTime) -> Option<LocalDateTime> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    let total_seconds = duration.as_secs();
    let days = (total_seconds / 86_400) as i64;
    let seconds_of_day = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let _ = year;

    Some(LocalDateTime {
        month,
        day,
        hour: (seconds_of_day / 3_600) as u16,
        minute: ((seconds_of_day % 3_600) / 60) as u16,
    })
}

#[cfg(not(windows))]
fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u16, u16) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_param = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_param + 2) / 5 + 1;
    let month = month_param + if month_param < 10 { 3 } else { -9 };
    year += (month <= 2) as i64;

    (year as i32, month as u16, day as u16)
}

fn normalize_percent_value(value: f64) -> Option<u8> {
    let percent = if (0.0..1.0).contains(&value) {
        value * 100.0
    } else {
        value
    };

    rounded_percent(percent)
}

fn percent_from_ratio(numerator: f64, denominator: f64) -> Option<u8> {
    if denominator <= 0.0 || numerator < 0.0 {
        return None;
    }

    rounded_percent((numerator / denominator) * 100.0)
}

fn rounded_percent(percent: f64) -> Option<u8> {
    if !(0.0..=100.0).contains(&percent) {
        return None;
    }

    Some(percent.round() as u8)
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_json_hint(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || is_cjk(*ch))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_cjk(ch: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

fn parse_percent(line: &str) -> Result<u8, QuotaParseError> {
    let Some(percent_at) = line.find('%') else {
        return Err(QuotaParseError::MissingPercent("unknown"));
    };
    let digits: String = line[..percent_at]
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    digits
        .parse::<u8>()
        .map_err(|_| QuotaParseError::InvalidPercent(line.to_owned()))
}

fn find_first_label(
    text_lower: &str,
    labels: &'static [&'static str],
) -> Option<(usize, &'static str)> {
    labels
        .iter()
        .filter_map(|label| text_lower.find(label).map(|position| (position, *label)))
        .min_by_key(|(position, _)| *position)
}

fn find_next_card_start(text_lower: &str, start_at: usize) -> Option<usize> {
    FIVE_HOUR_LABELS
        .iter()
        .chain(WEEKLY_LABELS.iter())
        .filter_map(|label| {
            text_lower[start_at..]
                .find(label)
                .map(|position| start_at + position)
        })
        .min()
}

fn normalize_analytics_text(text: &str) -> String {
    let decoded = decode_basic_html_entities(&decode_json_escapes(text));
    let mut normalized = String::with_capacity(decoded.len());
    let mut in_tag = false;

    for ch in decoded.chars() {
        match ch {
            '<' => {
                in_tag = true;
                push_separator(&mut normalized);
            }
            '>' if in_tag => {
                in_tag = false;
                push_separator(&mut normalized);
            }
            _ if in_tag => {}
            '"' | '\'' | '{' | '}' | '[' | ']' | ';' | '|' => {
                push_separator(&mut normalized);
            }
            _ if ch.is_whitespace() || ch.is_control() => {
                push_separator(&mut normalized);
            }
            _ => normalized.push(ch),
        }
    }

    normalized.trim().to_owned()
}

fn decode_basic_html_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn decode_json_escapes(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') | Some('r') | Some('t') => push_separator(&mut decoded),
            Some('u') => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    if let Some(hex_ch) = chars.next() {
                        hex.push(hex_ch);
                    }
                }

                if hex.len() == 4 && hex.chars().all(|hex_ch| hex_ch.is_ascii_hexdigit()) {
                    if let Ok(codepoint) = u32::from_str_radix(&hex, 16) {
                        if let Some(decoded_ch) = char::from_u32(codepoint) {
                            decoded.push(decoded_ch);
                            continue;
                        }
                    }
                    decoded.push('\\');
                    decoded.push('u');
                    decoded.push_str(&hex);
                } else {
                    decoded.push('\\');
                    decoded.push('u');
                    decoded.push_str(&hex);
                }
            }
            Some(other) => decoded.push(other),
            None => decoded.push('\\'),
        }
    }

    decoded
}

fn clean_reset_label(value: &str) -> String {
    let trimmed = value
        .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '：' | '-' | '–'));
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let Some(time_at) = tokens.iter().position(|token| looks_like_time_token(token)) else {
        return tokens.into_iter().take(6).collect::<Vec<_>>().join(" ");
    };
    let mut end = time_at + 1;
    if tokens
        .get(end)
        .is_some_and(|token| token.eq_ignore_ascii_case("am") || token.eq_ignore_ascii_case("pm"))
    {
        end += 1;
    }

    tokens[..end].join(" ")
}

fn push_separator(buffer: &mut String) {
    if !buffer.ends_with(' ') {
        buffer.push(' ');
    }
}

fn looks_like_time_token(token: &str) -> bool {
    let cleaned = token.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != ':');
    let Some((hours, minutes)) = cleaned.split_once(':') else {
        return false;
    };

    !hours.is_empty()
        && !minutes.is_empty()
        && hours.chars().all(|ch| ch.is_ascii_digit())
        && minutes.chars().take(2).all(|ch| ch.is_ascii_digit())
}
