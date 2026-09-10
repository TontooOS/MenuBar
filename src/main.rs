//! TontooOS Menubar
//!
//! - Breite: immer gesamte Bildschirmbreite (z.B. 1920 bei 1920x1080).
//! - Höhe: ohne Notch 24 logische Punkte (48 phys. @2x), mit Notch 37
//!   logische Punkte (74 phys. @2x) — wie macOS Tahoe.
//! - Komplett transparent, oben am Bildschirm platziert.
//! - Pro Monitor: eigenes Fenster (1:1 Spiegelung auf allen Bildschirmen).
//! - Links Octopus (CoreIcon per SDK, immer weiß) + "Test".
//! - SF Pro Display von /usr/share/fonts/OTF bzw. /usr/share/fonts/TTF.
//! - Verwendet UIKit fuer ColorScheme-Erkennung, sonst reines GTK4.
//! - Positionierung via X11 (GDK_BACKEND=x11) analog zum Dock.

use gtk::prelude::*;
use gtk::{Application, ApplicationWindow};
use std::cell::RefCell;
use std::rc::Rc;

sdk::preinclude!();
use UIKit::prelude::*;
// CoreIcon via SDK (wichtig: per SDK Lib, nicht direkter Pfad)
use CoreIcon::octopus::OctopusVariant;
use CoreIcon::{Color as IconColor, SFSymbol};
use CoreIcon::generator::{IconCanvas, RecolorMode, RecolorOptions};
// CoreWindows via SDK: selected-app resolution + Hide/Quit/ForceQuit actions.
use CoreWindows::WindowsProvider;
// Accessibility via SDK: DE/EN translations from lang/*.json.
use Accessibility::LangStore;

// ── Config ──────────────────────────────────────────────────────────────
// Breite: immer gesamte Bildschirmbreite (z.B. 1920 bei 1920x1080).
// Höhe: macOS 24–25 / 37 logische Punkte, für TontooOS etwas größer
// damit Octopus sichtbar bleibt (Asset hat Padding → Glyphe kleiner
// als Allocation). Standard nun 30 (60 phys @2x), Notch 42 (84 phys).
const BAR_HEIGHT_STANDARD: i32 = 30;
const BAR_HEIGHT_NOTCH: i32 = 42;
/// SF Pro wird im ISO unter /usr/share/fonts/OTF bzw. TTF installiert.
const SF_FAMILY: &str = "SF Pro Display";

thread_local! {
    static APP_MENUBAR_STATE: std::cell::RefCell<Vec<(gtk::Label, gtk::Popover)>> = std::cell::RefCell::new(Vec::new());
    static LAST_APP_NAME: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
    static OCTOPUS_MENUS: std::cell::RefCell<Vec<gtk::Popover>> = std::cell::RefCell::new(Vec::new());
    static LAST_SCHEME: std::cell::RefCell<Option<ColorScheme>> = std::cell::RefCell::new(None);
    static TOP_MENUS: std::cell::RefCell<Vec<(gtk::Box, gtk::Popover, TopMenuKind)>> = std::cell::RefCell::new(Vec::new());
    static BAR_WINDOWS: std::cell::RefCell<Vec<ApplicationWindow>> = std::cell::RefCell::new(Vec::new());
    static ZOOMED_WINDOWS: std::cell::RefCell<std::collections::HashSet<u64>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Kind of a top-level menubar menu (trigger wrap + popover).
#[derive(Clone, Copy)]
enum TopMenuKind {
    Octopus,
    App,
    Example(&'static str),
}

/// Höhe für einen Monitor bestimmen (Notch-Logik).
fn bar_height_for_monitor(monitor: Option<&gtk::gdk::Monitor>) -> i32 {
    if is_notch_monitor(monitor) {
        BAR_HEIGHT_NOTCH
    } else {
        BAR_HEIGHT_STANDARD
    }
}

/// Heuristik ob Monitor eine Notch hat (MacBook Air/Pro M1+).
/// 1) Env-Override `MENUBAR_NOTCH=1|37|true` erzwingt Notch-Höhe.
/// 2) Auto-Detect: nur interne Displays (`eDP`/`LVDS`) auf Apple-Hardware.
///    Externe Displays / iMac / alte MacBooks bleiben bei 24.
fn is_notch_monitor(monitor: Option<&gtk::gdk::Monitor>) -> bool {
    if let Ok(v) = std::env::var("MENUBAR_NOTCH") {
        let l = v.to_lowercase();
        return l == "1" || l == "true" || l == "37" || l == "yes";
    }
    if let Ok(v) = std::env::var("NOTCH") {
        let l = v.to_lowercase();
        if l == "1" || l == "true" { return true; }
    }
    // Nur interne Laptop-Panels sind Kandidaten für Notch
    let is_internal = if let Some(m) = monitor {
        m.connector()
            .map(|c| {
                let s = c.to_string().to_lowercase();
                s.contains("edp") || s.contains("lvds")
            })
            .unwrap_or(false)
    } else {
        false
    };
    if !is_internal {
        return false;
    }
    // Apple-Hardware prüfen
    let vendor = std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
        .unwrap_or_default()
        .to_lowercase();
    let product = std::fs::read_to_string("/sys/class/dmi/id/product_name")
        .unwrap_or_default()
        .to_lowercase();
    let board = std::fs::read_to_string("/sys/class/dmi/id/board_name")
        .unwrap_or_default()
        .to_lowercase();
    let combined = format!("{vendor} {product} {board}");
    if combined.contains("apple") && combined.contains("macbook") {
        // MacBookAir10,* / MacBookPro18,*+ haben Notch (ab 2021)
        // Sehr simpel: jedes Apple MacBook mit internem Panel -> Notch
        // (ältere Intel ohne Notch werden so fälschlich als Notch erkannt,
        //  können via MENUBAR_NOTCH=0 übersteuert werden)
        return true;
    }
    false
}

// ── i18n via Accessibility ──────────────────────────────────────────────
/// System language: "de_de" when $LANG/$LC_ALL starts with "de", else "en_us".
fn sys_lang() -> String {
    let code = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_else(|_| "en_US".to_string())
        .to_lowercase();
    if code.starts_with("de") {
        "de_de".to_string()
    } else {
        "en_us".to_string()
    }
}

/// Translate `key` with the system language (Accessibility LangStore).
fn trk(key: &str) -> String {
    let lang = sys_lang();
    LangStore::instance()
        .t(&lang, key, None)
        .unwrap_or_else(|| key.to_string())
}

/// Translate `key` with `%name%` placeholders from `args`.
fn trk_args(key: &str, args: &[(&str, &str)]) -> String {
    let map: std::collections::HashMap<String, String> = args
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let lang = sys_lang();
    LangStore::instance()
        .t(&lang, key, Some(&map))
        .unwrap_or_else(|| key.to_string())
}

/// Init the Accessibility LangStore from the menubar lang dir candidates.
/// Call once at startup, before any translation.
fn init_i18n() {
    let candidates = [
        format!("{}/lang", env!("CARGO_MANIFEST_DIR")),
        "./lang".to_string(),
        "/usr/share/tontoo/menubar/lang".to_string(),
        "/mnt/c/Users/arlo1/Documents/TontooProgramms/Menubar/lang".to_string(),
    ];
    let mut files = Vec::new();
    for dir in &candidates {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(f) = Accessibility::LangFile::from_file(&path) {
                        files.push(f);
                    }
                }
            }
        }
        if !files.is_empty() {
            break;
        }
    }
    if let Err(e) = LangStore::init(files, Some("en_us".to_string())) {
        eprintln!("[menubar] i18n init failed: {e}");
    }
}

