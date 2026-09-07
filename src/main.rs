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

// ── i18n ────────────────────────────────────────────────────────────────
/// Minimaler Lang-Loader: liest lang/en_us.json & lang/de_de.json.
/// Auswahl via $LANG / $LC_ALL (enthält "de" -> de_de, sonst en_us).
/// Fällt auf "Test" zurück wenn keine Datei vorhanden ist.
fn load_lang() -> std::collections::HashMap<String, String> {
    let lang_code = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_else(|_| "en_US".to_string())
        .to_lowercase();
    let use_de = lang_code.starts_with("de");

    let rel_paths = [
        // When running from cargo (dev): exe in target/debug/menubar -> ../../lang
        // When installed: /usr/bin/menubar -> /usr/share/tontoo/menubar/lang or ./lang
        format!(
            "{}/lang/{}",
            env!("CARGO_MANIFEST_DIR"),
            if use_de { "de_de.json" } else { "en_us.json" }
        ),
        format!("./lang/{}", if use_de { "de_de.json" } else { "en_us.json" }),
        format!("/usr/share/tontoo/menubar/lang/{}", if use_de { "de_de.json" } else { "en_us.json" }),
        // WSL absolute fallback
        format!(
            "/mnt/c/Users/arlo1/Documents/TontooProgramms/Menubar/lang/{}",
            if use_de { "de_de.json" } else { "en_us.json" }
        ),
    ];

    for p in &rel_paths {
        if let Ok(content) = std::fs::read_to_string(p) {
            if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(&content) {
                return map;
            }
        }
    }
    let mut fallback = std::collections::HashMap::new();
    fallback.insert("menubar.test".to_string(), "Test".to_string());
    fallback
}

fn tr(map: &std::collections::HashMap<String, String>, key: &str) -> String {
    map.get(key).cloned().unwrap_or_else(|| key.to_string())
}

// ── Transparent CSS + OS Shadow OFF (Shadow vorerst entfernt) ─────────
fn install_transparent_css() {
    if let Some(display) = gtk::gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        // Window vollständig transparent, nur Octopus + Label zeichnen.
        // SF Pro Fonts liegen im ISO unter /usr/share/fonts/OTF + /TTF.
        // OS-Compositor-Shadow deaktiviert, eigener Shadow aktuell aus.
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
                background-color: transparent;
                background: transparent;
            }}
            .menubar-bar {{
                background-color: transparent;
                background: transparent;
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
        label: &'static str,
        icon: Option<SFSymbol>,
        shortcut: Option<&'static str>,
        arrow: bool,
        sep_before: bool,
    }

    let entries: &[Entry] = &[
        Entry { label: "About This Maschine", icon: Some(CoreIcon::DESKTOPCOMPUTER), shortcut: None, arrow: false, sep_before: false },
        Entry { label: "System Settings…", icon: Some(CoreIcon::GEARSHAPE_FILL), shortcut: None, arrow: false, sep_before: false },
        Entry { label: "App Store…", icon: Some(CoreIcon::APP_FILL), shortcut: None, arrow: false, sep_before: false },
        Entry { label: "Recent Items", icon: Some(CoreIcon::CLOCK_FILL), shortcut: None, arrow: true, sep_before: false },
        Entry { label: "Force Quit…", icon: Some(CoreIcon::XMARK_OCTAGON_FILL), shortcut: Some("⌥⌘⎋"), arrow: false, sep_before: true },
        Entry { label: "Sleep", icon: Some(CoreIcon::MOON_FILL), shortcut: None, arrow: false, sep_before: true },
        Entry { label: "Restart…", icon: Some(CoreIcon::ARROW_COUNTERCLOCKWISE), shortcut: None, arrow: false, sep_before: false },
        Entry { label: "Shut Down…", icon: Some(CoreIcon::POWER), shortcut: None, arrow: false, sep_before: false },
        Entry { label: "Lock Screen", icon: Some(CoreIcon::LOCK_FILL), shortcut: Some("^⌘Q"), arrow: false, sep_before: true },
        Entry { label: "Log Out liveuser…", icon: Some(CoreIcon::RECTANGLE_PORTRAIT_AND_ARROW_RIGHT), shortcut: Some("⇧⌘Q"), arrow: false, sep_before: false },
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
        // Hover-Effekt via CSS :hover — zusätzlich Motion-Controller für Klasse
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

        let lbl = gtk::Label::new(Some(e.label));
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
        gesture.connect_pressed(|_, _, _, _| {});
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
    enum AppAction { HideCurrent, HideOthers, Quit }

    let entries: Vec<Entry> = vec![
        Entry { label: format!("Hide {}", app_name), icon: Some(CoreIcon::EYE_SLASH_FILL), action: AppAction::HideCurrent },
        Entry { label: "Hide Others".to_string(), icon: Some(CoreIcon::EYE_SLASH), action: AppAction::HideOthers },
        // separator vor Quit
        Entry { label: format!("Quit {}", app_name), icon: Some(CoreIcon::XMARK), action: AppAction::Quit },
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
            match act {
                AppAction::HideCurrent => {
                    if let Some(xid) = x11_place::get_active_window() {
                        x11_place::minimize_window(xid);
                    }
                }
                AppAction::HideOthers => {
                    let active = x11_place::get_active_window();
                    for xid in x11_place::get_client_list() {
                        if Some(xid) != active {
                            // Menubar/Dock selbst nicht verstecken (sind dock-type, meist nicht in Liste)
                            x11_place::minimize_window(xid);
                        }
                    }
                }
                AppAction::Quit => {
                    if let Some(xid) = x11_place::get_active_window() {
                        x11_place::close_window(xid);
                    }
                }
            }
            pop_clone.popdown();
        });
        row.add_controller(gesture);
        menu.append(&row);
    }

    popover.set_child(Some(&menu));
}

