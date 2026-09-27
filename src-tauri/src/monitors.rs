//! Cached monitor list.
//!
//! Enumerating displays creates a DXGI factory and queries the display
//! configuration for every output, which takes the system's display lock and
//! can hitch a running game. Paths that run while playing (the once-a-second
//! game tracking, toasts, the post-save disk check) use this cache instead.
//! It is dropped when Windows reports a display change (`WM_DISPLAYCHANGE`,
//! see `power`) and when the capture monitor setting changes.

use geniusclip_engine::MonitorInfo;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static CACHE: Mutex<Option<Vec<MonitorInfo>>> = Mutex::new(None);
/// Bumped by every invalidation, so a list enumerated before a display change
/// isn't cached after it.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The monitors, enumerated only when the cache is empty.
pub fn list() -> Vec<MonitorInfo> {
    if let Some(m) = CACHE.lock().as_ref() {
        return m.clone();
    }
    refresh()
}

/// Enumerates the monitors now (for the settings page) and caches the result.
pub fn refresh() -> Vec<MonitorInfo> {
    // Enumerated without the lock: an invalidation from the display-change
    // handler must never wait for it.
    let generation = GENERATION.load(Ordering::SeqCst);
    match geniusclip_engine::list_monitors() {
        Ok(m) => {
            let mut cache = CACHE.lock();
            // An empty list is usually a display transition: ask again next time.
            if !m.is_empty() && GENERATION.load(Ordering::SeqCst) == generation {
                *cache = Some(m.clone());
            }
            m
        }
        Err(e) => {
            log::warn!("monitors: {e:#}");
            Vec::new()
        }
    }
}

/// Displays were added, removed or changed mode.
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    *CACHE.lock() = None;
}

/// Changes whenever the cached list may have changed.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// The configured monitor, else the primary one, else the first.
pub fn pick<'a>(mons: &'a [MonitorInfo], id: Option<&str>) -> Option<&'a MonitorInfo> {
    id.and_then(|id| mons.iter().find(|m| m.id == id)).or_else(|| mons.iter().find(|m| m.primary)).or(mons.first())
}
