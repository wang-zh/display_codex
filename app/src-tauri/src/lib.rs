pub mod analytics;
pub mod app_log;
pub mod edge_credentials;
pub mod network_diagnostics;
pub mod quota;
pub mod quota_cache;

use std::{path::PathBuf, sync::Mutex};

use analytics::{fetch_analytics_page, AnalyticsError};
use app_log::append_log_line;
use edge_credentials::{load_edge_cookie_header, user_facing_credential_error};
use network_diagnostics::{collect_connection_diagnostics, ConnectionDiagnostics};
use quota::{
    apply_refresh_failure, parse_analytics_text, user_facing_parse_error, QuotaSource, QuotaState,
    QuotaStatus,
};
use quota_cache::{load_quota_cache, save_quota_cache};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ICON_ID: &str = "codex-quota";
const TRAY_TOOLTIP_MAX_CHARS: usize = 120;
const TRAY_ICON_SIZE: usize = 32;
const TRAY_ICON_SIZE_U32: u32 = TRAY_ICON_SIZE as u32;
const TRAY_ICON_CHANNELS: usize = 4;
const DIGIT_GLYPHS: [[&str; 5]; 10] = [
    ["111", "101", "101", "101", "111"],
    ["010", "110", "010", "010", "111"],
    ["111", "001", "111", "100", "111"],
    ["111", "001", "111", "001", "111"],
    ["101", "101", "111", "001", "001"],
    ["111", "100", "111", "001", "111"],
    ["111", "100", "111", "101", "111"],
    ["111", "001", "001", "001", "001"],
    ["111", "101", "111", "101", "111"],
    ["111", "101", "111", "001", "111"],
];
const DASH_GLYPH: [&str; 5] = ["000", "000", "111", "000", "000"];

#[derive(Default)]
struct QuotaRuntime {
    cached: Option<QuotaState>,
    failure_count: u32,
    cache_path: Option<PathBuf>,
    log_path: Option<PathBuf>,
    cookie_header_override: Option<String>,
}

#[tauri::command]
fn get_quota_state(state: tauri::State<'_, Mutex<QuotaRuntime>>) -> QuotaState {
    state
        .lock()
        .expect("quota runtime lock poisoned")
        .cached
        .clone()
        .unwrap_or_else(QuotaState::idle)
}

#[tauri::command]
fn get_log_file_path(state: tauri::State<'_, Mutex<QuotaRuntime>>) -> Option<String> {
    state
        .lock()
        .ok()
        .and_then(|runtime| runtime_log_file_path(&runtime))
}

#[tauri::command]
fn get_connection_diagnostics() -> ConnectionDiagnostics {
    collect_connection_diagnostics()
}

