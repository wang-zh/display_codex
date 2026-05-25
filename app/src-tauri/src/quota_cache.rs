use std::{fs, io, path::Path};

use crate::quota::QuotaState;

pub fn load_quota_cache(path: &Path) -> io::Result<Option<QuotaState>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)?;
    let state = serde_json::from_str(&json)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(state))
}

pub fn save_quota_cache(path: &Path, state: &QuotaState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, json)
}
