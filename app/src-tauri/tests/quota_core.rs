use app_lib::quota::{
    apply_refresh_failure, next_retry_delay_minutes, parse_analytics_text, user_facing_parse_error,
    QuotaEntry, QuotaKind, QuotaParseError, QuotaSource, QuotaState, QuotaStatus,
};

#[test]
fn parses_chinese_analytics_cards() {
    let text = r#"
余额
Codex 使用量会从您共享的智能体使用限额中扣除
5 小时使用限额
93% 剩余
重置时间：19:29
每周使用限额
87% 剩余
重置时间：2026年5月27日 9:07
"#;

    let state = parse_analytics_text(text).expect("expected quota cards to parse");

    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.entries[0].reset_label, "19:29");
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert_eq!(state.entries[1].reset_label, "2026年5月27日 9:07");
}

#[test]
fn parses_english_analytics_cards() {
    let text = r#"
Balance
Codex usage counts against your shared agent usage limits.
5-hour usage limit
93% remaining
Reset time: 19:29
Weekly usage limit
87% remaining
Reset time: May 27, 2026 9:07 AM
"#;

    let state = parse_analytics_text(text).expect("expected English quota cards to parse");

    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.entries[0].reset_label, "19:29");
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert_eq!(state.entries[1].reset_label, "May 27, 2026 9:07 AM");
}

#[test]
fn parses_html_analytics_cards() {
    let text = r#"
<section><h2>5 小时使用限额</h2><strong>93% 剩余</strong><span>重置时间：19:29</span></section>
<section><h2>每周使用限额</h2><strong>87% 剩余</strong><span>重置时间：2026年5月27日 9:07</span></section>
"#;

    let state = parse_analytics_text(text).expect("expected HTML quota cards to parse");

    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.entries[0].reset_label, "19:29");
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert_eq!(state.entries[1].reset_label, "2026年5月27日 9:07");
}

#[test]
fn parses_wham_usage_json_cards() {
    let text = r#"
{
  "usage_limits": [
    {
      "window": "codex_5_hour",
      "remaining_percent": 0.93,
      "reset_at": "2026-05-20T19:29:00+08:00"
    },
    {
      "window": "codex_weekly",
      "used": 13,
      "limit": 100,
      "resets_at": "2026-05-27T09:07:00+08:00"
    }
  ]
}
"#;

    let state = parse_analytics_text(text).expect("expected wham usage JSON to parse");

    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.entries[0].reset_label, "2026-05-20T19:29:00+08:00");
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert_eq!(state.entries[1].reset_label, "2026-05-27T09:07:00+08:00");
}

#[test]
fn parses_nested_wham_usage_json_cards() {
    let text = r#"
{
  "codex": {
    "five_hour": {
      "used_percent": 7,
      "reset_time": "19:29"
    },
    "weekly": {
      "remaining": 87,
      "limit": 100,
      "reset_time": "2026年5月27日 9:07"
    }
  }
}
"#;

    let state = parse_analytics_text(text).expect("expected nested wham usage JSON to parse");

    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.entries[0].reset_label, "19:29");
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert_eq!(state.entries[1].reset_label, "2026年5月27日 9:07");
}

#[test]
fn parses_current_wham_usage_rate_limit_windows() {
    let text = r#"
{
  "account_id": "acct",
  "additional_rate_limits": [],
  "code_review_rate_limit": null,
  "credits": null,
  "email": "hidden@example.com",
  "plan_type": "pro",
  "promo": null,
  "rate_limit": {
    "primary_window": {
      "used": 7,
      "limit": 100,
      "used_percent": 7,
      "reset_after_seconds": 5400
    },
    "secondary_window": {
      "used": 13,
      "limit": 100,
      "remaining_percent": 87,
      "reset_after_seconds": 604800
    }
  }
}
"#;

    let state = parse_analytics_text(text).expect("expected current wham usage JSON to parse");

    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 93);
    assert!(state.entries[0].reset_label.contains(':'));
    assert!(!state.entries[0].reset_label.contains('后'));
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 87);
    assert!(state.entries[1].reset_label.contains('月'));
    assert!(state.entries[1].reset_label.contains('日'));
    assert!(state.entries[1].reset_label.contains(':'));
    assert!(!state.entries[1].reset_label.contains('后'));
}

#[test]
fn ignores_reset_countdown_when_calculating_wham_remaining_percent() {
    let text = r#"
{
  "rate_limit": {
    "primary_window": {
      "used": 1,
      "limit": 100,
      "remaining_seconds": 0,
      "reset_after_seconds": 5400
    },
    "secondary_window": {
      "remaining_percent": 87,
      "reset_after_seconds": 604800
    }
  }
}
"#;

    let state = parse_analytics_text(text).expect("expected wham usage JSON to parse");

    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 99);
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 87);
}

#[test]
fn treats_one_used_percent_as_one_percent_not_fractional_full_usage() {
    let text = r#"
{
  "rate_limit": {
    "primary_window": {
      "used_percent": 1,
      "reset_after_seconds": 17067
    },
    "secondary_window": {
      "used_percent": 1,
      "reset_after_seconds": 553953
    }
  }
}
"#;

    let state = parse_analytics_text(text).expect("expected wham usage JSON to parse");

    assert_eq!(state.entries[0].kind, QuotaKind::FiveHour);
    assert_eq!(state.entries[0].remaining_percent, 99);
    assert_eq!(state.entries[1].kind, QuotaKind::Weekly);
    assert_eq!(state.entries[1].remaining_percent, 99);
}

#[test]
fn retry_delay_caps_after_repeated_failures() {
    assert_eq!(next_retry_delay_minutes(0), 5);
    assert_eq!(next_retry_delay_minutes(1), 10);
    assert_eq!(next_retry_delay_minutes(2), 20);
    assert_eq!(next_retry_delay_minutes(3), 30);
    assert_eq!(next_retry_delay_minutes(9), 30);
}

#[test]
fn refresh_failure_preserves_cached_entries() {
    let cached = QuotaState::from_entries(vec![
        QuotaEntry {
            kind: QuotaKind::FiveHour,
            remaining_percent: 93,
            reset_label: "19:29".to_owned(),
        },
        QuotaEntry {
            kind: QuotaKind::Weekly,
            remaining_percent: 87,
            reset_label: "2026年5月27日 9:07".to_owned(),
        },
    ]);

    let state = apply_refresh_failure(
        Some(cached),
        QuotaStatus::LoginRequired,
        "Please sign in to ChatGPT in Edge.",
    );

    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.entries[0].remaining_percent, 93);
    assert_eq!(state.source, QuotaSource::Cache);
    assert_eq!(state.status, QuotaStatus::LoginRequired);
    assert_eq!(
        state.error_summary.as_deref(),
        Some("Please sign in to ChatGPT in Edge.")
    );
}

#[test]
fn parse_error_message_explains_missing_quota_cards() {
    let message = user_facing_parse_error(&QuotaParseError::MissingCard("5-hour"));

    assert!(message.contains("usage 接口"));
    assert!(message.contains("5 小时额度"));
    assert!(message.contains("Cookie Header"));
}
