//! Menubar UI: one transparent layer bar per output.
//!
//! Manual layout (no stacks, so trigger rects are exact): octopus,
//! bold app name, example menu titles, a spacer, status symbols and the
//! clock. Triggers open [`crate::TontooUI`] `BarMenu` panels; actions
//! dispatch through [`crate::menus`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::TontooUI::elements::{
  BarMenu, BasicText, FileImage, FormattedText, SFSymbolImage, Span, TextAlignment,
  TextStyle, View,
};
use crate::TontooUI::renderer::window::{App, Viewport, WindowCommand};
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::{ThemeMode, ThemeWatcher};
use crate::{actions, i18n};
use crate::menus::{self, MenuActions, MenuId};
use vello::Scene;
use vello::kurbo::Affine;
use vello::peniko::{Brush, Color, ColorStop, Fill, Gradient};

/// Bar height without a notch (logical px, macOS Tahoe-like).
pub const BAR_HEIGHT_STANDARD: u32 = 30;
/// Bar height with a notch (logical px).
pub const BAR_HEIGHT_NOTCH: u32 = 42;
/// Example menu titles (Help has no menu, title with hover only).
pub const EXAMPLE_TITLES: [&str; 6] = [
  "menu.file",
  "menu.edit",
  "menu.view",
  "menu.go",
  "menu.window",
  "menu.help",
];

/// Bar height for an output: notch logic from the old bar.
/// `MENUBAR_NOTCH=1|37|true` forces notch height; otherwise only internal
/// laptop panels (`eDP`/`LVDS`) on Apple hardware qualify (overridable
/// with `MENUBAR_NOTCH=0`).
pub fn bar_height_for_output(output_name: &str) -> u32 {
  if let Ok(v) = std::env::var("MENUBAR_NOTCH") {
    let l = v.to_lowercase();
    if ["1", "true", "37", "yes"].contains(&l.as_str()) {
      return BAR_HEIGHT_NOTCH;
    }
    if l == "0" || l == "false" || l == "no" {
      return BAR_HEIGHT_STANDARD;
    }
  }
  if let Ok(v) = std::env::var("NOTCH") {
    let l = v.to_lowercase();
    if l == "1" || l == "true" {
      return BAR_HEIGHT_NOTCH;
    }
  }
  let lower = output_name.to_lowercase();
  if !(lower.contains("edp") || lower.contains("lvds")) {
    return BAR_HEIGHT_STANDARD;
  }
  let vendor = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor").unwrap_or_default();
  let product = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
  let board = std::fs::read_to_string("/sys/class/dmi/id/board_name").unwrap_or_default();
  let combined = format!("{vendor} {product} {board}").to_lowercase();
  if combined.contains("apple") && combined.contains("macbook") {
    return BAR_HEIGHT_NOTCH;
  }
  BAR_HEIGHT_STANDARD
}

/// Clock text like `"Mon Jun 23 7:29 PM"` from system time.
pub fn clock_text(now: SystemTime) -> String {
  let secs = now
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  // Days since epoch -> civil date (Howard Hinnant algorithm).
  let days = (secs / 86_400) as i64;
  let z = days + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z.rem_euclid(146_097);
  let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
  let y = yoe + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = doy - (153 * mp + 2) / 5 + 1;
  let m = if mp < 10 { mp + 3 } else { mp - 9 };
  let year = if m <= 2 { y + 1 } else { y };
  let _ = year;
  // 1970-01-01 was a Thursday (weekday 4, Sunday = 0).
  const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ];
  let weekday = WEEKDAYS[(days.rem_euclid(7) as usize + 4) % 7];
  let month = MONTHS[(m - 1) as usize];
  let mins = (secs / 60) % (24 * 60);
  let hour24 = mins / 60;
  let minute = mins % 60;
  let (hour12, ampm) = match hour24 {
    0 => (12, "AM"),
    1..=11 => (hour24, "AM"),
    12 => (12, "PM"),
    _ => (hour24 - 12, "PM"),
  };
  format!("{weekday} {month} {d} {hour12}:{minute:02} {ampm}")
}

struct Trigger {
  id: MenuId,
  rect: (f32, f32, f32, f32),
}

