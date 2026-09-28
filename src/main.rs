//! TontooOS Menubar: transparent top bar on every monitor.
//!
//! One layer-shell surface per Wayland output (top edge, exclusive zone,
//! no keyboard focus), each driving a [`bar::MenubarApp`]. Bar height is
//! 30 logical px, 42 on notch outputs (macOS Tahoe-like).
//!
//! Modules: [`bar`] (bar UI), [`menus`] (menu content), [`actions`] (app
//! actions), [`daemon`] (window daemon polling), [`x11`] (EWMH queries),
//! [`i18n`] (translations).

sdk::preinclude!();

mod actions;
mod bar;
mod daemon;
mod i18n;
mod menus;
mod x11;

use crate::TontooUI::renderer::{LayerBarOptions, LayerOutput, run_layer};

fn main() {
  println!(
    "TontooOS Menubar v{}.{}",
    env!("CARGO_PKG_VERSION_MAJOR"),
    env!("CARGO_PKG_VERSION_MINOR"),
  );

  i18n::init_i18n();
  daemon::start_daemon_poller();

  if let Err(err) = run_layer(|output: LayerOutput| {
    let height = bar::bar_height_for_output(&output.name);
    println!(
      "[menubar] output '{}': width={} scale={} bar={}",
      output.name, output.width, output.scale, height
    );
    Some((
      Box::new(bar::MenubarApp::new(height, &output.name)) as Box<dyn TontooUI::renderer::window::App>,
      LayerBarOptions::new("menubar", height),
    ))
  }) {
    eprintln!("[menubar] error: {err}");
    std::process::exit(1);
  }
}
