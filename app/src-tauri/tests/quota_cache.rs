use app_lib::{
    quota::{QuotaEntry, QuotaKind, QuotaState},
    quota_cache::{load_quota_cache, save_quota_cache},
};
use std::fs;

#[test]
fn saves_and_loads_quota_cache_json() {
    let root = unique_temp_dir("quota-cache");
    let cache_path = root.join("quota-cache.json");
    let mut state = QuotaState::from_entries(vec![QuotaEntry {
        kind: QuotaKind::FiveHour,
        remaining_percent: 93,
        reset_label: "19:29".to_owned(),
    }]);
    state.last_refresh_at = Some("刚刚".to_owned());
    state.next_refresh_at = Some("5 分钟后".to_owned());

    save_quota_cache(&cache_path, &state).expect("expected cache save");
    let loaded = load_quota_cache(&cache_path).expect("expected cache load");

    assert_eq!(loaded, Some(state));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn load_quota_cache_returns_none_for_missing_file() {
    let root = unique_temp_dir("quota-cache-missing");
    let cache_path = root.join("quota-cache.json");

    assert_eq!(load_quota_cache(&cache_path).unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}

fn unique_temp_dir(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}