#[tauri::command]
fn refresh_quota(
    app: tauri::AppHandle,
    state: tauri::State<'_, Mutex<QuotaRuntime>>,
) -> QuotaState {
    let (cookie_header_override, log_path) = {
        let runtime = state.lock().expect("quota runtime lock poisoned");
        (
            runtime.cookie_header_override.clone(),
            runtime.log_path.clone(),
        )
    };

    if let Some(sample) = read_development_sample() {
        log_runtime_event(
            log_path.as_ref(),
            "refresh_start",
            "source=development_sample",
        );
        let quota_state = finish_refresh(state, parse_quota_text(&sample, None));
        log_runtime_event(
            log_path.as_ref(),
            "refresh_done",
            format_refresh_result(&quota_state),
        );
        update_tray_presentation(&app, &quota_state);
        return quota_state;
    }

    log_runtime_event(log_path.as_ref(), "refresh_start", "source=live");
    let credential_result = match cookie_header_override {
        Some(cookie_header) => {
            log_runtime_event(
                log_path.as_ref(),
                "credential_ready",
                "source=temporary_override",
            );
            Ok(cookie_header)
        }
        None => match load_edge_cookie_header() {
            Ok(cookie_header) => {
                log_runtime_event(log_path.as_ref(), "credential_ready", "source=edge");
                Ok(cookie_header)
            }
            Err(error) => {
                let summary = user_facing_credential_error(&error);
                log_runtime_event(
                    log_path.as_ref(),
                    "credential_error",
                    format!("source=edge error={summary}"),
                );
                Err((QuotaStatus::LoginRequired, summary))
            }
        },
    };

    let result =
        credential_result.and_then(|cookie_header| match fetch_analytics_page(&cookie_header) {
            Ok(page) => {
                log_runtime_event(
                    log_path.as_ref(),
                    "analytics_fetch_ok",
                    &page.diagnostic_summary,
                );
                parse_quota_text(&page.body, Some(page.diagnostic_summary))
            }
            Err(error) => {
                let (status, message) = match error {
                    AnalyticsError::LoginRequired(message) => (QuotaStatus::LoginRequired, message),
                    AnalyticsError::Network(message) => (QuotaStatus::NetworkError, message),
                };
                log_runtime_event(
                    log_path.as_ref(),
                    "analytics_fetch_error",
                    format!("status={status:?} error={message}"),
                );
                Err((status, message))
            }
        });

    let quota_state = finish_refresh(state, result);
    log_runtime_event(
        log_path.as_ref(),
        "refresh_done",
        format_refresh_result(&quota_state),
    );
    update_tray_presentation(&app, &quota_state);
    quota_state
}

#[tauri::command]
fn set_cookie_header_override(
    value: String,
    state: tauri::State<'_, Mutex<QuotaRuntime>>,
) -> QuotaState {
    let mut runtime = state.lock().expect("quota runtime lock poisoned");
    let trimmed = value.trim();
    runtime.cookie_header_override = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    log_runtime_event(
        runtime.log_path.as_ref(),
        if trimmed.is_empty() {
            "cookie_override_cleared"
        } else {
            "cookie_override_set"
        },
        "value_redacted=true",
    );
    runtime.cached.clone().unwrap_or_else(QuotaState::idle)
}

fn parse_quota_text(
    text: &str,
    diagnostic_summary: Option<String>,
) -> Result<QuotaState, (QuotaStatus, String)> {
    parse_analytics_text(text)
        .map(|mut quota_state| {
            quota_state.last_refresh_at = Some("刚刚".to_owned());
            quota_state.next_refresh_at = Some("5 分钟后".to_owned());
            quota_state
        })
        .map_err(|error| {
            let mut summary = user_facing_parse_error(&error);
            if let Some(diagnostics) = diagnostic_summary {
                summary.push_str(" 响应诊断：");
                summary.push_str(&diagnostics);
            }
            (QuotaStatus::ParseError, summary)
        })
}

fn finish_refresh(
    state: tauri::State<'_, Mutex<QuotaRuntime>>,
    result: Result<QuotaState, (QuotaStatus, String)>,
) -> QuotaState {
    let mut runtime = state.lock().expect("quota runtime lock poisoned");
    match result {
        Ok(quota_state) => {
            runtime.failure_count = 0;
            runtime.cached = Some(quota_state.clone());
            persist_runtime_cache(&runtime, &quota_state);
            quota_state
        }
        Err((status, summary)) => {
            runtime.failure_count += 1;
            let failed = apply_refresh_failure(runtime.cached.clone(), status, summary);
            runtime.cached = Some(failed.clone());
            failed
        }
    }
}

fn persist_runtime_cache(runtime: &QuotaRuntime, state: &QuotaState) {
    if !should_persist_quota_cache(state) {
        return;
    }

    if let Some(cache_path) = &runtime.cache_path {
        let _ = save_quota_cache(cache_path, state);
    }
}

fn should_persist_quota_cache(state: &QuotaState) -> bool {
    state.source == QuotaSource::Live
        && state.status == QuotaStatus::Ok
        && !state.entries.is_empty()
}