pub struct MenubarApp {
  height: u32,
  octopus: FileImage,
  has_octopus: bool,
  app_name: FormattedText,
  titles: Vec<BasicText>,
  status: Vec<SFSymbolImage>,
  clock: BasicText,
  menu: BarMenu,
  open: Option<MenuId>,
  actions: Option<MenuActions>,
  pending: Rc<RefCell<Vec<Vec<usize>>>>,
  triggers: Vec<Trigger>,
  last_app: String,
  last_check: f64,
  last_clock: String,
  zoomed: HashSet<u64>,
  watcher: ThemeWatcher,
  text: Color,
  dark: bool,
}

impl MenubarApp {
  pub fn new(height: u32, output_name: &str) -> Self {
    let _ = output_name;
    let icon_size = (((height as f32 - 2.0) * 1.15).round() as f32).clamp(22.0, 48.0);
    let (octopus, has_octopus) = match actions::find_octopus_path() {
      Some(path) => (FileImage::new(PathBuf::from(path), icon_size, icon_size), true),
      None => (
        FileImage::new(PathBuf::from("__missing_octopus__"), icon_size, icon_size),
        false,
      ),
    };
    let mut titles = Vec::new();
    for key in EXAMPLE_TITLES {
      titles.push(
        BasicText::new(i18n::trk(key))
          .style(TextStyle::Footnote)
          .alignment(TextAlignment::Leading),
      );
    }
    let status = ["switch.2", "moon.fill", "battery.100", "wifi", "magnifyingglass"]
      .into_iter()
      .map(|s| SFSymbolImage::new(s).size(16.0))
      .collect();
    let pending: Rc<RefCell<Vec<Vec<usize>>>> = Rc::new(RefCell::new(Vec::new()));
    Self {
      height,
      octopus,
      has_octopus,
      app_name: FormattedText::spans(vec![Span::new("Finder").bold()])
        .style(TextStyle::Footnote)
        .alignment(TextAlignment::Leading),
      titles,
      status,
      clock: BasicText::new("--")
        .style(TextStyle::Footnote)
        .alignment(TextAlignment::Trailing),
      menu: BarMenu::new(Vec::new()),
      open: None,
      actions: None,
      pending,
      triggers: Vec::new(),
      last_app: "Finder".to_string(),
      last_check: f64::NEG_INFINITY,
      last_clock: String::new(),
      zoomed: HashSet::new(),
      watcher: ThemeWatcher::new(),
      text: Color::from_rgb8(0xd8, 0xd9, 0xd9),
      dark: true,
    }
  }

  /// Current bar display name (shell windows never appear).
  fn poll_app_name(&mut self, now: f64) {
    if now - self.last_check < 0.4 {
      return;
    }
    self.last_check = now;
    let cur = if crate::x11::get_active_window().is_none() {
      // No active window (e.g. bar click): keep the last real app.
      self.last_app.clone()
    } else {
      actions::selected_app().0
    };
    let next = if actions::is_shell_name(&cur) {
      if self.last_app.trim().is_empty() {
        "Finder".to_string()
      } else {
        self.last_app.clone()
      }
    } else {
      cur
    };
    if next != self.last_app {
      self.last_app = next.clone();
      self.app_name.set_source(vec![Span::new(next).bold()]);
      // Rebuild the open app menu so it tracks the new name.
      if self.open == Some(MenuId::App) {
        self.open_menu(MenuId::App);
      }
    }
  }

  fn poll_clock(&mut self) {
    let text = clock_text(SystemTime::now());
    if text != self.last_clock {
      self.last_clock = text.clone();
      self.clock.set_text(text);
    }
  }

  fn open_menu(&mut self, id: MenuId) {
    let pending = self.pending.clone();
    let (menu, acts) = menus::build_menu(id, &self.last_app, move |path| {
      pending.borrow_mut().push(path);
    });
    self.menu = menu;
    self.actions = Some(acts);
    self.open = Some(id);
    // Anchor below the trigger (set when placed; fallback to bar height).
    let anchor = self
      .triggers
      .iter()
      .find(|t| t.id == id)
      .map(|t| (t.rect.0, t.rect.1 + t.rect.3 + 6.0))
      .unwrap_or((0.0, self.height as f32 + 6.0));
    self.menu.open_at(anchor.0, anchor.1);
  }

  fn close_menu(&mut self) {
    self.menu.close();
    self.open = None;
    self.actions = None;
  }