// ── Transparent Window + Top-to-Bottom Shadow Gradient ────────────────
fn install_transparent_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        // Window selbst transparent, Bar mit Verlauf von ganz oben (stark)
        // bis Ende der Bar (leicht) fuer Lesbarkeit.
        // SF Pro Fonts liegen im ISO unter /usr/share/fonts/OTF + /TTF.
        // OS-Compositor-Shadow deaktiviert, eigener Shadow = Verlauf.
        provider.load_from_string(&format!(
            r#"
            window {{
                background-color: transparent;
                background: transparent;
                box-shadow: none;
            }}
            window decoration,
            decoration,
            headerbar,
            .csd decoration,
            .solid-csd decoration,
            window.csd decoration,
            window.solid-csd decoration {{
                background: transparent;
                background-color: transparent;
                box-shadow: none;
                border: none;
                outline: none;
            }}
            .menubar-root {{
                background: linear-gradient(to bottom,
                    rgba(0,0,0,0.24) 0%,
                    rgba(0,0,0,0.15) 30%,
                    rgba(0,0,0,0) 60%,
                    rgba(0,0,0,0) 100%);
                background-color: transparent;
            }}
            .menubar-bar {{
                background: linear-gradient(to bottom,
                    rgba(0,0,0,0.24) 0%,
                    rgba(0,0,0,0.15) 30%,
                    rgba(0,0,0,0) 60%,
                    rgba(0,0,0,0) 100%);
                background-color: transparent;
                min-height: {h}px;
                border: none;
                box-shadow: none;
            }}
            .menubar-label {{
                font-family: '{sf}', 'SF Pro', sans-serif;
                font-size: 13px;
                font-weight: 400;
                color: #F5F5F7;
            }}
            /* App name is bold: same provider + later rule beats .menubar-label. */
            .menubar-label.app-name-label {{
                font-weight: 800;
                letter-spacing: -0.2px;
            }}
            .menubar-octopus {{
                background: transparent;
                box-shadow: none;
            }}
            /* Octopus hover/selected — merkt man dass er selectbar ist */
            .octopus-wrap {{
                background: transparent;
                border-radius: 6px;
                padding: 2px 4px;
                transition: background 140ms ease;
            }}
            .octopus-wrap.octopus-hover {{
                background: rgba(255,255,255,0.12);
            }}
            .octopus-wrap.octopus-active {{
                background: rgba(255,255,255,0.18);
            }}
            /* Popover / Menu — transparent background, wie macOS */
            popover {{
                background: transparent;
                border: none;
                box-shadow: none;
            }}
            popover contents {{
                background: transparent;
                border: none;
            }}
            popover > contents {{
                background: transparent;
            }}
            .octopus-menu {{
                background: rgba(36,36,38,0.88);
                border-radius: 12px;
                padding: 6px;
                border: 1px solid rgba(255,255,255,0.14);
                box-shadow: none;
                min-width: 260px;
            }}
            .octopus-menu.light {{
                background: rgba(236,236,236,0.94);
                border: 1px solid rgba(0,0,0,0.12);
            }}
            .octopus-menu-item {{
                background: transparent;
                border: none;
                border-radius: 6px;
                padding: 0px;
                min-height: 26px;
                transition: background 120ms ease;
            }}
            .octopus-menu-item:hover,
            .octopus-menu-item.hovered {{
                background: rgba(255,255,255,0.12);
            }}
            .octopus-menu.light .octopus-menu-item:hover,
            .octopus-menu.light .octopus-menu-item.hovered {{
                background: rgba(0,0,0,0.08);
            }}
            .octopus-menu-item:active {{
                background: rgba(255,255,255,0.18);
            }}
            .octopus-menu.light .octopus-menu-item:active {{
                background: rgba(0,0,0,0.12);
            }}
            /* Disabled rows (e.g. Force Quit / Quit with no real app selected) */
            .octopus-menu-item:disabled {{
                background: transparent;
            }}
            .octopus-menu-item:disabled .octopus-menu-label {{
                color: rgba(245,245,247,0.35);
            }}
            .octopus-menu.light .octopus-menu-item:disabled .octopus-menu-label {{
                color: rgba(30,30,30,0.35);
            }}
            .octopus-menu-item:disabled .octopus-menu-shortcut {{
                color: rgba(245,245,247,0.25);
            }}
            .octopus-menu.light .octopus-menu-item:disabled .octopus-menu-shortcut {{
                color: rgba(30,30,30,0.25);
            }}
            .octopus-menu-item:disabled .octopus-menu-icon {{
                opacity: 0.35;
            }}
            .octopus-menu-label {{
                color: #F5F5F7;
                font-family: '{sf}', 'SF Pro Display', sans-serif;
                font-size: 13px;
                font-weight: 400;
            }}
            .octopus-menu.light .octopus-menu-label {{
                color: #1E1E1E;
            }}
            .octopus-menu-shortcut {{
                color: rgba(245,245,247,0.55);
                font-family: '{sf}', 'SF Pro Display', sans-serif;
                font-size: 12px;
            }}
            .octopus-menu.light .octopus-menu-shortcut {{
                color: rgba(30,30,30,0.55);
            }}
            .octopus-menu-icon {{
                background: transparent;
            }}
            separator.octopus-sep {{
                background: rgba(255,255,255,0.12);
                min-height: 1px;
                margin: 4px 8px;
            }}
            .octopus-menu.light separator.octopus-sep {{
                background: rgba(0,0,0,0.12);
            }}
            "#,
            h = BAR_HEIGHT_STANDARD,
            sf = SF_FAMILY
        ));
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 20,
        );
    }
}

// ── CoreIcon Octopus (per SDK, immer weiß) ─────────────────────────────
/// Resolve the white octopus PNG via SDK/CoreIcon assets.
/// Versucht mehrere absolute Pfade (Cargo-Manifest, WSL-mount, System).
fn find_octopus_white_path() -> Option<String> {
    // SDK/CoreIcon via `use CoreIcon` — Pfade müssen absolut aufgelöst werden,
    // da `OctopusVariant::White.path()` relativ zu `assets/TontooOS` ist und
    // aus dem Menubar-CWD nicht auflösbar wäre.
    let candidates = [
        // Dev-Layout: Menubar -> ../../TontooLibs/CoreIcon/...
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
        "C:/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/TontooOS/Tontoo_White.png".to_string(),
        "/Library/System/CoreIcon/assets/TontooOS/Tontoo_White.png".to_string(),
        // Fallback: relative zum CWD (CoreIcon crate root)
        "assets/TontooOS/Tontoo_White.png".to_string(),
        OctopusVariant::White.path(),
    ];
    for p in candidates {
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}

/// Erzeuge ein Temp-PNG des weißen Octopus (via SDK `OctopusVariant::White`
/// + `IconColor::WHITE` Tint). Falls die Asset-Datei gefunden wurde, wird
/// sie nach `/tmp` skaliert/kopiert, damit `gtk::Image` kein riesiges
/// 1024px-PNG skalieren muss und der Pfad nach Installation stabil ist.
fn octopus_white_temp_path() -> Option<String> {
    if let Some(src) = find_octopus_white_path() {
        let out = std::env::temp_dir()
            .join("tontoo-menubar-octopus-white-32.png")
            .to_string_lossy()
            .to_string();

        if std::path::Path::new(&out).exists() {
            return Some(out);
        }

        let sdk_ok = (|| -> Option<()> {
            let img = CoreIcon::octopus::use_octopus_variant(OctopusVariant::White, IconColor::WHITE).ok()?;
            // 128px für scharfe Retina-Darstellung (40*2=80 phys + Reserve)
            let thumb = image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Lanczos3);
            thumb.save(&out).ok()?;
            Some(())
        })();

        if sdk_ok.is_some() && std::path::Path::new(&out).exists() {
            return Some(out);
        }

        if let Ok(img) = image::open(&src) {
            let thumb = img.resize(128, 128, image::imageops::FilterType::Lanczos3);
            if thumb.save(&out).is_ok() {
                return Some(out);
            }
        }
        // Letzter Fallback: direkt Originalpfad nutzen (GTK skaliert selbst)
        return Some(src);
    }
    None
}

fn build_octopus_widget(bar_height: i32) -> gtk::Widget {
    // Nur Icon 15% größer als vorher (bar-2), Bar selbst bleibt gleich.
    // Dadurch ragt das Icon leicht aus der Bar heraus (overflow).
    // Vorher bar-2 (28/40) → neu (bar-2)*1.15 ≈ 32/46.
    let base = (bar_height - 2).clamp(20, 40);
    let icon_size = ((base as f32 * 1.15).round() as i32).clamp(22, 48);
    let _ = OctopusVariant::White;
    let _ = IconColor::WHITE;

    if let Some(path) = octopus_white_temp_path().or_else(find_octopus_white_path) {
        let img = gtk::Image::from_file(&path);
        img.set_pixel_size(icon_size);
        img.add_css_class("menubar-octopus");
        img.set_valign(gtk::Align::Center);
        img.set_halign(gtk::Align::Start);
        // Weiß erzwingen: SDK-generiertes PNG ist bereits WHITE getintet (Shaded-Mode).
        // Kein extra Padding, damit Glyphe die Bar optisch füllt.
        UIKit::widget::apply_css(
            &img,
            ".menubar-octopus { background: transparent; -gtk-icon-filter: none; margin: 0; padding: 0; }",
        );
        return img.upcast();
    }

    let lbl = gtk::Label::new(Some("🐙"));
    lbl.add_css_class("menubar-octopus");
    lbl.set_valign(gtk::Align::Center);
    lbl.set_halign(gtk::Align::Start);
    UIKit::widget::apply_css(&lbl, ".menubar-octopus { color: white; font-size: 16px; background: transparent; }");
    lbl.upcast()
}

// ── CoreIcon SF Icons für Menü (transparent + hover) ───────────────────
fn ensure_coreicon_assets() {
    // Generator nutzt `ASSETS_DIR` relativ — für Menubar absolut setzen.
    // Versuche mehrere Kandidaten (Cargo-Manifest, WSL-mount).
    let candidates = [
        format!("{}/../../TontooLibs/CoreIcon/assets/icons", env!("CARGO_MANIFEST_DIR")),
        "/mnt/c/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/icons".to_string(),
        "C:/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/icons".to_string(),
        "/Library/System/CoreIcon/assets/icons".to_string(),
        "assets/icons".to_string(),
    ];
    for c in candidates {
        if std::path::Path::new(&c).exists() {
            unsafe { CoreIcon::generator::ASSETS_DIR = Box::leak(c.into_boxed_str()) };
            break;
        }
    }
}

fn sf_icon_path(symbol: SFSymbol) -> Option<String> {
    ensure_coreicon_assets();
    let name = symbol.name();
    let rel = format!("{}.png", name);
    // Nach ASSETS_DIR auflösen — generator nutzt ASSETS_DIR global
    let base = unsafe { CoreIcon::generator::ASSETS_DIR };
    let p = format!("{}/{}", base, rel);
    if std::path::Path::new(&p).exists() {
        return Some(p);
    }
    // Fallback absolute Kandidaten
    let candidates = [
        format!("{}/../../TontooLibs/CoreIcon/assets/icons/{}", env!("CARGO_MANIFEST_DIR"), rel),
        format!("/mnt/c/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/icons/{}", rel),
        format!("C:/Users/arlo1/Documents/TontooLibs/CoreIcon/assets/icons/{}", rel),
    ];
    for c in candidates {
        if std::path::Path::new(&c).exists() {
            return Some(c);
        }
    }
    None
}

