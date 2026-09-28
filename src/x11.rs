//! X11 EWMH helpers: active window queries and window actions.
//!
//! Pure `x11rb`, no GTK. Used for the selected-app detection and the
//! Hide/Quit/Zoom/Close fallbacks. Returns `None`/`false` without an X
//! server; callers fall back to the CoreWindows daemon data.

#[cfg(target_os = "linux")]
mod imp {
  use std::sync::OnceLock;
  use x11rb::connection::Connection;
  use x11rb::protocol::xproto::ConnectionExt;
  type Conn = x11rb::rust_connection::RustConnection;

  fn connection() -> Option<&'static (Conn, usize)> {
    static CONN: OnceLock<Option<(Conn, usize)>> = OnceLock::new();
    CONN.get_or_init(|| x11rb::connect(None).ok()).as_ref()
  }

  pub fn get_active_window() -> Option<u32> {
    let (conn, screen) = connection()?;
    let root = conn.setup().roots[*screen].root;
    let atom = conn
      .intern_atom(false, b"_NET_ACTIVE_WINDOW")
      .ok()?
      .reply()
      .ok()?
      .atom;
    let prop = conn
      .get_property(false, root, atom, x11rb::protocol::xproto::AtomEnum::WINDOW, 0, 1)
      .ok()?
      .reply()
      .ok()?;
    if prop.value.len() >= 4 {
      let xid = u32::from_ne_bytes([prop.value[0], prop.value[1], prop.value[2], prop.value[3]]);
      if xid != 0 {
        return Some(xid);
      }
    }
    None
  }

  pub fn get_window_pid(xid: u32) -> Option<i32> {
    let (conn, _) = connection()?;
    let atom = conn.intern_atom(false, b"_NET_WM_PID").ok()?.reply().ok()?.atom;
    let prop = conn
      .get_property(
        false,
        xid,
        atom,
        x11rb::protocol::xproto::AtomEnum::CARDINAL,
        0,
        1,
      )
      .ok()?
      .reply()
      .ok()?;
    if prop.value.len() >= 4 {
      Some(i32::from_ne_bytes([
        prop.value[0],
        prop.value[1],
        prop.value[2],
        prop.value[3],
      ]))
    } else {
      None
    }
  }

  pub fn get_client_list() -> Vec<u32> {
    let Some((conn, screen)) = connection() else {
      return Vec::new();
    };
    let root = conn.setup().roots[*screen].root;
    let atom = match conn
      .intern_atom(false, b"_NET_CLIENT_LIST")
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(r) => r.atom,
      None => return Vec::new(),
    };
    let prop = match conn
      .get_property(false, root, atom, x11rb::protocol::xproto::AtomEnum::WINDOW, 0, 4096)
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(p) => p,
      None => return Vec::new(),
    };
    if prop.value.is_empty() {
      return Vec::new();
    }
    prop.value
      .chunks_exact(4)
      .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
      .filter(|&x| x != 0)
      .collect()
  }

  pub fn get_wm_class(xid: u32) -> Option<String> {
    let (conn, _) = connection()?;
    let atom = conn.intern_atom(false, b"WM_CLASS").ok()?.reply().ok()?.atom;
    let prop = conn
      .get_property(false, xid, atom, x11rb::protocol::xproto::AtomEnum::STRING, 0, 1024)
      .ok()?
      .reply()
      .ok()?;
    if prop.format != 8 || prop.value.is_empty() {
      return None;
    }
    let parts: Vec<&[u8]> = prop.value.split(|&b| b == 0).collect();
    // WM_CLASS = instance + '\0' + class + '\0'
    let class = parts
      .get(1)
      .and_then(|c| if c.is_empty() { None } else { Some(*c) })
      .or_else(|| parts.first().copied())
      .map(|c| String::from_utf8_lossy(c).trim().to_string())
      .filter(|s| !s.is_empty())?;
    Some(class)
  }

  pub fn get_active_app_name() -> String {
    if let Some(xid) = get_active_window() {
      if let Some(cls) = get_wm_class(xid) {
        let mut chars = cls.chars();
        if let Some(first) = chars.next() {
          let rest: String = chars.collect();
          let cap = first.to_uppercase().collect::<String>() + &rest;
          if cap.to_lowercase() != "menubar" && cap.to_lowercase() != "dock" {
            return cap;
          }
          return "Finder".to_string();
        }
        return cls;
      }
      if let Some(name) = get_net_wm_name(xid) {
        return name;
      }
    }
    "Finder".to_string()
  }

  fn get_net_wm_name(xid: u32) -> Option<String> {
    let (conn, _) = connection()?;
    let atom = conn.intern_atom(false, b"_NET_WM_NAME").ok()?.reply().ok()?.atom;
    let utf8 = conn.intern_atom(false, b"UTF8_STRING").ok()?.reply().ok()?.atom;
    let prop = conn.get_property(false, xid, atom, utf8, 0, 1024).ok()?.reply().ok()?;
    if prop.value.is_empty() {
      return None;
    }
    let s = String::from_utf8_lossy(&prop.value).trim().to_string();
    if s.is_empty() {
      None
    } else {
      Some(s)
    }
  }

  pub fn get_window_title(xid: u32) -> Option<String> {
    get_net_wm_name(xid)
  }

  /// Toggle fullscreen via EWMH (_NET_WM_STATE_TOGGLE).
  pub fn toggle_fullscreen(xid: u32) -> bool {
    let Some((conn, screen)) = connection() else {
      return false;
    };
    let root = conn.setup().roots[*screen].root;
    let wm_state = match conn
      .intern_atom(false, b"_NET_WM_STATE")
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(r) => r.atom,
      None => return false,
    };
    let fs = match conn
      .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(r) => r.atom,
      None => return false,
    };
    let data = x11rb::protocol::xproto::ClientMessageData::from([2u32, fs, 0, 0, 0]);
    let event = x11rb::protocol::xproto::ClientMessageEvent {
      response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
      sequence: 0,
      window: xid,
      type_: wm_state,
      format: 32,
      data,
    };
    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
      | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
    conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
  }

  pub fn minimize_window(xid: u32) -> bool {
    let Some((conn, screen)) = connection() else {
      return false;
    };
    let root = conn.setup().roots[*screen].root;
    let wm_change = match conn
      .intern_atom(false, b"WM_CHANGE_STATE")
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(r) => r.atom,
      None => return false,
    };
    let data = x11rb::protocol::xproto::ClientMessageData::from([3u32, 0, 0, 0, 0]);
    let event = x11rb::protocol::xproto::ClientMessageEvent {
      response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
      sequence: 0,
      window: xid,
      type_: wm_change,
      format: 32,
      data,
    };
    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
      | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
    conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
  }

  pub fn close_window(xid: u32) -> bool {
    let Some((conn, screen)) = connection() else {
      return false;
    };
    let root = conn.setup().roots[*screen].root;
    let net_close = match conn
      .intern_atom(false, b"_NET_CLOSE_WINDOW")
      .ok()
      .and_then(|c| c.reply().ok())
    {
      Some(r) => r.atom,
      None => return false,
    };
    let data =
      x11rb::protocol::xproto::ClientMessageData::from([x11rb::CURRENT_TIME, 2, 0, 0, 0]);
    let event = x11rb::protocol::xproto::ClientMessageEvent {
      response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
      sequence: 0,
      window: xid,
      type_: net_close,
      format: 32,
      data,
    };
    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
      | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
    conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
  }
}

#[cfg(target_os = "linux")]
pub use imp::{
  close_window, get_active_app_name, get_active_window, get_client_list, get_window_pid,
  get_window_title, get_wm_class, minimize_window, toggle_fullscreen,
};

#[cfg(not(target_os = "linux"))]
mod stub {
  pub fn get_active_window() -> Option<u32> {
    None
  }
  pub fn get_window_pid(_: u32) -> Option<i32> {
    None
  }
  pub fn get_window_title(_: u32) -> Option<String> {
    None
  }
  pub fn get_client_list() -> Vec<u32> {
    Vec::new()
  }
  pub fn get_wm_class(_: u32) -> Option<String> {
    None
  }
  pub fn get_active_app_name() -> String {
    "Finder".to_string()
  }
  pub fn minimize_window(_: u32) -> bool {
    false
  }
  pub fn close_window(_: u32) -> bool {
    false
  }
  pub fn toggle_fullscreen(_: u32) -> bool {
    false
  }
}

#[cfg(not(target_os = "linux"))]
pub use stub::{
  close_window, get_active_app_name, get_active_window, get_client_list, get_window_pid,
  get_window_title, get_wm_class, minimize_window, toggle_fullscreen,
};