  fn drain_actions(&mut self) {
    let paths: Vec<Vec<usize>> = self.pending.borrow_mut().drain(..).collect();
    for path in paths {
      match (&self.open, &self.actions) {
        (Some(MenuId::Octopus), Some(MenuActions::Octopus(table))) => {
          menus::handle_octopus(&path, table)
        }
        (Some(MenuId::App), Some(MenuActions::App(table))) => {
          menus::handle_app(&path, table)
        }
        (Some(MenuId::Example(_)), Some(MenuActions::Example(table))) => {
          menus::handle_example(&path, table, &mut self.zoomed)
        }
        _ => {}
      }
    }
  }

  fn trigger_at(&self, x: f32, y: f32) -> Option<MenuId> {
    self.triggers.iter().find_map(|t| {
      let (rx, ry, rw, rh) = t.rect;
      if x >= rx && x <= rx + rw && y >= ry && y <= ry + rh {
        Some(t.id)
      } else {
        None
      }
    })
  }

  /// Lay out the bar left to right, recording trigger rects. Returns the
  /// x cursor after the left block (for menus, unused) — right block is
  /// placed from the right edge.
  #[allow(clippy::too_many_arguments)]
  fn layout_bar(&mut self, fonts: &mut FontSystem, w: f32, h: f32) {
    self.triggers.clear();
    let mut x = 6.0f32;
    // Octopus trigger.
    if self.has_octopus {
      let (iw, ih) = self.octopus.measure(fonts);
      let iy = (h - ih) / 2.0;
      self.octopus.place(fonts, x, iy, iw, ih);
      self.triggers.push(Trigger {
        id: MenuId::Octopus,
        rect: (x - 4.0, 0.0, iw + 8.0, h),
      });
      x += iw + 2.0;
    }
    // App name trigger.
    {
      let (nw, nh) = self.app_name.measure(fonts);
      let ny = (h - nh) / 2.0;
      self.app_name.place(fonts, x, ny, nw, nh);
      self.triggers.push(Trigger {
        id: MenuId::App,
        rect: (x - 4.0, 0.0, nw + 18.0, h),
      });
      x += nw + 14.0;
    }
    // Example titles.
    for (index, key) in EXAMPLE_TITLES.into_iter().enumerate() {
      let id = MenuId::Example(key);
      // Help has no menu: title with hover only (no trigger).
      let title = &mut self.titles[index];
      let (tw, th) = title.measure(fonts);
      let ty = (h - th) / 2.0;
      title.place(fonts, x, ty, tw, th);
      if key != "menu.help" {
        self.triggers.push(Trigger {
          id,
          rect: (x - 4.0, 0.0, tw + 8.0, h),
        });
      }
      x += tw + 2.0;
    }
    // Right block: status icons + clock, right-aligned.
    let mut right_w = 10.0f32;
    let (cw, _) = self.clock.measure(fonts);
    right_w += cw;
    for icon in &mut self.status {
      let (iw, _) = icon.measure(fonts);
      right_w += 8.0 + iw;
    }
    let mut rx = w - right_w;
    for icon in &mut self.status {
      let (iw, ih) = icon.measure(fonts);
      icon.place(fonts, rx, (h - ih) / 2.0, iw, ih);
      rx += iw + 8.0;
    }
    let (_, ch) = self.clock.measure(fonts);
    self.clock.place(fonts, rx, (h - ch) / 2.0, cw, ch);
  }

  fn theme_children(&mut self, mode: ThemeMode) {
    let dark = mode == ThemeMode::Dark;
    self.octopus.set_theme(dark);
    self.octopus.set_focused(true);
    self.app_name.set_theme(mode);
    self.app_name.set_focused(true);
    for title in &mut self.titles {
      title.set_theme(mode);
      title.set_focused(true);
    }
    for icon in &mut self.status {
      icon.set_theme(self.text, dark);
      icon.set_focused(true);
    }
    self.clock.set_theme(mode);
    self.clock.set_focused(true);
    self.menu.set_theme(mode);
    self.menu.set_focused(true);
  }