fn menu_sf_icon(symbol: SFSymbol, color: IconColor) -> gtk::Widget {
    const SIZE: i32 = 16;
    let color_tag = if color == IconColor::WHITE { "w" } else { "b" };
    let out = format!("/tmp/tontoo-menu-icon-{}-{}-{}.png", symbol.name().replace('.', "-"), SIZE, color_tag);
    if !std::path::Path::new(&out).exists() {
        if let Some(src_path) = sf_icon_path(symbol) {
            if let Ok(img) = image::open(&src_path) {
                let rgba = img.to_rgba8();
                let opts = RecolorOptions::new(color, 1.0).mode(RecolorMode::Replace);
                let tinted = IconCanvas::recolor_image(&rgba, &opts);
                let thumb = image::imageops::resize(&tinted, SIZE as u32, SIZE as u32, image::imageops::FilterType::Lanczos3);
                let _ = thumb.save(&out);
            }
        }
    }
    if std::path::Path::new(&out).exists() {
        let img = gtk::Image::from_file(&out);
        img.set_pixel_size(SIZE);
        img.add_css_class("octopus-menu-icon");
        return img.upcast();
    }
    // Fallback: leeres 16px-Box falls Symbol nicht gefunden
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_size_request(SIZE, SIZE);
    b.upcast()
}

fn build_octopus_menu(parent: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_autohide(true);
    popover.set_parent(parent);
    popover.set_offset(0, 6);
    set_octopus_menu_content(&popover);
    OCTOPUS_MENUS.with(|s| s.borrow_mut().push(popover.clone()));
    popover
}

/// Launch a .app bundle via the tapp runner, with optional extra arguments
/// (e.g. `tapp /System/Applications/AboutThisApp.app /path/to/Steam.app`).
/// TApp is always installed on the machine, so /usr/bin/tapp exists.
fn launch_via_tapp(bundle: &str, args: &[String]) -> bool {
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

/// Launch ~/Applications/SystemOverview.app via the tapp runner.
fn launch_system_overview() {
    let Ok(home) = std::env::var("HOME") else {
        eprintln!("[menubar] cannot launch SystemOverview: $HOME not set");
        return;
    };
    launch_via_tapp(&format!("{home}/Applications/SystemOverview.app"), &[]);
}

/// .app bundle path of the selected app, if it runs from a real bundle
/// (CoreWindows classification). Plain binaries have no bundle path.
fn selected_app_bundle() -> Option<String> {
    let pid = x11_place::get_active_window().and_then(x11_place::get_window_pid)?;
    if !daemon_reachable() {
        return None;
    }
    let raws = WindowsProvider::from_env().list_raw().ok()?;
    let raw = raws.iter().find(|w| w.pid == Some(pid))?;
    CoreWindows::classify(raw)
        .bundle_dir
        .map(|p| p.to_string_lossy().to_string())
}

/// Open AboutThisApp for the selected app:
/// `tapp /System/Applications/AboutThisApp.app <bundle path>`.
/// Without a known bundle path, AboutThisApp still opens (no argument).
fn open_about_this_app() {
    const ABOUT: &str = "/System/Applications/AboutThisApp.app";
    let extra = selected_app_bundle().map(|b| vec![b]).unwrap_or_default();
    launch_via_tapp(ABOUT, &extra);
}

/// Fast reachability probe for the window daemon.
/// The CoreWindows lib blocks up to 10s per call (REQUEST_TIMEOUT), so it
/// must never be touched on the GTK main thread unless a connection
/// succeeds immediately. Missing socket / refused connection = fast `false`.
fn daemon_reachable() -> bool {
    let path = std::env::var("WINDOWS_SOCKET")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(CoreWindows::DEFAULT_SOCKET_PATH));
    if !path.exists() {
        return false;
    }
    std::os::unix::net::UnixStream::connect(&path).is_ok()
}

/// Daemon window list, but only when the daemon answers immediately.
/// Returns `None` when unreachable so callers fall back to X11 fast.
fn corewindows_windows() -> Option<Vec<CoreWindows::WindowInfo>> {
    if !daemon_reachable() {
        return None;
    }
    WindowsProvider::from_env().windows().ok()
}

/// Shell windows (menubar/dock) are never shown as the app name:
/// the bar keeps displaying the last real app instead.
fn is_shell_name(name: &str) -> bool {
    matches!(name.trim().to_lowercase().as_str(), "menubar" | "dock" | "")
}

/// True while the pointer is over one of our bar windows.
fn pointer_over_bar() -> bool {
    let Some(display) = gtk::gdk::Display::default() else {
        return false;
    };
    let Some(pointer) = display.default_seat().and_then(|s| s.pointer()) else {
        return false;
    };
    let (surf_opt, _, _) = pointer.surface_at_position();
    let Some(surf) = surf_opt else {
        return false;
    };
    BAR_WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .any(|win| win.surface().as_ref() == Some(&surf))
    })
}

/// Bar display name: the selected app, but shell windows never appear.
/// Clicking the menubar keeps the previous app shown; only focusing a
/// real window changes the text.
fn bar_app_name() -> String {
    // Clicking the bar can unfocus the app without focusing anything new:
    // while the pointer is over the bar, keep the last real app instead
    // of falling back to "Finder".
    if x11_place::get_active_window().is_none() && pointer_over_bar() {
        let last = LAST_APP_NAME.with(|s| s.borrow().clone());
        if !last.trim().is_empty() {
            return last;
        }
    }
    let cur = selected_app().0;
    if is_shell_name(&cur) {
        let last = LAST_APP_NAME.with(|s| s.borrow().clone());
        if last.trim().is_empty() {
            "Finder".to_string()
        } else {
            last
        }
    } else {
        cur
    }
}

