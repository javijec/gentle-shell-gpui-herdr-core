//! "Is there a newer Gentle Shell?", answered the way the launcher's own
//! interactive startup check answers it (`lib/gentle-shell-startup-update.ts`):
//! the npm registry's `gentle-pi/latest`, stable versions only, a one-day
//! cache and the "Skip this version" choice, both kept in the launcher's
//! `config.json` (`pi_rpc::launcher_config_path`) so the app and the
//! terminal agree. One difference: a cached "nothing newer" is asked again
//! after an hour, so a release published after the check shows the same
//! day (the fresh answer goes back into the shared cache).
//!
//! And "is there a newer Engram?" (`engram`), from its GitHub releases.
//!
//! Version parsing, comparison and the offer are `gentle_core::version`.
//! Here, `registry` asks npm and `cache` reads and writes the launcher config.

mod cache;
pub mod engram;
mod registry;

use std::time::{SystemTime, UNIX_EPOCH};

use gentle_core::version::{cached_answer_holds, offer_for};
use serde_json::Value;

pub use gentle_core::version::{UpdateOffer, current_from_versions};

/// How long a cached "nothing newer" holds before the registry is asked again.
const RECHECK_MS: u64 = 60 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Blocking: the offer for `current`, from the cache or the registry.
/// Offline or unreadable answers mean "no offer", never an error.
pub fn check(current: &str) -> Option<UpdateOffer> {
    let path = pi_rpc::launcher_config_path();
    let config = cache::read_config(&path);
    let skipped = config
        .get("skippedStartupRelease")
        .and_then(Value::as_str)
        .map(str::to_string);
    let now = now_ms();
    let cached = cache::cached_latest(&config, now);
    if let Some((latest, age)) = &cached
        && cached_answer_holds(
            current,
            latest,
            skipped.as_deref(),
            *age,
            cache::TTL_MS,
            RECHECK_MS,
        )
    {
        return offer_for(current, latest, skipped.as_deref());
    }
    let latest = match registry::fetch_latest() {
        Some(latest) => {
            // Like the launcher: only a real answer is worth a day's trust.
            let record =
                serde_json::json!({"checkedAt": cache::format_iso_ms(now), "latest": latest});
            let _ = cache::write_key(&path, "startupReleaseCheck", record);
            latest
        }
        // Offline: the day-old answer still beats none.
        None => cached?.0,
    };
    offer_for(current, &latest, skipped.as_deref())
}

/// "Skip this version": the key the terminal's startup prompt writes too.
pub fn skip(version: &str) -> std::io::Result<()> {
    let path = pi_rpc::launcher_config_path();
    cache::write_key(&path, "skippedStartupRelease", Value::from(version))
}
