//! App actions for the menubar: selected-app detection (CoreWindows +
//! X11), launching via `tapp`, Hide/Quit/Zoom/Close/ForceQuit.

use crate::CoreWindows::{self, WindowsProvider};
use crate::{daemon, x11};

/// Shell windows (menubar/dock) are never shown as the app name:
/// the bar keeps displaying the last real app instead.
pub fn is_shell_name(name: &str) -> bool {
  matches!(name.trim().to_lowercase().as_str(), "menubar" | "dock" | "")
}

/// Selected app as (display name, pid).
/// Name resolution via CoreWindows (real .app bundles), X11 WM_CLASS fallback.
/// The daemon has no focus concept, so the active window still comes from X11
/// and is bridged to CoreWindows via _NET_WM_PID.
pub fn selected_app() -> (String, Option<i32>) {
  let pid = x11::get_active_window().and_then(x11::get_window_pid);
  if let Some(pid) = pid {
    if let Some(windows) = daemon::cached_windows() {
      if let Some(w) = windows.iter().find(|w| w.pid == Some(pid)) {
        // Bundle names are reliable; plain X11 titles are not app names.
        if w.bundle_id.is_some() {
          if let Some(name) = w.app_name.clone().filter(|s| !s.trim().is_empty()) {
            return (name, Some(pid));
          }
        }
      }
    }
  }
  (x11::get_active_app_name(), pid)
}

/// Daemon window ids owned by `pid` (empty when the daemon is unreachable).
pub fn daemon_windows_of(pid: i32) -> Vec<u64> {
  daemon::cached_windows()
    .map(|ws| {
      ws.into_iter()
        .filter(|w| w.pid == Some(pid))
        .map(|w| w.id)
        .collect()
    })
    .unwrap_or_default()
}

/// Launch a .app bundle via the tapp runner, with optional extra arguments.
/// Tapp is always installed on the machine, so /usr/bin/tapp exists.
pub fn launch_via_tapp(bundle: &str, args: &[String]) -> bool {
  const TAPP: &str = "/usr/bin/tapp";
  if !std::path::Path::new(bundle).exists() {
    eprintln!("[menubar] bundle not found: {bundle}");
    return false;
  }
  let mut cmd = std::process::Command::new(TAPP);
  cmd.arg(bundle);
  for a in args {
    cmd.arg(a);
  }
  match cmd
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
  {
    Ok(_) => {
      println!("[menubar] launched {bundle} via tapp");
      true
    }
    Err(e) => {
      eprintln!("[menubar] tapp launch of {bundle} failed: {e}");
      false
    }
  }
}

/// Launch SystemOverview via the tapp runner.
pub fn launch_system_overview() {
  const SYSTEM_LINK: &str = "/Applications/SystemOverview.app";
  const BUNDLE: &str = "/System/Applications/systemoverview.app";
  if std::path::Path::new(SYSTEM_LINK).exists() {
    launch_via_tapp(SYSTEM_LINK, &[]);
    return;
  }
  if std::path::Path::new(BUNDLE).exists() {
    launch_via_tapp(BUNDLE, &[]);
    return;
  }
  let Ok(home) = std::env::var("HOME") else {
    eprintln!("[menubar] cannot launch SystemOverview: $HOME not set");
    return;
  };
  launch_via_tapp(&format!("{home}/Applications/SystemOverview.app"), &[]);
}

/// .app bundle path of the selected app, if it runs from a real bundle
/// (CoreWindows classification). Plain binaries have no bundle path.
pub fn selected_app_bundle() -> Option<String> {
  let pid = x11::get_active_window().and_then(x11::get_window_pid)?;
  let windows = daemon::cached_windows()?;
  let w = windows.iter().find(|w| w.pid == Some(pid))?;
  let raw = CoreWindows::RawWindow {
    id: w.id,
    app_id: w.app_id.clone(),
    title: w.title.clone(),
    pid: w.pid,
    minimized: w.minimized,
  };
  CoreWindows::classify(&raw)
    .bundle_dir
    .map(|p| p.to_string_lossy().to_string())
}

/// Open AboutThisApp for the selected app:
/// `tapp /System/Applications/AboutThisApp.app <bundle path>`.
/// Without a known bundle path, AboutThisApp still opens (no argument).
pub fn open_about_this_app() {
  const ABOUT: &str = "/System/Applications/AboutThisApp.app";
  let extra = selected_app_bundle().map(|b| vec![b]).unwrap_or_default();
  launch_via_tapp(ABOUT, &extra);
}

/// Hide the selected app (minimize its windows). X11 fallback when the
/// daemon is unreachable.
pub fn hide_current_app() {
  if let (_, Some(pid)) = selected_app() {
    let ids = daemon_windows_of(pid);
    if !ids.is_empty() {
      let provider = WindowsProvider::from_env();
      for id in ids {
        if let Err(e) = provider.minimize_window(id) {
          eprintln!("[menubar] minimize {id} failed: {e}");
        }
      }
      return;
    }
  }
  if let Some(xid) = x11::get_active_window() {
    x11::minimize_window(xid);
  }
}

