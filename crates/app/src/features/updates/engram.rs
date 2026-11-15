//! "Is there a newer Engram?": its latest GitHub release, cached in the
//! app's `engram-update.json` (`checkedAt`, `latest`, and the version the
//! person chose to skip). A "newer" answer is trusted for six hours, a
//! "nothing newer" for one.

use std::path::{Path, PathBuf};

use gentle_core::version::{UpdateOffer, cached_answer_holds, offer_for};
use serde_json::{Value, json};

const TTL_MS: u64 = 6 * 60 * 60 * 1000;
const RECHECK_MS: u64 = 60 * 60 * 1000;

fn store() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("gentle-shell-gpui").join("engram-update.json"))
}

fn read(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn write(path: &Path, value: &Value) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, value.to_string());
}

fn offer(installed: &str, latest: &str, skipped: Option<&str>) -> Option<UpdateOffer> {
    offer_for(installed, latest, skipped).map(|offer| UpdateOffer {
        notes_url: format!(
            "https://github.com/Gentleman-Programming/engram/releases/tag/v{latest}"
        ),
        ..offer
    })
}

/// Blocking: the offer for the `installed` Engram, from the cache or
/// GitHub. Offline or unreadable answers mean "no offer".
pub fn check(installed: &str) -> Option<UpdateOffer> {
    let path = store()?;
    let mut saved = read(&path);
    let skipped = saved["skipped"].as_str().map(str::to_string);
    let now = super::now_ms();
    let cached = saved["latest"]
        .as_str()
        .zip(saved["checkedAt"].as_u64())
        .map(|(latest, at)| (latest.to_string(), now.saturating_sub(at)));
    if let Some((latest, age)) = &cached
        && cached_answer_holds(
            installed,
            latest,
            skipped.as_deref(),
            *age,
            TTL_MS,
            RECHECK_MS,
        )
    {
        return offer(installed, latest, skipped.as_deref());
    }
    let latest = match crate::features::setup::engram_latest() {
        Ok(latest) => {
            saved["latest"] = json!(latest);
            saved["checkedAt"] = json!(now);
            write(&path, &saved);
            latest
        }
        Err(_) => cached.filter(|(_, age)| *age < TTL_MS)?.0,
    };
    offer(installed, &latest, skipped.as_deref())
}

/// "Skip this version".
pub fn skip(version: &str) -> std::io::Result<()> {
    let path = store().ok_or_else(|| std::io::Error::other("no data folder"))?;
    let mut saved = read(&path);
    saved["skipped"] = json!(version);
    write(&path, &saved);
    Ok(())
}