/// Selected app as (display name, pid).
/// Name resolution via CoreWindows (real .app bundles), X11 WM_CLASS fallback.
/// The daemon has no focus concept, so the active window still comes from X11
/// and is bridged to CoreWindows via _NET_WM_PID.
fn selected_app() -> (String, Option<i32>) {
    let pid = x11_place::get_active_window().and_then(x11_place::get_window_pid);
    if let Some(pid) = pid {
        if let Some(windows) = corewindows_windows() {
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
    (x11_place::get_active_app_name(), pid)
}

/// Daemon window ids owned by `pid` (empty when the daemon is unreachable).
fn daemon_windows_of(pid: i32) -> Vec<u64> {
    corewindows_windows()
        .map(|ws| {
            ws.into_iter()
                .filter(|w| w.pid == Some(pid))
                .map(|w| w.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Hide the selected app (minimize its windows). X11 fallback when the
/// daemon is unreachable.
fn hide_current_app() {
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
    if let Some(xid) = x11_place::get_active_window() {
        x11_place::minimize_window(xid);
    }
}

/// Hide everything except the selected app. X11 fallback when unreachable.
fn hide_other_apps() {
    let active_pid = x11_place::get_active_window().and_then(x11_place::get_window_pid);
    let provider = WindowsProvider::from_env();
    if let Some(windows) = corewindows_windows() {
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
    let active = x11_place::get_active_window();
    for xid in x11_place::get_client_list() {
        if Some(xid) != active {
            // Menubar/Dock selbst nicht verstecken (sind dock-type, meist nicht in Liste)
            x11_place::minimize_window(xid);
        }
    }
}

/// Quit the selected app (graceful close of its windows). X11 fallback.
fn quit_current_app() {
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
        if let Some(xid) = x11_place::get_active_window() {
            x11_place::close_window(xid);
        }
    }
}

/// Daemon id of the focused window: match active pid + title,
/// else first window of the active pid. None when unreachable.
fn focused_daemon_window() -> Option<u64> {
    let xid = x11_place::get_active_window()?;
    let pid = x11_place::get_window_pid(xid)?;
    let windows = corewindows_windows()?;
    let own: Vec<&CoreWindows::WindowInfo> =
        windows.iter().filter(|w| w.pid == Some(pid)).collect();
    if own.is_empty() {
        return None;
    }
    if let Some(title) = x11_place::get_window_title(xid) {
        if let Some(w) = own.iter().find(|w| w.title.as_deref() == Some(title.as_str())) {
            return Some(w.id);
        }
    }
    own.first().map(|w| w.id)
}

/// Minimize the focused window. X11 fallback when the daemon is unreachable.
fn minimize_focused_window() {
    if let Some(id) = focused_daemon_window() {
        if WindowsProvider::from_env().minimize_window(id).is_ok() {
            return;
        }
    }
    if let Some(xid) = x11_place::get_active_window() {
        x11_place::minimize_window(xid);
    }
}

/// Gracefully close the focused window. X11 fallback when unreachable.
fn close_focused_window() {
    if let Some(id) = focused_daemon_window() {
        if WindowsProvider::from_env().close_window(id).is_ok() {
            return;
        }
    }
    if let Some(xid) = x11_place::get_active_window() {
        x11_place::close_window(xid);
    }
}

/// Toggle fullscreen (Zoom) on the focused window.
/// The daemon has no fullscreen getter, so zoomed ids are tracked locally.
/// X11 fallback toggles via EWMH directly.
fn toggle_zoom_focused_window() {
    if let Some(id) = focused_daemon_window() {
        let provider = WindowsProvider::from_env();
        let zoomed = ZOOMED_WINDOWS.with(|s| s.borrow().contains(&id));
        if provider.set_fullscreen(id, !zoomed).is_ok() {
            ZOOMED_WINDOWS.with(|s| {
                let mut set = s.borrow_mut();
                if zoomed {
                    set.remove(&id);
                } else {
                    set.insert(id);
                }
            });
            return;
        }
    }
    if let Some(xid) = x11_place::get_active_window() {
        x11_place::toggle_fullscreen(xid);
    }
}

/// Force quit the selected app (SIGKILL, no save dialog).
/// Never touches the menubar or dock themselves.
fn force_quit_current_app() {
    let Some(xid) = x11_place::get_active_window() else {
        return;
    };
    if let Some(cls) = x11_place::get_wm_class(xid) {
        let lower = cls.to_lowercase();
        if lower == "menubar" || lower == "dock" {
            return;
        }
    }
    if let Some(pid) = x11_place::get_window_pid(xid) {
        match CoreWindows::force_quit_pid(pid) {
            Ok(()) => println!("[menubar] force quit pid {pid}"),
            Err(e) => eprintln!("[menubar] force quit pid {pid} failed: {e}"),
        }
    }
}

fn set_octopus_menu_content(popover: &gtk::Popover) {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.add_css_class("octopus-menu");
    menu.set_halign(gtk::Align::Start);
    menu.set_valign(gtk::Align::Start);
    let is_dark = ColorScheme::detect_system() == ColorScheme::Dark;
    let icon_col = if is_dark { IconColor::WHITE } else { IconColor::from_rgb(30, 30, 30) };
    if is_dark {
        menu.remove_css_class("light");
    } else {
        menu.add_css_class("light");
    }

    struct Entry {
        label: String,
        icon: Option<SFSymbol>,
        shortcut: Option<&'static str>,
        arrow: bool,
        sep_before: bool,
        action: OctopusAction,
    }

    #[derive(Clone, Copy)]
    enum OctopusAction {
        None,
        OpenSystemOverview,
        OpenSystemSettings,
        ForceQuitActive,
    }

    let user = std::env::var("USER").unwrap_or_else(|_| "liveuser".to_string());
    // With no real app selected (bar shows "Finder"), Force Quit is disabled.
    let quit_enabled = bar_app_name() != "Finder";
    let entries: Vec<Entry> = vec![
        Entry { label: trk("menu.about"), icon: Some(CoreIcon::DESKTOPCOMPUTER), shortcut: None, arrow: false, sep_before: false, action: OctopusAction::OpenSystemOverview },
        Entry { label: trk("menu.settings"), icon: Some(CoreIcon::GEARSHAPE_FILL), shortcut: None, arrow: false, sep_before: false, action: OctopusAction::OpenSystemSettings },
        Entry { label: trk("menu.appstore"), icon: Some(CoreIcon::APP_FILL), shortcut: None, arrow: false, sep_before: false, action: OctopusAction::None },
        Entry { label: trk("menu.recent"), icon: Some(CoreIcon::CLOCK_FILL), shortcut: None, arrow: true, sep_before: false, action: OctopusAction::None },
        Entry { label: trk("menu.forcequit"), icon: Some(CoreIcon::XMARK_OCTAGON_FILL), shortcut: Some("⌥⌘⎋"), arrow: false, sep_before: true, action: OctopusAction::ForceQuitActive },
        Entry { label: trk("menu.sleep"), icon: Some(CoreIcon::MOON_FILL), shortcut: None, arrow: false, sep_before: true, action: OctopusAction::None },
        Entry { label: trk("menu.restart"), icon: Some(CoreIcon::ARROW_COUNTERCLOCKWISE), shortcut: None, arrow: false, sep_before: false, action: OctopusAction::None },
        Entry { label: trk("menu.shutdown"), icon: Some(CoreIcon::POWER), shortcut: None, arrow: false, sep_before: false, action: OctopusAction::None },
        Entry { label: trk("menu.lock"), icon: Some(CoreIcon::LOCK_FILL), shortcut: Some("^⌘Q"), arrow: false, sep_before: true, action: OctopusAction::None },
        Entry { label: trk_args("menu.logout", &[("user", &user)]), icon: Some(CoreIcon::RECTANGLE_PORTRAIT_AND_ARROW_RIGHT), shortcut: Some("⇧⌘Q"), arrow: false, sep_before: false, action: OctopusAction::None },
    ];

    for e in entries {
        if e.sep_before {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.add_css_class("octopus-sep");
            menu.append(&sep);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("octopus-menu-item");
        row.set_halign(gtk::Align::Fill);
        row.set_hexpand(true);
        // Force Quit is disabled with no real app selected (grayed out).
        let row_enabled = !matches!(e.action, OctopusAction::ForceQuitActive) || quit_enabled;
        if !row_enabled {
            row.set_sensitive(false);
        }
        // Hover-Effekt via CSS :hover — zusätzlich Motion-Controller für Klasse
        let motion = gtk::EventControllerMotion::new();
        let row_c = row.clone();
        motion.connect_enter(move |_, _, _| {
            if row_c.is_sensitive() {
                row_c.add_css_class("hovered");
            }
        });
        let row_c2 = row.clone();
        motion.connect_leave(move |_| row_c2.remove_css_class("hovered"));
        row.add_controller(motion);

        if let Some(sym) = e.icon {
            let icon_w = menu_sf_icon(sym, icon_col);
            icon_w.set_margin_start(4);
            row.append(&icon_w);
        } else {
            let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            sp.set_size_request(16, 16);
            row.append(&sp);
        }

        let lbl = gtk::Label::new(Some(e.label.as_str()));
        lbl.add_css_class("octopus-menu-label");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        row.append(&lbl);

        if let Some(sc) = e.shortcut {
            let sc_lbl = gtk::Label::new(Some(sc));
            sc_lbl.add_css_class("octopus-menu-shortcut");
            sc_lbl.set_halign(gtk::Align::End);
            row.append(&sc_lbl);
        }
        if e.arrow {
            let arrow = gtk::Label::new(Some("›"));
            arrow.add_css_class("octopus-menu-shortcut");
            arrow.set_margin_start(8);
            row.append(&arrow);
        }

        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let act = e.action;
        let pop_c = popover.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            if !row_enabled {
                return;
            }
            match act {
                OctopusAction::OpenSystemOverview => {
                    launch_system_overview();
                    pop_c.popdown();
                }
                OctopusAction::OpenSystemSettings => {
                    launch_via_tapp("/System/Applications/systemsettings.app", &[]);
                    pop_c.popdown();
                }
                OctopusAction::ForceQuitActive => {
                    force_quit_current_app();
                    pop_c.popdown();
                }
                OctopusAction::None => {}
            }
        });
        row.add_controller(gesture);

        menu.append(&row);
    }

    popover.set_child(Some(&menu));
}

fn build_app_menu(parent: &impl IsA<gtk::Widget>, app_name: &str) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_autohide(true);
    popover.set_parent(parent);
    popover.set_offset(0, 6);
    set_app_menu_content(&popover, app_name);
    popover
}

fn set_app_menu_content(popover: &gtk::Popover, app_name: &str) {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.add_css_class("octopus-menu");
    menu.set_halign(gtk::Align::Start);
    menu.set_valign(gtk::Align::Start);
    let is_dark = ColorScheme::detect_system() == ColorScheme::Dark;
    let icon_col = if is_dark { IconColor::WHITE } else { IconColor::from_rgb(30, 30, 30) };
    if is_dark {
        menu.remove_css_class("light");
    } else {
        menu.add_css_class("light");
    }

    struct Entry {
        label: String,
        icon: Option<SFSymbol>,
        action: AppAction,
    }
    #[derive(Clone, Copy)]
    enum AppAction { HideCurrent, HideOthers, AboutApp, Quit }

    let entries: Vec<Entry> = vec![
        Entry { label: trk_args("menu.hide_app", &[("app", app_name)]), icon: Some(CoreIcon::EYE_SLASH_FILL), action: AppAction::HideCurrent },
        Entry { label: trk("menu.hide_others"), icon: Some(CoreIcon::EYE_SLASH), action: AppAction::HideOthers },
        // separator vor About/Quit (same category)
        Entry { label: trk_args("menu.about_app", &[("app", app_name)]), icon: Some(CoreIcon::INFO_CIRCLE), action: AppAction::AboutApp },
        Entry { label: trk_args("menu.quit_app", &[("app", app_name)]), icon: Some(CoreIcon::XMARK), action: AppAction::Quit },
    ];

    for (idx, e) in entries.iter().enumerate() {
        if idx == 2 {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.add_css_class("octopus-sep");
            menu.append(&sep);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("octopus-menu-item");
        row.set_halign(gtk::Align::Fill);
        row.set_hexpand(true);
        // Quit is disabled with no real app selected (grayed out).
        let row_enabled = !matches!(e.action, AppAction::Quit) || app_name != "Finder";
        if !row_enabled {
            row.set_sensitive(false);
        }
        let motion = gtk::EventControllerMotion::new();
        let row_c = row.clone();
        motion.connect_enter(move |_, _, _| {
            if row_c.is_sensitive() {
                row_c.add_css_class("hovered");
            }
        });
        let row_c2 = row.clone();
        motion.connect_leave(move |_| row_c2.remove_css_class("hovered"));
        row.add_controller(motion);

        if let Some(sym) = e.icon {
            let icon_w = menu_sf_icon(sym, icon_col);
            icon_w.set_margin_start(4);
            row.append(&icon_w);
        } else {
            let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            sp.set_size_request(16, 16);
            row.append(&sp);
        }

        let lbl = gtk::Label::new(Some(&e.label));
        lbl.add_css_class("octopus-menu-label");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        row.append(&lbl);

        let act = e.action;
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let pop_clone = popover.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            if !row_enabled {
                return;
            }
            match act {
                AppAction::HideCurrent => hide_current_app(),
                AppAction::HideOthers => hide_other_apps(),
                AppAction::AboutApp => open_about_this_app(),
                AppAction::Quit => quit_current_app(),
            }
            pop_clone.popdown();
        });
        row.add_controller(gesture);
        menu.append(&row);
    }

    popover.set_child(Some(&menu));
}

// ── Top-level menu registry (click toggle + macOS hover-switch) ──────
/// Refresh a top-level menu before showing it.
fn refresh_top_menu(popover: &gtk::Popover, kind: TopMenuKind) {
    match kind {
        TopMenuKind::Octopus => set_octopus_menu_content(popover),
        TopMenuKind::App => {
            set_app_menu_content(popover, &bar_app_name());
        }
        TopMenuKind::Example(title) => set_example_menu_content(popover, title),
    }
}

/// Close every open top-level menu except `except` (if given).
fn close_other_top_menus(except: Option<&gtk::Popover>) {
    TOP_MENUS.with(|m| {
        for (w, p, _) in m.borrow().iter() {
            let is_except = except.map(|e| p == e).unwrap_or(false);
            if !is_except && p.is_visible() {
                p.popdown();
                w.remove_css_class("octopus-active");
            }
        }
    });
}

/// Wire a menubar trigger: click toggles its popover, hovering it while
/// another top-level menu is open switches to it (macOS behavior).
fn register_top_menu(wrap: &gtk::Box, popover: &gtk::Popover, kind: TopMenuKind) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let pop_c = popover.clone();
    let wrap_c = wrap.clone();
    click.connect_pressed(move |_, _, _, _| {
        if pop_c.is_visible() {
            pop_c.popdown();
            wrap_c.remove_css_class("octopus-active");
        } else {
            close_other_top_menus(Some(&pop_c));
            refresh_top_menu(&pop_c, kind);
            pop_c.popup();
            wrap_c.add_css_class("octopus-active");
        }
    });
    wrap.add_controller(click);
    let wrap_closed = wrap.clone();
    popover.connect_closed(move |_| wrap_closed.remove_css_class("octopus-active"));

    let pop_h = popover.clone();
    let wrap_h = wrap.clone();
    let switcher = gtk::EventControllerMotion::new();
    switcher.connect_enter(move |_, _, _| {
        let other_open = TOP_MENUS.with(|m| {
            m.borrow()
                .iter()
                .any(|(_, p, _)| p != &pop_h && p.is_visible())
        });
        if other_open && !pop_h.is_visible() {
            close_other_top_menus(Some(&pop_h));
            refresh_top_menu(&pop_h, kind);
            pop_h.popup();
            wrap_h.add_css_class("octopus-active");
        }
    });
    wrap.add_controller(switcher);

    TOP_MENUS.with(|m| m.borrow_mut().push((wrap.clone(), popover.clone(), kind)));
}

// ── Example menus (static placeholders, macOS-style) ───────────────────
fn build_example_menu(parent: &impl IsA<gtk::Widget>, title: &'static str) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_autohide(true);
    popover.set_parent(parent);
    popover.set_offset(0, 6);
    set_example_menu_content(&popover, title);
    popover
}

fn set_example_menu_content(popover: &gtk::Popover, menu_key: &str) {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.add_css_class("octopus-menu");
    menu.set_halign(gtk::Align::Start);
    menu.set_valign(gtk::Align::Start);
    let is_dark = ColorScheme::detect_system() == ColorScheme::Dark;
    let icon_col = if is_dark {
        IconColor::WHITE
    } else {
        IconColor::from_rgb(30, 30, 30)
    };
    if is_dark {
        menu.remove_css_class("light");
    } else {
        menu.add_css_class("light");
    }

    struct ExampleRow {
        key: &'static str,
        icon: Option<SFSymbol>,
        arrow: bool,
        sep_before: bool,
        action: ExampleAction,
    }

    #[derive(Clone, Copy)]
    enum ExampleAction {
        None,
        MinimizeWindow,
        ToggleZoom,
        CloseWindow,
    }

    // Per-menu rows (macOS-style), no shortcuts shown.
    let rows: Vec<ExampleRow> = match menu_key {
        "menu.edit" => vec![
            ExampleRow { key: "menu.edit_undo", icon: Some(CoreIcon::ARROW_UTURN_BACKWARD), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.edit_redo", icon: Some(CoreIcon::ARROW_UTURN_FORWARD), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.edit_cut", icon: Some(CoreIcon::SCISSORS), arrow: false, sep_before: true, action: ExampleAction::None },
            ExampleRow { key: "menu.edit_copy", icon: Some(CoreIcon::DOC_ON_DOC), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.edit_paste", icon: Some(CoreIcon::DOC_ON_CLIPBOARD), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.edit_selectall", icon: Some(CoreIcon::SQUARE_DASHED), arrow: false, sep_before: true, action: ExampleAction::None },
        ],
        "menu.view" => vec![
            ExampleRow { key: "menu.view_zoomin", icon: Some(CoreIcon::PLUS_MAGNIFYINGGLASS), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.view_zoomout", icon: Some(CoreIcon::MINUS_MAGNIFYINGGLASS), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.view_refresh", icon: Some(CoreIcon::ARROW_CLOCKWISE), arrow: false, sep_before: false, action: ExampleAction::None },
        ],
        "menu.go" => vec![
            ExampleRow { key: "menu.go_back", icon: Some(CoreIcon::CHEVRON_BACKWARD), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_forward", icon: Some(CoreIcon::CHEVRON_FORWARD), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_recents", icon: Some(CoreIcon::CLOCK), arrow: false, sep_before: true, action: ExampleAction::None },
            ExampleRow { key: "menu.go_documents", icon: Some(CoreIcon::FOLDER), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_desktop", icon: Some(CoreIcon::FOLDER), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_downloads", icon: Some(CoreIcon::FOLDER), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_home", icon: Some(CoreIcon::HOUSE), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_computer", icon: Some(CoreIcon::DESKTOPCOMPUTER), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_applications", icon: Some(CoreIcon::FOLDER), arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_recentfolders", icon: Some(CoreIcon::FOLDER_FILL), arrow: true, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.go_tofolder", icon: Some(CoreIcon::FOLDER_FILL), arrow: false, sep_before: true, action: ExampleAction::None },
        ],
        "menu.window" => vec![
            ExampleRow { key: "menu.win_minimize", icon: Some(CoreIcon::MINUS), arrow: false, sep_before: false, action: ExampleAction::MinimizeWindow },
            ExampleRow { key: "menu.win_zoom", icon: Some(CoreIcon::ARROW_UP_LEFT_AND_ARROW_DOWN_RIGHT), arrow: false, sep_before: false, action: ExampleAction::ToggleZoom },
            ExampleRow { key: "menu.win_close", icon: Some(CoreIcon::XMARK), arrow: false, sep_before: false, action: ExampleAction::CloseWindow },
            ExampleRow { key: "menu.win_front", icon: Some(CoreIcon::SQUARE_STACK), arrow: false, sep_before: true, action: ExampleAction::None },
        ],
        _ => vec![
            ExampleRow { key: "menu.example1", icon: None, arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.example2", icon: None, arrow: false, sep_before: false, action: ExampleAction::None },
            ExampleRow { key: "menu.example3", icon: None, arrow: false, sep_before: true, action: ExampleAction::None },
        ],
    };

    for e in rows {
        if e.sep_before {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.add_css_class("octopus-sep");
            menu.append(&sep);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("octopus-menu-item");
        row.set_halign(gtk::Align::Fill);
        row.set_hexpand(true);
        let motion = gtk::EventControllerMotion::new();
        let row_c = row.clone();
        motion.connect_enter(move |_, _, _| row_c.add_css_class("hovered"));
        let row_c2 = row.clone();
        motion.connect_leave(move |_| row_c2.remove_css_class("hovered"));
        row.add_controller(motion);

        if let Some(sym) = e.icon {
            let icon_w = menu_sf_icon(sym, icon_col);
            icon_w.set_margin_start(4);
            row.append(&icon_w);
        } else {
            let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            sp.set_size_request(16, 16);
            row.append(&sp);
        }

        let lbl = gtk::Label::new(Some(&trk(e.key)));
        lbl.add_css_class("octopus-menu-label");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        row.append(&lbl);

        if e.arrow {
            let arrow = gtk::Label::new(Some("›"));
            arrow.add_css_class("octopus-menu-shortcut");
            arrow.set_margin_start(8);
            row.append(&arrow);
        }

        let act = e.action;
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let pop_c = popover.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            match act {
                ExampleAction::MinimizeWindow => minimize_focused_window(),
                ExampleAction::ToggleZoom => toggle_zoom_focused_window(),
                ExampleAction::CloseWindow => close_focused_window(),
                ExampleAction::None => {}
            }
            pop_c.popdown();
        });
        row.add_controller(gesture);

        menu.append(&row);
    }

    popover.set_child(Some(&menu));
}

// ── Clock (example status area) ─────────────────────────────────────────
/// Refresh the clock label via `date` (format like "Mon Jun 23 7:29 PM").
fn update_clock(label: &gtk::Label) {
    let text = std::process::Command::new("date")
        .arg("+%a %b %e %-I:%M %p")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "--".to_string());
    label.set_text(&text);
}

// ── Bar Content (Top-to-Bottom Shadow, App-Name bold dicker) ───────────
fn build_bar_content(app_name: &str, scheme: ColorScheme, bar_height: i32) -> gtk::Widget {
    let is_dark = scheme == ColorScheme::Dark;
    let fg = if is_dark { "#F5F5F7" } else { "#1E1E1E" };
    // Nur oben Shadow für Lesbarkeit auf hellem Background (weiß auf hell)
    let text_shadow = if is_dark {
        "0 1px 4px rgba(0,0,0,0.85), 0 1px 1px rgba(0,0,0,0.70), 0 0 6px rgba(0,0,0,0.40)"
    } else {
        "0 1px 3px rgba(255,255,255,0.95), 0 0 5px rgba(255,255,255,0.80)"
    };

    let base_icon_tmp = (bar_height - 2).clamp(20, 40);
    let icon_h_tmp = ((base_icon_tmp as f32 * 1.15).round() as i32).clamp(22, 48);
    let win_h_tmp = bar_height.max(icon_h_tmp + 4);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("menubar-bar");
    bar.set_hexpand(true);
    bar.set_vexpand(false);
    bar.set_valign(gtk::Align::Center);
    bar.set_halign(gtk::Align::Fill);
    bar.set_height_request(win_h_tmp);
    bar.set_overflow(gtk::Overflow::Visible);

    let octopus = build_octopus_widget(bar_height);
    let octopus_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    octopus_wrap.add_css_class("octopus-wrap");
    octopus_wrap.set_valign(gtk::Align::Center);
    octopus_wrap.set_halign(gtk::Align::Start);
    octopus_wrap.set_hexpand(false);
    octopus_wrap.set_margin_start(6);
    octopus_wrap.append(&octopus);

    // Hover merkt man: heller Hintergrund
    let motion = gtk::EventControllerMotion::new();
    let wrap_h = octopus_wrap.clone();
    motion.connect_enter(move |_, _, _| wrap_h.add_css_class("octopus-hover"));
    let wrap_h2 = octopus_wrap.clone();
    motion.connect_leave(move |_| wrap_h2.remove_css_class("octopus-hover"));
    octopus_wrap.add_controller(motion);

    let popover = build_octopus_menu(&octopus_wrap);
    register_top_menu(&octopus_wrap, &popover, TopMenuKind::Octopus);

    let app_label = gtk::Label::new(Some(app_name));
    app_label.add_css_class("menubar-label");
    app_label.add_css_class("app-name-label");
    app_label.set_halign(gtk::Align::Start);
    app_label.set_valign(gtk::Align::Center);
    app_label.set_xalign(0.0);

    let app_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    app_wrap.add_css_class("octopus-wrap");
    app_wrap.set_valign(gtk::Align::Center);
    app_wrap.set_halign(gtk::Align::Start);
    app_wrap.set_hexpand(false);
    app_wrap.set_margin_start(2);
    app_wrap.set_margin_end(14);
    app_wrap.append(&app_label);
    let motion2 = gtk::EventControllerMotion::new();
    let wrap_a = app_wrap.clone();
    motion2.connect_enter(move |_, _, _| wrap_a.add_css_class("octopus-hover"));
    let wrap_a2 = app_wrap.clone();
    motion2.connect_leave(move |_| wrap_a2.remove_css_class("octopus-hover"));
    app_wrap.add_controller(motion2);
    let app_popover = build_app_menu(&app_wrap, app_name);
    register_top_menu(&app_wrap, &app_popover, TopMenuKind::App);
    APP_MENUBAR_STATE.with(|s| s.borrow_mut().push((app_label.clone(), app_popover.clone())));

    let css = format!(
        ".menubar-label {{ font-family: '{}', 'SF Pro', sans-serif; font-size: 13px; font-weight: 400; color: {}; text-shadow: {}; }}\n.app-name-label {{ font-weight: 800; font-family: '{}', 'SF Pro Display', sans-serif; letter-spacing: -0.2px; }}\n.menubar-bar {{ background: linear-gradient(to bottom, rgba(0,0,0,0.24) 0%, rgba(0,0,0,0.15) 30%, rgba(0,0,0,0) 60%, rgba(0,0,0,0) 100%); background-color: transparent; min-height: {}px; box-shadow: none; border: none; }}",
        SF_FAMILY, fg, text_shadow, SF_FAMILY, win_h_tmp
    );
    UIKit::widget::apply_css(&bar, &css);
    let label_css = format!(".menubar-label {{ color: {}; }} .app-name-label {{ color: {}; }}", fg, fg);
    UIKit::widget::apply_css(&app_label, &label_css);

    bar.append(&octopus_wrap);
    bar.append(&app_wrap);

    // Example app menus (static placeholders, macOS-style).
    for key in ["menu.file", "menu.edit", "menu.view", "menu.go", "menu.window", "menu.help"] {
        let lbl = gtk::Label::new(Some(&trk(key)));
        lbl.add_css_class("menubar-label");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_valign(gtk::Align::Center);
        lbl.set_xalign(0.0);
        let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        wrap.add_css_class("octopus-wrap");
        wrap.set_valign(gtk::Align::Center);
        wrap.set_halign(gtk::Align::Start);
        wrap.set_hexpand(false);
        wrap.set_margin_start(2);
        wrap.set_margin_end(2);
        wrap.append(&lbl);
        let motion = gtk::EventControllerMotion::new();
        let w_h = wrap.clone();
        motion.connect_enter(move |_, _, _| w_h.add_css_class("octopus-hover"));
        let w_h2 = wrap.clone();
        motion.connect_leave(move |_| w_h2.remove_css_class("octopus-hover"));
        wrap.add_controller(motion);
        // Help has no menu yet: title with hover only.
        if key != "menu.help" {
            let pop = build_example_menu(&wrap, key);
            register_top_menu(&wrap, &pop, TopMenuKind::Example(key));
        }
        bar.append(&wrap);
    }

    // Spacer pushes the status area to the right edge.
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);

    // Example status area (macOS-style): icons + clock.
    // Status icons are always white: the top shadow keeps them readable.
    let icon_col = IconColor::WHITE;
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status.set_valign(gtk::Align::Center);
    status.set_halign(gtk::Align::End);
    status.set_margin_end(10);
    for sym in [
        CoreIcon::SWITCH_2,
        CoreIcon::MOON_FILL,
        CoreIcon::BATTERY_100,
        CoreIcon::WIFI,
        CoreIcon::MAGNIFYINGGLASS,
    ] {
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cell.set_valign(gtk::Align::Center);
        cell.append(&menu_sf_icon(sym, icon_col));
        status.append(&cell);
    }
    let clock = gtk::Label::new(Some("--"));
    clock.add_css_class("menubar-label");
    clock.set_halign(gtk::Align::End);
    clock.set_valign(gtk::Align::Center);
    clock.set_xalign(1.0);
    status.append(&clock);
    bar.append(&status);
    update_clock(&clock);
    let clock_c = clock.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(20), move || {
        update_clock(&clock_c);
        glib::ControlFlow::Continue
    });

    bar.upcast()
}

