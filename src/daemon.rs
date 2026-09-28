//! Window daemon polling (off the UI thread).
//!
//! The CoreWindows lib blocks up to 10s per call, so the daemon is polled
//! on a background thread into a cache. The bar only ever reads the cache
//! (fresh within 5s), never blocks on IPC.

use crate::CoreWindows::WindowsProvider;
use crate::CoreWindows::{WindowInfo, DEFAULT_SOCKET_PATH};

struct DaemonCache {
  windows: Vec<WindowInfo>,
  last_ok: Option<std::time::Instant>,
}

fn daemon_cache() -> &'static std::sync::Mutex<DaemonCache> {
  static CACHE: std::sync::OnceLock<std::sync::Mutex<DaemonCache>> = std::sync::OnceLock::new();
  CACHE.get_or_init(|| {
    std::sync::Mutex::new(DaemonCache {
      windows: Vec::new(),
      last_ok: None,
    })
  })
}

/// Fast reachability probe for the window daemon. Missing socket or refused
/// connection means `false` immediately.
pub fn daemon_reachable() -> bool {
  let path = std::env::var("WINDOWS_SOCKET")
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|_| std::path::PathBuf::from(DEFAULT_SOCKET_PATH));
  if !path.exists() {
    return false;
  }
  std::os::unix::net::UnixStream::connect(&path).is_ok()
}

/// Background daemon poller: a hung daemon must never freeze the bar.
/// Started once, polls every 800ms.
pub fn start_daemon_poller() {
  static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
  if STARTED.set(()).is_err() {
    return;
  }
  std::thread::Builder::new()
    .name("menubar-daemon-poll".to_string())
    .spawn(|| {
      let mut last_warn: Option<std::time::Instant> = None;
      loop {
        if daemon_reachable() {
          match WindowsProvider::from_env().windows() {
            Ok(ws) => {
              if let Ok(mut c) = daemon_cache().lock() {
                c.windows = ws;
                c.last_ok = Some(std::time::Instant::now());
              }
            }
            Err(e) => {
              let now = std::time::Instant::now();
              let due = last_warn
                .map(|t| now.duration_since(t) > std::time::Duration::from_secs(10))
                .unwrap_or(true);
              if due {
                eprintln!("[menubar] daemon poll failed: {e}");
                last_warn = Some(now);
              }
            }
          }
        }
        std::thread::sleep(std::time::Duration::from_millis(800));
      }
    })
    .expect("daemon poll thread");
}

/// Fresh daemon data (answered within the last 5s), or `None`.
/// Main-thread safe: pure cache read, never blocks on IPC.
pub fn cached_windows() -> Option<Vec<WindowInfo>> {
  let c = daemon_cache().lock().ok()?;
  match c.last_ok {
    Some(t) if t.elapsed() < std::time::Duration::from_secs(5) => Some(c.windows.clone()),
    _ => None,
  }
}
