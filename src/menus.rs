//! Menu content for the menubar: Octopus, app and example menus built
//! from [`crate::TontooUI`] `BarItem`s, plus action dispatch.
//!
//! Each builder returns the items and a parallel action table (one entry
//! per row, `None` for dividers and inert rows), so a `BarMenu` path like
//! `[2]` maps back to its action.

use crate::CoreIcon;
use crate::TontooUI::elements::{BarItem, BarMenu};
use crate::{actions, i18n};

/// Which top-level menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
  Octopus,
  App,
  Example(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub enum OctopusAction {
  None,
  OpenSystemOverview,
  OpenSystemSettings,
  ForceQuitActive,
}

#[derive(Clone, Copy, Debug)]
pub enum AppAction {
  HideCurrent,
  HideOthers,
  AboutApp,
  Quit,
}

#[derive(Clone, Copy, Debug)]
pub enum ExampleAction {
  None,
  MinimizeWindow,
  ToggleZoom,
  CloseWindow,
}

/// Octopus menu rows (macOS-style system menu).
pub fn octopus_menu(app_name: &str) -> (Vec<BarItem>, Vec<Option<OctopusAction>>) {
  // With no real app selected (bar shows "Finder"), Force Quit is disabled.
  let quit_enabled = app_name != "Finder";
  let user = std::env::var("USER").unwrap_or_else(|_| "liveuser".to_string());
  let rows: Vec<(BarItem, Option<OctopusAction>)> = vec![
    (
      BarItem::action(i18n::trk("menu.about"))
        .icon(CoreIcon::DESKTOPCOMPUTER.name())
        .build(),
      Some(OctopusAction::OpenSystemOverview),
    ),
    (
      BarItem::action(i18n::trk("menu.settings"))
        .icon(CoreIcon::GEARSHAPE_FILL.name())
        .build(),
      Some(OctopusAction::OpenSystemSettings),
    ),
    (
      BarItem::action(i18n::trk("menu.appstore"))
        .icon(CoreIcon::APP_FILL.name())
        .build(),
      Some(OctopusAction::None),
    ),
    (
      BarItem::action(i18n::trk("menu.recent"))
        .icon(CoreIcon::CLOCK_FILL.name())
        .build(),
      Some(OctopusAction::None),
    ),
    (BarItem::divider(), None),
    (
      BarItem::action(i18n::trk("menu.forcequit"))
        .icon(CoreIcon::XMARK_OCTAGON_FILL.name())
        .shortcut("⌥⌘⎋")
        .enabled(quit_enabled)
        .build(),
      Some(OctopusAction::ForceQuitActive),
    ),
    (BarItem::divider(), None),
    (
      BarItem::action(i18n::trk("menu.sleep"))
        .icon(CoreIcon::MOON_FILL.name())
        .build(),
      Some(OctopusAction::None),
    ),
    (
      BarItem::action(i18n::trk("menu.restart"))
        .icon(CoreIcon::ARROW_COUNTERCLOCKWISE.name())
        .build(),
      Some(OctopusAction::None),
    ),
    (
      BarItem::action(i18n::trk("menu.shutdown"))
        .icon(CoreIcon::POWER.name())
        .build(),
      Some(OctopusAction::None),
    ),
    (BarItem::divider(), None),
    (
      BarItem::action(i18n::trk("menu.lock"))
        .icon(CoreIcon::LOCK_FILL.name())
        .shortcut("^⌘Q")
        .build(),
      Some(OctopusAction::None),
    ),
    (
      BarItem::action(i18n::trk_args("menu.logout", &[("user", &user)]))
        .icon(CoreIcon::RECTANGLE_PORTRAIT_AND_ARROW_RIGHT.name())
        .shortcut("⇧⌘Q")
        .build(),
      Some(OctopusAction::None),
    ),
  ];
  split(rows)
}

/// App menu rows for `app_name` (Hide, About, Quit).
pub fn app_menu(app_name: &str) -> (Vec<BarItem>, Vec<Option<AppAction>>) {
  // Quit is disabled with no real app selected (grayed out).
  let quit_enabled = app_name != "Finder";
  let rows: Vec<(BarItem, Option<AppAction>)> = vec![
    (
      BarItem::action(i18n::trk_args("menu.hide_app", &[("app", app_name)]))
        .icon(CoreIcon::EYE_SLASH_FILL.name())
        .build(),
      Some(AppAction::HideCurrent),
    ),
    (
      BarItem::action(i18n::trk("menu.hide_others"))
        .icon(CoreIcon::EYE_SLASH.name())
        .build(),
      Some(AppAction::HideOthers),
    ),
    (BarItem::divider(), None),
    (
      BarItem::action(i18n::trk_args("menu.about_app", &[("app", app_name)]))
        .icon(CoreIcon::INFO_CIRCLE.name())
        .build(),
      Some(AppAction::AboutApp),
    ),
    (
      BarItem::action(i18n::trk_args("menu.quit_app", &[("app", app_name)]))
        .icon(CoreIcon::XMARK.name())
        .enabled(quit_enabled)
        .build(),
      Some(AppAction::Quit),
    ),
  ];
  split(rows)
}

/// Example app menus (static placeholders, macOS-style). Window rows act
/// on the focused window; everything else is inert.
pub fn example_menu(key: &str) -> (Vec<BarItem>, Vec<Option<ExampleAction>>) {
  let row = |label: &str, symbol: &CoreIcon::SFSymbol| {
    (
      BarItem::action(i18n::trk(label))
        .icon(symbol.name())
        .build(),
      Some(ExampleAction::None),
    )
  };
  let action_row = |label: &str, symbol: &CoreIcon::SFSymbol, action: ExampleAction| {
    (
      BarItem::action(i18n::trk(label))
        .icon(symbol.name())
        .build(),
      Some(action),
    )
  };
  let rows: Vec<(BarItem, Option<ExampleAction>)> = match key {
    "menu.edit" => vec![
      row("menu.edit_undo", &CoreIcon::ARROW_UTURN_BACKWARD),
      row("menu.edit_redo", &CoreIcon::ARROW_UTURN_FORWARD),
      (BarItem::divider(), None),
      row("menu.edit_cut", &CoreIcon::SCISSORS),
      row("menu.edit_copy", &CoreIcon::DOC_ON_DOC),
      row("menu.edit_paste", &CoreIcon::DOC_ON_CLIPBOARD),
      (BarItem::divider(), None),
      row("menu.edit_selectall", &CoreIcon::SQUARE_DASHED),
    ],
    "menu.view" => vec![
      row("menu.view_zoomin", &CoreIcon::PLUS_MAGNIFYINGGLASS),
      row("menu.view_zoomout", &CoreIcon::MINUS_MAGNIFYINGGLASS),
      row("menu.view_refresh", &CoreIcon::ARROW_CLOCKWISE),
    ],
    "menu.go" => vec![
      row("menu.go_back", &CoreIcon::CHEVRON_BACKWARD),
      row("menu.go_forward", &CoreIcon::CHEVRON_FORWARD),
      (BarItem::divider(), None),
      row("menu.go_recents", &CoreIcon::CLOCK),
      row("menu.go_documents", &CoreIcon::FOLDER),
      row("menu.go_desktop", &CoreIcon::FOLDER),
      row("menu.go_downloads", &CoreIcon::FOLDER),
      row("menu.go_home", &CoreIcon::HOUSE),
      row("menu.go_computer", &CoreIcon::DESKTOPCOMPUTER),
      row("menu.go_applications", &CoreIcon::FOLDER),
      row("menu.go_recentfolders", &CoreIcon::FOLDER_FILL),
      (BarItem::divider(), None),
      row("menu.go_tofolder", &CoreIcon::FOLDER_FILL),
    ],
    "menu.window" => vec![
      action_row(
        "menu.win_minimize",
        &CoreIcon::MINUS,
        ExampleAction::MinimizeWindow,
      ),
      action_row(
        "menu.win_zoom",
        &CoreIcon::ARROW_UP_LEFT_AND_ARROW_DOWN_RIGHT,
        ExampleAction::ToggleZoom,
      ),
      action_row("menu.win_close", &CoreIcon::XMARK, ExampleAction::CloseWindow),
      (BarItem::divider(), None),
      row("menu.win_front", &CoreIcon::SQUARE_STACK),
    ],
    _ => vec![
      row("menu.example1", &CoreIcon::CIRCLE),
      row("menu.example2", &CoreIcon::CIRCLE),
      (BarItem::divider(), None),
      row("menu.example3", &CoreIcon::CIRCLE),
    ],
  };
  split(rows)
}

fn split<T: Copy>(rows: Vec<(BarItem, Option<T>)>) -> (Vec<BarItem>, Vec<Option<T>>) {
  rows.into_iter().unzip()
}

/// Dispatch an octopus menu path.
pub fn handle_octopus(path: &[usize], table: &[Option<OctopusAction>]) {
  let action = path
    .first()
    .and_then(|i| table.get(*i))
    .copied()
    .flatten();
  match action {
    Some(OctopusAction::OpenSystemOverview) => actions::launch_system_overview(),
    Some(OctopusAction::OpenSystemSettings) => {
      actions::launch_via_tapp("/System/Applications/systemsettings.app", &[]);
    }
    Some(OctopusAction::ForceQuitActive) => actions::force_quit_current_app(),
    _ => {}
  }
}

/// Dispatch an app menu path.
pub fn handle_app(path: &[usize], table: &[Option<AppAction>]) {
  let action = path
    .first()
    .and_then(|i| table.get(*i))
    .copied()
    .flatten();
  match action {
    Some(AppAction::HideCurrent) => actions::hide_current_app(),
    Some(AppAction::HideOthers) => actions::hide_other_apps(),
    Some(AppAction::AboutApp) => actions::open_about_this_app(),
    Some(AppAction::Quit) => actions::quit_current_app(),
    _ => {}
  }
}

/// Dispatch an example menu path (window actions get the zoom set).
pub fn handle_example(
  path: &[usize],
  table: &[Option<ExampleAction>],
  zoomed: &mut std::collections::HashSet<u64>,
) {
  let action = path
    .first()
    .and_then(|i| table.get(*i))
    .copied()
    .flatten();
  match action {
    Some(ExampleAction::MinimizeWindow) => actions::minimize_focused_window(),
    Some(ExampleAction::ToggleZoom) => actions::toggle_zoom_focused_window(zoomed),
    Some(ExampleAction::CloseWindow) => actions::close_focused_window(),
    _ => {}
  }
}

/// Build a fresh `BarMenu` for `id` (content rebuilt on every open, so
/// labels and enabled states are always current).
pub fn build_menu(
  id: MenuId,
  app_name: &str,
  on_action: impl FnMut(Vec<usize>) + 'static,
) -> (BarMenu, MenuActions) {
  match id {
    MenuId::Octopus => {
      let (items, table) = octopus_menu(app_name);
      (BarMenu::new(items).on_action(on_action), MenuActions::Octopus(table))
    }
    MenuId::App => {
      let (items, table) = app_menu(app_name);
      (BarMenu::new(items).on_action(on_action), MenuActions::App(table))
    }
    MenuId::Example(key) => {
      let (items, table) = example_menu(key);
      (
        BarMenu::new(items).on_action(on_action),
        MenuActions::Example(table),
      )
    }
  }
}

/// Action table belonging to an open menu, for path dispatch.
#[derive(Debug)]
pub enum MenuActions {
  Octopus(Vec<Option<OctopusAction>>),
  App(Vec<Option<AppAction>>),
  Example(Vec<Option<ExampleAction>>),
}