  /// Top readability shadow (dark mode only), like the old bar gradient.
  fn draw_shade(&self, scene: &mut Scene, scale: f32, w: f32, h: f32) {
    if !self.dark {
      return;
    }
    let s = scale as f64;
    let stops = vec![
      ColorStop {
        offset: 0.0,
        color: Color::from_rgba8(0, 0, 0, 61).into(),
      },
      ColorStop {
        offset: 0.3,
        color: Color::from_rgba8(0, 0, 0, 38).into(),
      },
      ColorStop {
        offset: 0.6,
        color: Color::from_rgba8(0, 0, 0, 0).into(),
      },
      ColorStop {
        offset: 1.0,
        color: Color::from_rgba8(0, 0, 0, 0).into(),
      },
    ];
    let brush = Brush::Gradient(
      Gradient::new_linear((0.0, 0.0), (0.0, h as f64 * s)).with_stops(stops.as_slice()),
    );
    let rect = vello::kurbo::Rect::new(0.0, 0.0, w as f64 * s, h as f64 * s);
    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &rect);
  }
}

impl App for MenubarApp {
  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
    viewport: Viewport,
    time_secs: f64,
  ) {
    self.watcher.poll(time_secs);
    self.watcher.set_focused(true, time_secs);
    let palette = self.watcher.palette(time_secs);
    let mode = self.watcher.theme().mode;
    self.dark = mode == ThemeMode::Dark;
    self.text = palette.text;

    self.poll_app_name(time_secs);
    self.poll_clock();
    self.theme_children(mode);
    self.drain_actions();

    let (w, h) = (viewport.width, viewport.height);
    self.layout_bar(fonts, w, h);
    self.menu.set_viewport(viewport.x, viewport.y, w, h);

    self.draw_shade(scene, fonts.scale, w, h);
    if self.has_octopus {
      self.octopus.draw(scene, fonts, images);
    }
    self.app_name.draw(scene, fonts, images);
    for title in &mut self.titles {
      title.draw(scene, fonts, images);
    }
    for icon in &mut self.status {
      icon.draw(scene, fonts, images);
    }
    self.clock.draw(scene, fonts, images);
    if self.open.is_some() {
      self.menu.draw(scene, fonts, images);
    }
  }

  fn background(&self) -> Color {
    Color::TRANSPARENT
  }

  fn transparent_body(&self) -> bool {
    true
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    None
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    let (fx, fy) = (x as f32, y as f32);
    if let Some(id) = self.trigger_at(fx, fy) {
      // Toggle: same menu closes, another one switches.
      if self.open == Some(id) {
        self.close_menu();
      } else {
        self.open_menu(id);
      }
      return;
    }
    if self.open.is_some() {
      self.menu.mouse_down(x, y);
      if !self.menu.is_open() {
        self.open = None;
        self.actions = None;
      }
    }
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    if self.open.is_some() && self.trigger_at(x as f32, y as f32).is_none() {
      self.menu.mouse_up(x, y);
      self.drain_actions();
      if !self.menu.is_open() {
        self.open = None;
        self.actions = None;
      }
    }
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    let (fx, fy) = (x as f32, y as f32);
    // macOS hover-switch: hovering another trigger while a menu is open
    // switches to it.
    if self.open.is_some() {
      if let Some(id) = self.trigger_at(fx, fy) {
        if Some(id) != self.open {
          self.open_menu(id);
          return;
        }
      }
      self.menu.set_hover(fx, fy);
    }
  }

  fn set_focused(&mut self, _focused: bool) {
    // Layer surfaces never take keyboard focus; never desaturate.
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bar_height_env_override() {
    std::env::set_var("MENUBAR_NOTCH", "1");
    assert_eq!(bar_height_for_output("HDMI-1"), BAR_HEIGHT_NOTCH);
    std::env::set_var("MENUBAR_NOTCH", "0");
    assert_eq!(bar_height_for_output("eDP-1"), BAR_HEIGHT_STANDARD);
    std::env::remove_var("MENUBAR_NOTCH");
    std::env::remove_var("NOTCH");
  }

  #[test]
  fn clock_formats_like_date() {
    // 2026-09-28 14:05 UTC is a Monday.
    let at = UNIX_EPOCH + std::time::Duration::from_secs(1_790_604_300);
    assert_eq!(clock_text(at), "Mon Sep 28 2:05 PM");
  }

  #[test]
  fn shell_names_filtered() {
    assert!(crate::actions::is_shell_name("menubar"));
    assert!(crate::actions::is_shell_name(""));
    assert!(!crate::actions::is_shell_name("Finder"));
  }
}