// ── Bar Content (Shadow nur oben, App-Name bold dicker) ───────────────
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
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let pop_c = popover.clone();
    let wrap_c = octopus_wrap.clone();
    click.connect_pressed(move |_, _, _, _| {
        if pop_c.is_visible() {
            pop_c.popdown();
            wrap_c.remove_css_class("octopus-active");
        } else {
            set_octopus_menu_content(&pop_c);
            pop_c.popup();
            wrap_c.add_css_class("octopus-active");
        }
    });
    octopus_wrap.add_controller(click);
    let wrap_closed = octopus_wrap.clone();
    popover.connect_closed(move |_| wrap_closed.remove_css_class("octopus-active"));

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
    let click2 = gtk::GestureClick::new();
    click2.set_button(1);
    let pop2 = app_popover.clone();
    let wrap2 = app_wrap.clone();
    click2.connect_pressed(move |_, _, _, _| {
        if pop2.is_visible() {
            pop2.popdown();
            wrap2.remove_css_class("octopus-active");
        } else {
            let cur = x11_place::get_active_app_name();
            set_app_menu_content(&pop2, &cur);
            pop2.popup();
            wrap2.add_css_class("octopus-active");
        }
    });
    app_wrap.add_controller(click2);
    let wrap_closed2 = app_wrap.clone();
    app_popover.connect_closed(move |_| wrap_closed2.remove_css_class("octopus-active"));
    APP_MENUBAR_STATE.with(|s| s.borrow_mut().push((app_label.clone(), app_popover.clone())));

    let css = format!(
        ".menubar-label {{ font-family: '{}', 'SF Pro', sans-serif; font-size: 13px; font-weight: 400; color: {}; text-shadow: {}; }}\n.app-name-label {{ font-weight: 800; font-family: '{}', 'SF Pro Display', sans-serif; letter-spacing: -0.2px; }}\n.menubar-bar {{ background: transparent; background-color: transparent; min-height: {}px; box-shadow: none; border: none; }}",
        SF_FAMILY, fg, text_shadow, SF_FAMILY, win_h_tmp
    );
    UIKit::widget::apply_css(&bar, &css);
    let label_css = format!(".menubar-label {{ color: {}; }} .app-name-label {{ color: {}; }}", fg, fg);
    UIKit::widget::apply_css(&app_label, &label_css);

    bar.append(&octopus_wrap);
    bar.append(&app_wrap);
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
    pub fn set_input_region(_: u32, _: i32, _: i32, _: u16, _: u16) -> bool { false }
    pub fn get_active_window() -> Option<u32> { None }
    pub fn get_client_list() -> Vec<u32> { Vec::new() }
    pub fn get_wm_class(_: u32) -> Option<String> { None }
    pub fn get_active_app_name() -> String { "Finder".to_string() }
    pub fn minimize_window(_: u32) -> bool { false }
    pub fn close_window(_: u32) -> bool { false }
}

// ── Per-Monitor Fenster ────────────────────────────────────────────────

/// Erzeuge fuer jeden Monitor ein eigenes, transparentes Fenster oben.
/// 1:1 Spiegelung — Octopus + selected App-Name (bold), Hover + Menüs.
fn spawn_bars(app: &Application) {
    let app_name_initial = x11_place::get_active_app_name();
    LAST_APP_NAME.with(|s| *s.borrow_mut() = app_name_initial.clone());
    APP_MENUBAR_STATE.with(|s| s.borrow_mut().clear());
    OCTOPUS_MENUS.with(|s| s.borrow_mut().clear());
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

    // Live-Update: selected App alle 400ms pollen (WM_CLASS, nicht Titel)
    {
        let last_cell = std::rc::Rc::new(std::cell::RefCell::new(app_name_initial.clone()));
        glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
            let cur = x11_place::get_active_app_name();
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
                let cur_app = x11_place::get_active_app_name();
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