// ── X11 Platzierung (wie Dock) ────────────────────────────────────────
#[cfg(target_os = "linux")]
mod x11_place {
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    type Conn = x11rb::rust_connection::RustConnection;
    fn connection() -> Option<&'static (Conn, usize)> {
        static CONN: OnceLock<Option<(Conn, usize)>> = OnceLock::new();
        CONN.get_or_init(|| x11rb::connect(None).ok()).as_ref()
    }
    pub fn xid_of(surface: &gtk::gdk::Surface) -> Option<u32> {
        extern "C" { fn gdk_x11_surface_get_xid(surface: *const std::ffi::c_void) -> u32; }
        use glib::translate::{Stash, ToGlibPtr};
        let ptr: Stash<'_, *const gtk::gdk::ffi::GdkSurface, _> = surface.to_glib_none();
        Some(unsafe { gdk_x11_surface_get_xid(ptr.0.cast()) })
    }
    pub fn move_window(xid: u32, x: i32, y: i32) -> bool {
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _};
        let Some((conn, _)) = connection() else { return false; };
        conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y)).is_ok()
    }
    pub fn set_position_hints(xid: u32, x: i32, y: i32) -> bool {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
        let Some((conn, _)) = connection() else { return false; };
        const USPOSITION: i32 = 1 << 0;
        const PPOSITION: i32 = 1 << 1;
        let mut hints = match conn
            .get_property(false, xid, AtomEnum::WM_NORMAL_HINTS, AtomEnum::WM_SIZE_HINTS, 0, 18)
            .ok().and_then(|r| r.reply().ok())
        {
            Some(reply) if reply.value_len > 0 => {
                let mut v = [0i32; 18];
                for (i, chunk) in reply.value.chunks(4).take(18).enumerate() {
                    if chunk.len() == 4 { v[i] = i32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]); }
                }
                v
            }
            _ => [0i32; 18],
        };
        hints[0] |= USPOSITION | PPOSITION;
        hints[2] = x;
        hints[3] = y;
        let mut value = Vec::with_capacity(72);
        for v in hints { value.extend_from_slice(&v.to_ne_bytes()); }
        conn.change_property(PropMode::REPLACE, xid, AtomEnum::WM_NORMAL_HINTS, AtomEnum::WM_SIZE_HINTS, 32, 18, &value).is_ok()
    }
    pub fn set_dock_type(xid: u32) -> bool {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
        let Some((conn, _)) = connection() else { return false; };
        let type_atom = match conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let dock_atom = match conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE_DOCK").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let data = dock_atom.to_ne_bytes();
        // OS-Shadow komplett deaktivieren: _GTK_FRAME_EXTENTS = 0 und shadow hints.
        // Mutter/KWin zeichnen sonst einen eigenen Drop-Shadow um DOCK-Fenster.
        if let Some(atom) = conn.intern_atom(false, b"_GTK_FRAME_EXTENTS").ok().and_then(|c| c.reply().ok()).map(|r| r.atom) {
            let extents = [0u32, 0, 0, 0];
            let mut bytes = Vec::with_capacity(16);
            for v in extents { bytes.extend_from_slice(&v.to_ne_bytes()); }
            let _ = conn.change_property(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, 32, 4, &bytes);
        }
        // KWin-spezifisch: _KDE_NET_WM_SHADOW leeren
        if let Ok(cookie) = conn.intern_atom(false, b"_KDE_NET_WM_SHADOW") {
            if let Ok(reply) = cookie.reply() {
                let _ = conn.delete_property(xid, reply.atom);
            }
        }
        conn.change_property(PropMode::REPLACE, xid, type_atom, AtomEnum::ATOM, 32, 1, &data).is_ok()
    }
    /// OS-Shadow explizit deaktivieren (zusätzlich zu _GTK_FRAME_EXTENTS).
    /// Wird direkt nach dem Map aufgerufen.
    pub fn disable_shadow(xid: u32) -> bool {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
        let Some((conn, _)) = connection() else { return false; };
        let mut ok = true;
        if let Some(atom) = conn.intern_atom(false, b"_GTK_FRAME_EXTENTS").ok().and_then(|c| c.reply().ok()).map(|r| r.atom) {
            let extents = [0u32, 0, 0, 0];
            let mut bytes = Vec::with_capacity(16);
            for v in extents { bytes.extend_from_slice(&v.to_ne_bytes()); }
            ok &= conn.change_property(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, 32, 4, &bytes).is_ok();
        }
        ok
    }
    /// Never take keyboard focus: clicks/hover still work, but the app
    /// below keeps _NET_ACTIVE_WINDOW and keyboard focus (shell behavior).
    /// WM_HINTS = { flags=InputHint, input=False, ... }.
    pub fn set_no_focus(xid: u32) -> bool {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
        let Some((conn, _)) = connection() else { return false; };
        const INPUT_HINT: i32 = 1 << 0;
        const NORMAL_STATE: i32 = 1;
        let hints = [INPUT_HINT, 0, NORMAL_STATE, 0, 0, 0, 0, 0, 0];
        let mut value = Vec::with_capacity(36);
        for v in hints { value.extend_from_slice(&v.to_ne_bytes()); }
        conn.change_property(PropMode::REPLACE, xid, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 32, 9, &value).is_ok()
    }
    /// Input-Region auf die Bar beschränken (Schatten bleibt visuell, aber klick-transparent).
    pub fn set_input_region(xid: u32, x: i32, y: i32, w: u16, h: u16) -> bool {
        use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
        use x11rb::protocol::xproto;
        let Some((conn, _)) = connection() else { return false; };
        let rect = xproto::Rectangle { x: x as i16, y: y as i16, width: w, height: h };
        conn.shape_rectangles(SO::SET, SK::INPUT, xproto::ClipOrdering::UNSORTED, xid, 0, 0, std::slice::from_ref(&rect)).is_ok()
    }
    /// Keep window always on top
    pub fn set_keep_above(xid: u32) -> bool {
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
        let Some((conn, screen)) = connection() else { return false; };
        let root = conn.setup().roots[*screen].root;
        let state_atom = match conn.intern_atom(false, b"_NET_WM_STATE").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let above_atom = match conn.intern_atom(false, b"_NET_WM_STATE_ABOVE").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let sticky_atom = match conn.intern_atom(false, b"_NET_WM_STATE_STICKY").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let atoms = [above_atom, sticky_atom];
        let mut bytes = Vec::new();
        for a in atoms { bytes.extend_from_slice(&a.to_ne_bytes()); }
        let _ = conn.change_property(PropMode::REPLACE, xid, state_atom, AtomEnum::ATOM, 32, 2, &bytes);
        let _ = root;
        true
    }

    // ── App-Name & Window-Management (selected app) ──────────────────────
    pub fn get_active_window() -> Option<u32> {
        let (conn, screen) = connection()?;
        let root = conn.setup().roots[*screen].root;
        let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").ok()?.reply().ok()?.atom;
        let prop = conn.get_property(false, root, atom, x11rb::protocol::xproto::AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
        if prop.value.len() >= 4 {
            let xid = u32::from_ne_bytes([prop.value[0], prop.value[1], prop.value[2], prop.value[3]]);
            if xid != 0 { return Some(xid); }
        }
        None
    }

    pub fn get_window_pid(xid: u32) -> Option<i32> {
        let (conn, _) = connection()?;
        let atom = conn.intern_atom(false, b"_NET_WM_PID").ok()?.reply().ok()?.atom;
        let prop = conn.get_property(false, xid, atom, x11rb::protocol::xproto::AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
        if prop.value.len() >= 4 {
            Some(i32::from_ne_bytes([prop.value[0], prop.value[1], prop.value[2], prop.value[3]]))
        } else {
            None
        }
    }

    pub fn get_client_list() -> Vec<u32> {
        let Some((conn, screen)) = connection() else { return Vec::new(); };
        let root = conn.setup().roots[*screen].root;
        let atom = match conn.intern_atom(false, b"_NET_CLIENT_LIST").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return Vec::new() };
        let prop = match conn.get_property(false, root, atom, x11rb::protocol::xproto::AtomEnum::WINDOW, 0, 4096).ok().and_then(|c| c.reply().ok()) { Some(p) => p, None => return Vec::new() };
        if prop.value.is_empty() { return Vec::new(); }
        prop.value.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).filter(|&x| x != 0).collect()
    }

    pub fn get_wm_class(xid: u32) -> Option<String> {
        let (conn, _) = connection()?;
        let atom = conn.intern_atom(false, b"WM_CLASS").ok()?.reply().ok()?.atom;
        let prop = conn.get_property(false, xid, atom, x11rb::protocol::xproto::AtomEnum::STRING, 0, 1024).ok()?.reply().ok()?;
        if prop.format != 8 || prop.value.is_empty() { return None; }
        let parts: Vec<&[u8]> = prop.value.split(|&b| b == 0).collect();
        // WM_CLASS = instance + '\0' + class + '\0'
        let class = parts.get(1).and_then(|c| if c.is_empty() { None } else { Some(*c) })
            .or_else(|| parts.first().copied())
            .map(|c| String::from_utf8_lossy(c).trim().to_string())
            .filter(|s| !s.is_empty())?;
        Some(class)
    }

    pub fn get_active_app_name() -> String {
        if let Some(xid) = get_active_window() {
            if let Some(cls) = get_wm_class(xid) {
                // Foot → Foot (capitalize first letter, keep rest)
                let mut chars = cls.chars();
                if let Some(first) = chars.next() {
                    let rest: String = chars.collect();
                    // special: lower "foot" -> "Foot"
                    let cap = first.to_uppercase().collect::<String>() + &rest;
                    // Filter out menubar itself if it becomes active (should not)
                    if cap.to_lowercase() != "menubar" && cap.to_lowercase() != "dock" {
                        return cap;
                    }
                    return "Finder".to_string();
                }
                return cls;
            }
            // Fallback: _NET_WM_NAME
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
        if prop.value.is_empty() { return None; }
        let s = String::from_utf8_lossy(&prop.value).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    pub fn get_window_title(xid: u32) -> Option<String> {
        get_net_wm_name(xid)
    }

    /// Toggle fullscreen via EWMH (_NET_WM_STATE_TOGGLE).
    pub fn toggle_fullscreen(xid: u32) -> bool {
        let Some((conn, screen)) = connection() else { return false; };
        let root = conn.setup().roots[*screen].root;
        let wm_state = match conn.intern_atom(false, b"_NET_WM_STATE").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let fs = match conn.intern_atom(false, b"_NET_WM_STATE_FULLSCREEN").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        // data[0] = _NET_WM_STATE_TOGGLE (2), data[1] = fullscreen atom
        let data = x11rb::protocol::xproto::ClientMessageData::from([2u32, fs, 0, 0, 0]);
        let event = x11rb::protocol::xproto::ClientMessageEvent {
            response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
            sequence: 0,
            window: xid,
            type_: wm_state,
            format: 32,
            data,
        };
        let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
    }

    pub fn minimize_window(xid: u32) -> bool {
        let Some((conn, screen)) = connection() else { return false; };
        let root = conn.setup().roots[*screen].root;
        let wm_change = match conn.intern_atom(false, b"WM_CHANGE_STATE").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let data = x11rb::protocol::xproto::ClientMessageData::from([3u32, 0, 0, 0, 0]); // IconicState = 3
        let event = x11rb::protocol::xproto::ClientMessageEvent {
            response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
            sequence: 0,
            window: xid,
            type_: wm_change,
            format: 32,
            data,
        };
        let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
    }

    pub fn close_window(xid: u32) -> bool {
        let Some((conn, screen)) = connection() else { return false; };
        let root = conn.setup().roots[*screen].root;
        let net_close = match conn.intern_atom(false, b"_NET_CLOSE_WINDOW").ok().and_then(|c| c.reply().ok()) { Some(r) => r.atom, None => return false };
        let data = x11rb::protocol::xproto::ClientMessageData::from([x11rb::CURRENT_TIME, 2, 0, 0, 0]);
        let event = x11rb::protocol::xproto::ClientMessageEvent {
            response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
            sequence: 0,
            window: xid,
            type_: net_close,
            format: 32,
            data,
        };
        let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
    }
}

#[cfg(not(target_os = "linux"))]
mod x11_place {
    pub fn xid_of(_: &gtk::gdk::Surface) -> Option<u32> { None }
    pub fn move_window(_: u32, _: i32, _: i32) -> bool { false }
    pub fn set_position_hints(_: u32, _: i32, _: i32) -> bool { false }
    pub fn set_dock_type(_: u32) -> bool { false }
    pub fn set_keep_above(_: u32) -> bool { false }
    pub fn disable_shadow(_: u32) -> bool { false }
    pub fn set_no_focus(_: u32) -> bool { false }
    pub fn set_input_region(_: u32, _: i32, _: i32, _: u16, _: u16) -> bool { false }
    pub fn get_active_window() -> Option<u32> { None }
    pub fn get_window_pid(_: u32) -> Option<i32> { None }
    pub fn get_window_title(_: u32) -> Option<String> { None }
    pub fn get_client_list() -> Vec<u32> { Vec::new() }
    pub fn get_wm_class(_: u32) -> Option<String> { None }
    pub fn get_active_app_name() -> String { "Finder".to_string() }
    pub fn minimize_window(_: u32) -> bool { false }
    pub fn close_window(_: u32) -> bool { false }
    pub fn toggle_fullscreen(_: u32) -> bool { false }
}

// ── Per-Monitor Fenster ────────────────────────────────────────────────

/// Erzeuge fuer jeden Monitor ein eigenes, transparentes Fenster oben.
/// 1:1 Spiegelung — Octopus + selected App-Name (bold), Hover + Menüs.
fn spawn_bars(app: &Application) {
    let t0 = std::time::Instant::now();
    let app_name_initial = bar_app_name();
    eprintln!("[menubar] selected_app took {:?}", t0.elapsed());
    LAST_APP_NAME.with(|s| *s.borrow_mut() = app_name_initial.clone());
    APP_MENUBAR_STATE.with(|s| s.borrow_mut().clear());
    OCTOPUS_MENUS.with(|s| s.borrow_mut().clear());
    TOP_MENUS.with(|s| s.borrow_mut().clear());
    BAR_WINDOWS.with(|s| s.borrow_mut().clear());
    ZOOMED_WINDOWS.with(|s| s.borrow_mut().clear());
    let scheme = ColorScheme::detect_system();
    LAST_SCHEME.with(|s| *s.borrow_mut() = Some(scheme));

    let display = match gtk::gdk::Display::default() {
        Some(d) => d,
        None => {
            eprintln!("[menubar] no display");
            return;
        }
    };

    let monitors = display.monitors();
    let n = monitors.n_items();
    println!("[menubar] spawning {} bar(s), scheme={:?}, app='{}'", n.max(1), scheme, app_name_initial);
    eprintln!("[menubar] pre-spawn took {:?}", t0.elapsed());

    // Falle n == 0 (headless) -> ein Dummy-Fenster
    let count = if n == 0 { 1 } else { n };

    // Sammle Fenster damit sie nicht sofort gedroppt werden
    let windows: Rc<RefCell<Vec<ApplicationWindow>>> = Rc::new(RefCell::new(Vec::new()));

    for i in 0..count {
        let monitor = if n > 0 {
            monitors.item(i).and_then(|o| o.downcast::<gtk::gdk::Monitor>().ok())
        } else {
            None
        };

        let (mx, my, mw, _mh, scale) = if let Some(m) = &monitor {
            let g = m.geometry();
            let s = m.scale_factor().max(1);
            // geometry is in logical pixels; X11 move needs physical
            (g.x(), g.y(), g.width(), g.height(), s)
        } else {
            (0, 0, 1920, 1080, 1)
        };

        let logical_w = mw;
        let logical_h = bar_height_for_monitor(monitor.as_ref());
        let base_icon = (logical_h - 2).clamp(20, 40);
        let icon_h = ((base_icon as f32 * 1.15).round() as i32).clamp(22, 48);
        // Window muss Bar + Icon-Padding fassen (Icon ragt leicht heraus)
        let win_h = logical_h.max(icon_h + 4);

        let phys_x = mx * scale;
        let phys_y = my * scale;

        let win = ApplicationWindow::builder()
            .application(app)
            .title("TontooOS Menubar")
            .decorated(false)
            .resizable(false)
            .default_width(logical_w)
            .default_height(win_h)
            .build();

        win.set_decorated(false);
        win.set_resizable(false);
        win.set_overflow(gtk::Overflow::Visible);
        // Shell behavior: never steal keyboard focus from the app below,
        // pointer events (click/hover) keep working.
        win.set_focusable(false);

        let content = build_bar_content(&app_name_initial, scheme, logical_h);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("menubar-root");
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_valign(gtk::Align::Start);
        root.set_overflow(gtk::Overflow::Visible);
        root.append(&content);
        win.set_child(Some(&root));

        win.set_size_request(logical_w, win_h);

        // Wayland layer-shell: pin the bar to the top edge with an exclusive
        // zone. The compositor then places it at (0,0) full-width, keeps it
        // above normal windows and blends its alpha (translucency). On X11
        // (XWayland fallback) this is skipped and the X11 self-positioning
        // below takes over instead.
        let wayland_first = std::env::var("GDK_BACKEND")
            .map(|v| {
                v.split(',')
                    .next()
                    .map(|s| s.trim() == "wayland")
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if wayland_first {
            use gtk4_layer_shell::LayerShell;
            win.init_layer_shell();
            win.set_layer(gtk4_layer_shell::Layer::Top);
            win.set_anchor(gtk4_layer_shell::Edge::Top, true);
            win.set_anchor(gtk4_layer_shell::Edge::Left, true);
            win.set_anchor(gtk4_layer_shell::Edge::Right, true);
            win.set_anchor(gtk4_layer_shell::Edge::Bottom, false);
            win.set_exclusive_zone(win_h);
            win.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        }

        // Present zuerst, dann X11-Move (Surface existiert erst nach Realize)
        win.present();

        // OS-Shadow bleibt deaktiviert (kein eigener Shadow); Höhe per Monitor
        let win_clone = win.clone();
        let logical_w_c = logical_w;
        let logical_h_c = logical_h;
        glib::idle_add_local_once(move || {
            if let Some(surface) = win_clone.surface() {
                if let Some(xid) = x11_place::xid_of(&surface) {
                    x11_place::set_position_hints(xid, phys_x, phys_y);
                    x11_place::move_window(xid, phys_x, phys_y);
                    x11_place::set_dock_type(xid);
                    x11_place::disable_shadow(xid);
                    x11_place::set_no_focus(xid);
                    x11_place::set_keep_above(xid);
                    let scale = win_clone.scale_factor().max(1);
                    x11_place::set_input_region(xid, 0, 0, (logical_w_c * scale) as u16, (logical_h_c * scale) as u16);
                    let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, logical_w_c, logical_h_c));
                    surface.set_input_region(&region);
                }
            }
        });
        let win2 = win.clone();
        let logical_w2 = logical_w;
        let logical_h2 = logical_h;
        glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            if let Some(surface) = win2.surface() {
                if let Some(xid) = x11_place::xid_of(&surface) {
                    x11_place::move_window(xid, phys_x, phys_y);
                    x11_place::disable_shadow(xid);
                    x11_place::set_no_focus(xid);
                    let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, logical_w2, logical_h2));
                    surface.set_input_region(&region);
                }
            }
        });

        // Input region exakt auf Bar beschränken (falls später klick-transparent gewünscht)
        // Für Menubar bleibt volle Breite aktiv — keine Einschränkung nötig.

        println!("[menubar] bar {} -> monitor {}: pos=({},{}) bar={}x{} win={}x{} scale={} icon={}", i, i, mx, my, logical_w, logical_h, logical_w, win_h, scale, icon_h);
        windows.borrow_mut().push(win);
    }

    // Mirror bar windows for the pointer-over-bar check (sticky app name).
    BAR_WINDOWS.with(|b| *b.borrow_mut() = windows.borrow().clone());

    // Live-Update: selected App alle 400ms pollen (WM_CLASS, nicht Titel)
    {
        let last_cell = std::rc::Rc::new(std::cell::RefCell::new(app_name_initial.clone()));
        glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
            let cur = bar_app_name();
            let mut last = last_cell.borrow_mut();
            if *last != cur {
                *last = cur.clone();
                LAST_APP_NAME.with(|s| *s.borrow_mut() = cur.clone());
                APP_MENUBAR_STATE.with(|state| {
                    for (lbl, pop) in state.borrow().iter() {
                        lbl.set_text(&cur);
                        set_app_menu_content(pop, &cur);
                    }
                });
            }
            glib::ControlFlow::Continue
        });
    }

    // Theme Live-Update: Light/Dark -> Menüs weiß/schwarz neu einfärben
    {
        let last_scheme = std::rc::Rc::new(std::cell::RefCell::new(scheme));
        glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
            let cur = ColorScheme::detect_system();
            if *last_scheme.borrow() != cur {
                *last_scheme.borrow_mut() = cur;
                LAST_SCHEME.with(|s| *s.borrow_mut() = Some(cur));
                OCTOPUS_MENUS.with(|menus| {
                    for pop in menus.borrow().iter() {
                        set_octopus_menu_content(pop);
                    }
                });
                let cur_app = bar_app_name();
                APP_MENUBAR_STATE.with(|state| {
                    for (_, pop) in state.borrow().iter() {
                        set_app_menu_content(pop, &cur_app);
                    }
                });
            }
            glib::ControlFlow::Continue
        });
    }

    // Auf Monitor-Hotplug reagieren: neu spawnen
    let app_clone = app.clone();
    let windows_clone = windows.clone();
    // Leak prevention: keep handler alive
    let _ = windows;
    monitors.connect_items_changed(move |_, _, _, _| {
        // Bestehende Fenster schließen und neu aufbauen
        for w in windows_clone.borrow().iter() {
            w.close();
        }
        windows_clone.borrow_mut().clear();
        // Neu spawnen nach kurzer Pause
        let app2 = app_clone.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            spawn_bars(&app2);
        });
    });
}

fn main() {
    println!(
        "TontooOS Menubar v{}.{} (UIKit v{}.{}.{})",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        UITKIT_VERSION.0,
        UITKIT_VERSION.1,
        UITKIT_VERSION.2
    );

    // Wie Dock: X11 erzwingen damit self-positioning funktioniert.
    // TontooCompositor unter Wayland wird später layer-shell nutzen; XWayland fallback.
    if std::env::var("GDK_BACKEND").is_err() {
        std::env::set_var("GDK_BACKEND", "x11");
    }

    init_i18n();

    let app = Application::builder()
        .application_id("org.tontoo.menubar")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|_| {
        install_transparent_css();
    });

    app.connect_activate(|app| {
        spawn_bars(app);
    });

    // Aufnutzen: bereits laufende Instanz ersetzt sich nicht — mehrere Starts okay.
    app.run_with_args::<&str>(&[]);
}