fn restore_cached_quota_state(mut state: QuotaState) -> QuotaState {
    state.source = QuotaSource::Cache;
    state.error_summary = None;
    state.next_refresh_at = None;
    state.status = if state.entries.is_empty() {
        QuotaStatus::Idle
    } else {
        QuotaStatus::Stale
    };
    state
}

fn runtime_log_file_path(runtime: &QuotaRuntime) -> Option<String> {
    runtime
        .log_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
}

fn log_runtime_event(log_path: Option<&PathBuf>, event: &str, message: impl AsRef<str>) {
    if let Some(log_path) = log_path {
        let _ = append_log_line(log_path, event, message);
    }
}

fn format_refresh_result(state: &QuotaState) -> String {
    let quota_values = state
        .entries
        .iter()
        .map(|entry| {
            format!(
                "{}={}",
                match entry.kind {
                    quota::QuotaKind::FiveHour => "five_hour",
                    quota::QuotaKind::Weekly => "weekly",
                },
                entry.remaining_percent
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "status={:?} source={:?} entries={}{}{}",
        state.status,
        state.source,
        state.entries.len(),
        if quota_values.is_empty() { "" } else { " " },
        quota_values
    )
}

fn read_development_sample() -> Option<String> {
    std::env::var("CODEX_QUOTA_ANALYTICS_TEXT")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn toggle_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn should_hide_instead_of_close(window_label: &str) -> bool {
    window_label == MAIN_WINDOW_LABEL
}

fn update_tray_presentation(app: &tauri::AppHandle, state: &QuotaState) {
    if let Some(tray) = app.tray_by_id(TRAY_ICON_ID) {
        let _ = tray.set_tooltip(Some(format_tray_tooltip(state)));
        let _ = tray.set_icon(Some(render_quota_tray_icon(five_hour_remaining_percent(
            state,
        ))));
    }
}

fn five_hour_remaining_percent(state: &QuotaState) -> Option<u8> {
    state
        .entries
        .iter()
        .find(|entry| entry.kind == quota::QuotaKind::FiveHour)
        .map(|entry| entry.remaining_percent)
}

fn render_quota_tray_icon(percent: Option<u8>) -> Image<'static> {
    Image::new_owned(
        render_tray_icon_rgba(percent),
        TRAY_ICON_SIZE_U32,
        TRAY_ICON_SIZE_U32,
    )
}

fn render_tray_icon_rgba(percent: Option<u8>) -> Vec<u8> {
    let mut rgba = vec![0; TRAY_ICON_SIZE * TRAY_ICON_SIZE * TRAY_ICON_CHANNELS];
    let background = tray_icon_background(percent);
    let radius = 15_i32;
    let center = 15_i32;

    for y in 0..TRAY_ICON_SIZE {
        for x in 0..TRAY_ICON_SIZE {
            let dx = x as i32 - center;
            let dy = y as i32 - center;
            if dx * dx + dy * dy <= radius * radius {
                set_tray_icon_pixel(&mut rgba, x, y, background);
            }
        }
    }

    draw_tray_icon_label(
        &mut rgba,
        &tray_icon_text_for_percent(percent),
        (255, 255, 255, 255),
    );
    rgba
}

fn tray_icon_text_for_percent(percent: Option<u8>) -> String {
    percent
        .map(|value| value.min(100).to_string())
        .unwrap_or_else(|| "--".to_owned())
}

fn tray_icon_background(percent: Option<u8>) -> (u8, u8, u8, u8) {
    match percent {
        Some(value) if value >= 50 => (34, 197, 94, 255),
        Some(value) if value >= 20 => (245, 158, 11, 255),
        Some(_) => (239, 68, 68, 255),
        None => (100, 116, 139, 255),
    }
}

fn draw_tray_icon_label(rgba: &mut [u8], label: &str, color: (u8, u8, u8, u8)) {
    let glyph_count = label.chars().count();
    if glyph_count == 0 {
        return;
    }

    let scale = match glyph_count {
        1 => 5,
        2 => 4,
        _ => 3,
    };
    let gap = if glyph_count >= 3 { 1 } else { 2 };
    let glyph_width = 3 * scale;
    let total_width = glyph_count * glyph_width + glyph_count.saturating_sub(1) * gap;
    let total_height = 5 * scale;
    let start_x = TRAY_ICON_SIZE.saturating_sub(total_width) / 2;
    let start_y = TRAY_ICON_SIZE.saturating_sub(total_height) / 2;

    for (index, character) in label.chars().enumerate() {
        let Some(glyph) = tray_icon_glyph(character) else {
            continue;
        };
        let glyph_x = start_x + index * (glyph_width + gap);
        draw_tray_icon_glyph(rgba, glyph, glyph_x, start_y, scale, color);
    }
}

fn tray_icon_glyph(character: char) -> Option<&'static [&'static str; 5]> {
    match character {
        '0' => Some(&DIGIT_GLYPHS[0]),
        '1' => Some(&DIGIT_GLYPHS[1]),
        '2' => Some(&DIGIT_GLYPHS[2]),
        '3' => Some(&DIGIT_GLYPHS[3]),
        '4' => Some(&DIGIT_GLYPHS[4]),
        '5' => Some(&DIGIT_GLYPHS[5]),
        '6' => Some(&DIGIT_GLYPHS[6]),
        '7' => Some(&DIGIT_GLYPHS[7]),
        '8' => Some(&DIGIT_GLYPHS[8]),
        '9' => Some(&DIGIT_GLYPHS[9]),
        '-' => Some(&DASH_GLYPH),
        _ => None,
    }
}

fn draw_tray_icon_glyph(
    rgba: &mut [u8],
    glyph: &[&str; 5],
    start_x: usize,
    start_y: usize,
    scale: usize,
    color: (u8, u8, u8, u8),
) {
    for (row_index, row) in glyph.iter().enumerate() {
        for (column_index, bit) in row.as_bytes().iter().enumerate() {
            if *bit != b'1' {
                continue;
            }

            for y_offset in 0..scale {
                for x_offset in 0..scale {
                    set_tray_icon_pixel(
                        rgba,
                        start_x + column_index * scale + x_offset,
                        start_y + row_index * scale + y_offset,
                        color,
                    );
                }
            }
        }
    }
}

fn set_tray_icon_pixel(rgba: &mut [u8], x: usize, y: usize, color: (u8, u8, u8, u8)) {
    if x >= TRAY_ICON_SIZE || y >= TRAY_ICON_SIZE {
        return;
    }

    let offset = (y * TRAY_ICON_SIZE + x) * TRAY_ICON_CHANNELS;
    rgba[offset] = color.0;
    rgba[offset + 1] = color.1;
    rgba[offset + 2] = color.2;
    rgba[offset + 3] = color.3;
}

fn format_tray_tooltip(state: &QuotaState) -> String {
    let mut lines = vec!["Codex 额度".to_owned()];

    if state.entries.is_empty() {
        lines.push(tray_status_label(state.status).to_owned());
        if let Some(summary) = state
            .error_summary
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            lines.push(summary.to_owned());
        }
    } else {
        lines.extend(state.entries.iter().map(|entry| {
            format!(
                "{}：{}% 剩余，重置 {}",
                tray_entry_label(entry.kind),
                entry.remaining_percent,
                entry.reset_label
            )
        }));
        if state.status != QuotaStatus::Ok {
            lines.push(format!("状态：{}", tray_status_label(state.status)));
        }
    }

    truncate_tray_tooltip(&lines.join("\n"))
}