/// Hide everything except the selected app. X11 fallback when unreachable.
pub fn hide_other_apps() {
  let active_pid = x11::get_active_window().and_then(x11::get_window_pid);
  let provider = WindowsProvider::from_env();
  if let Some(windows) = daemon::cached_windows() {
    if active_pid.is_some() || !windows.is_empty() {
      for w in &windows {
        if w.pid != active_pid {
          if let Err(e) = provider.minimize_window(w.id) {
            eprintln!("[menubar] minimize {} failed: {e}", w.id);
          }
        }
      }
      return;
    }
  }
  let active = x11::get_active_window();
  for xid in x11::get_client_list() {
    if Some(xid) != active {
      x11::minimize_window(xid);
    }
  }
}

/// Quit the selected app (graceful close of its windows). X11 fallback.
pub fn quit_current_app() {
  if let (_, Some(pid)) = selected_app() {
    let ids = daemon_windows_of(pid);
    if !ids.is_empty() {
      let provider = WindowsProvider::from_env();
      for id in ids {
        if let Err(e) = provider.close_window(id) {
          eprintln!("[menubar] close {id} failed: {e}");
        }
      }
      return;
    }
    if let Some(xid) = x11::get_active_window() {
      x11::close_window(xid);
    }
  }
}

/// Daemon id of the focused window: match active pid + title,
/// else first window of the active pid. None when unreachable.
pub fn focused_daemon_window() -> Option<u64> {
  let xid = x11::get_active_window()?;
  let pid = x11::get_window_pid(xid)?;
  let windows = daemon::cached_windows()?;
  let own: Vec<&CoreWindows::WindowInfo> =
    windows.iter().filter(|w| w.pid == Some(pid)).collect();
  if own.is_empty() {
    return None;
  }
  if let Some(title) = x11::get_window_title(xid) {
    if let Some(w) = own.iter().find(|w| w.title.as_deref() == Some(title.as_str())) {
      return Some(w.id);
    }
  }
  own.first().map(|w| w.id)
}

/// Minimize the focused window. X11 fallback when the daemon is unreachable.
pub fn minimize_focused_window() {
  if let Some(id) = focused_daemon_window() {
    if WindowsProvider::from_env().minimize_window(id).is_ok() {
      return;
    }
  }
  if let Some(xid) = x11::get_active_window() {
    x11::minimize_window(xid);
  }
}

/// Gracefully close the focused window. X11 fallback when unreachable.
pub fn close_focused_window() {
  if let Some(id) = focused_daemon_window() {
    if WindowsProvider::from_env().close_window(id).is_ok() {
      return;
    }
  }
  if let Some(xid) = x11::get_active_window() {
    x11::close_window(xid);
  }
}

/// Toggle fullscreen (Zoom) on the focused window.
/// The daemon has no fullscreen getter, so the caller tracks zoomed ids.
/// X11 fallback toggles via EWMH directly.
pub fn toggle_zoom_focused_window(zoomed: &mut std::collections::HashSet<u64>) {
  if let Some(id) = focused_daemon_window() {
    let provider = WindowsProvider::from_env();
    let is_zoomed = zoomed.contains(&id);
    if provider.set_fullscreen(id, !is_zoomed).is_ok() {
      if is_zoomed {
        zoomed.remove(&id);
      } else {
        zoomed.insert(id);
      }
      return;
    }
  }
  if let Some(xid) = x11::get_active_window() {
    x11::toggle_fullscreen(xid);
  }
}

/// Force quit the selected app (SIGKILL, no save dialog).
/// Never touches the menubar or dock themselves.
pub fn force_quit_current_app() {
  let Some(xid) = x11::get_active_window() else {
    return;
  };
  if let Some(cls) = x11::get_wm_class(xid) {
    let lower = cls.to_lowercase();
    if lower == "menubar" || lower == "dock" {
      return;
    }
  }
  if let Some(pid) = x11::get_window_pid(xid) {
    match CoreWindows::force_quit_pid(pid) {
      Ok(()) => println!("[menubar] force quit pid {pid}"),
      Err(e) => eprintln!("[menubar] force quit pid {pid} failed: {e}"),
    }
  }
}

/// Resolve the white octopus asset (CoreIcon) to an absolute path.
/// Candidates cover the dev layout, WSL mounts and the system install.
pub fn find_octopus_path() -> Option<String> {
  let candidates = [
    format!(
      "{}/../../TontooLibs/CoreIcon/assets/TontooOS/Tontoo_White.png",
      env!("CARGO_MANIFEST_DIR")
    ),
    format!(
      "{}/../../TontooLibs/CoreIcon/assets/TontooOS/tontoo_white.png",
      env!("CARGO_MANIFEST_DIR")
    ),
    "/mnt/c/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/TontooOS/Tontoo_White.png".to_string(),
    "/mnt/c/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/TontooOS/tontoo_white.png".to_string(),
    "/Library/System/CoreIcon/assets/TontooOS/Tontoo_White.png".to_string(),
    "assets/TontooOS/Tontoo_White.png".to_string(),
  ];
  for p in candidates {
    if std::path::Path::new(&p).exists() {
      return Some(p);
    }
  }
  None
}