fn tray_entry_label(kind: quota::QuotaKind) -> &'static str {
    match kind {
        quota::QuotaKind::FiveHour => "5小时",
        quota::QuotaKind::Weekly => "每周",
    }
}

fn tray_status_label(status: QuotaStatus) -> &'static str {
    match status {
        QuotaStatus::Idle => "等待刷新",
        QuotaStatus::Refreshing => "正在刷新",
        QuotaStatus::Ok => "数据已更新",
        QuotaStatus::Stale => "使用上次成功数据",
        QuotaStatus::LoginRequired => "需要登录态",
        QuotaStatus::ParseError => "解析失败",
        QuotaStatus::NetworkError => "网络失败",
    }
}

fn truncate_tray_tooltip(value: &str) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(TRAY_TOOLTIP_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(QuotaRuntime::default()))
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            if should_hide_instead_of_close(window.label()) {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let mut initial_tray_state = QuotaState::idle();
            if let Ok(cache_dir) = app.path().app_local_data_dir() {
                let cache_path = cache_dir.join("quota-cache.json");
                let log_path = cache_dir.join("codex-quota.log");
                let cached = load_quota_cache(&cache_path)
                    .ok()
                    .flatten()
                    .map(restore_cached_quota_state);
                initial_tray_state = cached.clone().unwrap_or_else(QuotaState::idle);
                let runtime = app.state::<Mutex<QuotaRuntime>>();
                if let Ok(mut runtime) = runtime.lock() {
                    runtime.cached = cached;
                    runtime.cache_path = Some(cache_path);
                    runtime.log_path = Some(log_path.clone());
                };
                log_runtime_event(
                    Some(&log_path),
                    "app_start",
                    format_refresh_result(&initial_tray_state),
                );
            }

            let toggle_i = MenuItem::with_id(app, "toggle", "打开/隐藏详情", true, None::<&str>)?;
            let refresh_i = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle_i, &refresh_i, &settings_i, &quit_i])?;

            TrayIconBuilder::with_id(TRAY_ICON_ID)
                .icon(render_quota_tray_icon(five_hour_remaining_percent(
                    &initial_tray_state,
                )))
                .tooltip(format_tray_tooltip(&initial_tray_state))
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_main_window(app),
                    "refresh" => {
                        if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                            let _ = window.emit("quota-refresh-requested", ());
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "settings" => {
                        if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                            let _ = window.emit("quota-settings-requested", ());
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_quota_state,
            get_log_file_path,
            get_connection_diagnostics,
            refresh_quota,
            set_cookie_header_override
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_main_window_closes_to_tray() {
        assert!(should_hide_instead_of_close("main"));
        assert!(!should_hide_instead_of_close("settings"));
    }

    #[test]
    fn tray_tooltip_summarizes_available_quota() {
        let state = QuotaState::from_entries(vec![
            quota::QuotaEntry {
                kind: quota::QuotaKind::FiveHour,
                remaining_percent: 93,
                reset_label: "19:29".to_owned(),
            },
            quota::QuotaEntry {
                kind: quota::QuotaKind::Weekly,
                remaining_percent: 87,
                reset_label: "5月27日 9:07".to_owned(),
            },
        ]);

        assert_eq!(
            format_tray_tooltip(&state),
            "Codex 额度\n5小时：93% 剩余，重置 19:29\n每周：87% 剩余，重置 5月27日 9:07"
        );
    }

    #[test]
    fn five_hour_percent_drives_tray_icon() {
        let state = QuotaState::from_entries(vec![
            quota::QuotaEntry {
                kind: quota::QuotaKind::Weekly,
                remaining_percent: 87,
                reset_label: "5月27日 9:07".to_owned(),
            },
            quota::QuotaEntry {
                kind: quota::QuotaKind::FiveHour,
                remaining_percent: 99,
                reset_label: "19:29".to_owned(),
            },
        ]);

        assert_eq!(five_hour_remaining_percent(&state), Some(99));
    }

    #[test]
    fn refresh_result_log_includes_quota_values() {
        let state = QuotaState::from_entries(vec![
            quota::QuotaEntry {
                kind: quota::QuotaKind::FiveHour,
                remaining_percent: 98,
                reset_label: "14:30".to_owned(),
            },
            quota::QuotaEntry {
                kind: quota::QuotaKind::Weekly,
                remaining_percent: 62,
                reset_label: "5月27日 9:07".to_owned(),
            },
        ]);

        assert_eq!(
            format_refresh_result(&state),
            "status=Ok source=Live entries=2 five_hour=98 weekly=62"
        );
    }

    #[test]
    fn tray_icon_text_supports_empty_and_three_digit_percent() {
        assert_eq!(tray_icon_text_for_percent(None), "--");
        assert_eq!(tray_icon_text_for_percent(Some(99)), "99");
        assert_eq!(tray_icon_text_for_percent(Some(100)), "100");
        assert_eq!(tray_icon_text_for_percent(Some(255)), "100");
    }

    #[test]
    fn tray_icon_pixels_change_with_five_hour_percent() {
        let full = render_tray_icon_rgba(Some(99));
        let low = render_tray_icon_rgba(Some(10));
        let empty = render_tray_icon_rgba(None);

        assert_eq!(full.len(), 32 * 32 * 4);
        assert_ne!(full, low);
        assert_ne!(low, empty);
        assert!(full.chunks_exact(4).any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn cached_failure_without_entries_restarts_as_idle_cache() {
        let cached = QuotaState {
            entries: Vec::new(),
            last_refresh_at: None,
            next_refresh_at: Some("5 分钟后".to_owned()),
            source: quota::QuotaSource::Cache,
            status: QuotaStatus::NetworkError,
            error_summary: Some("error sending request for url".to_owned()),
        };

        let restored = restore_cached_quota_state(cached);

        assert_eq!(restored.source, quota::QuotaSource::Cache);
        assert_eq!(restored.status, QuotaStatus::Idle);
        assert_eq!(restored.error_summary, None);
        assert_eq!(restored.next_refresh_at, None);
    }

    #[test]
    fn cached_entries_restart_as_stale_without_old_error() {
        let cached = QuotaState {
            entries: vec![quota::QuotaEntry {
                kind: quota::QuotaKind::FiveHour,
                remaining_percent: 88,
                reset_label: "19:29".to_owned(),
            }],
            last_refresh_at: Some("刚刚".to_owned()),
            next_refresh_at: Some("5 分钟后".to_owned()),
            source: quota::QuotaSource::Live,
            status: QuotaStatus::NetworkError,
            error_summary: Some("temporary network error".to_owned()),
        };

        let restored = restore_cached_quota_state(cached);

        assert_eq!(restored.source, quota::QuotaSource::Cache);
        assert_eq!(restored.status, QuotaStatus::Stale);
        assert_eq!(restored.error_summary, None);
        assert_eq!(restored.entries[0].remaining_percent, 88);
    }

    #[test]
    fn only_live_ok_quota_is_persisted_to_restart_cache() {
        let live = QuotaState::from_entries(vec![quota::QuotaEntry {
            kind: quota::QuotaKind::FiveHour,
            remaining_percent: 88,
            reset_label: "19:29".to_owned(),
        }]);
        let failed = apply_refresh_failure(
            Some(live.clone()),
            QuotaStatus::NetworkError,
            "temporary network error",
        );

        assert!(should_persist_quota_cache(&live));
        assert!(!should_persist_quota_cache(&failed));
    }

    #[test]
    fn runtime_log_file_path_is_exposed_without_logging_contents() {
        let runtime = QuotaRuntime {
            log_path: Some(PathBuf::from(r"C:\Users\TR\AppData\Local\codex-quota.log")),
            ..QuotaRuntime::default()
        };

        assert_eq!(
            runtime_log_file_path(&runtime),
            Some(r"C:\Users\TR\AppData\Local\codex-quota.log".to_owned())
        );
    }
}
