# Menubar

Menubar renders the TontooOS top bar: octopus, bold selected app name,
example menu titles, status symbols and a clock, on a transparent
layer-shell surface per output. Dropdown menus are transparency-only
`BarMenu` panels (no blur). The selected app resolves through
CoreWindows (daemon) with an X11 fallback; menu actions drive Hide,
Quit, About, Zoom and `tapp` launches.

## Modules

| Module | Path | Description |
|---|---|---|
| `main` | `src/main.rs` | Entry point, `run_layer` factory (notch heights) |
| `bar` | `src/bar.rs` | `MenubarApp`: manual bar layout, triggers, clock, theme |
| `menus` | `src/menus.rs` | Octopus/app/example menu content plus action dispatch |
| `actions` | `src/actions.rs` | Selected-app detection, `tapp` launches, window actions |
| `daemon` | `src/daemon.rs` | Background window-daemon poller with fresh cache |
| `x11` | `src/x11.rs` | X11 EWMH queries and actions (no GTK) |
| `i18n` | `src/i18n.rs` | Accessibility translations (`en_us`, `de_de`) |

## Bar Layout

The bar is 30 logical px high, 42 on notch outputs. Notch detection:
`MENUBAR_NOTCH=1|37|true` forces notch height (`0` forces standard);
otherwise only internal laptop panels (`eDP`/`LVDS`) on Apple hardware
qualify (DMI check, same as before).

```rust
pub fn bar_height_for_output(output_name: &str) -> u32
```

Layout is manual (no stacks, so trigger rects are exact): 6px left
margin, octopus (`(height - 2) * 1.15`, clamped 22..48), bold app name,
example titles, spacer, status symbols (16px, theme-tinted), clock.
Elements are vertically centered; the right block is right-aligned with
a 10px margin.

## App Name

```rust
pub fn selected_app() -> (String, Option<i32>)
pub fn is_shell_name(name: &str) -> bool
```

Polled every 0.4s: the active window pid (X11) bridged to CoreWindows
daemon rows (bundle names win over X11 titles). Shell windows
(`menubar`, `dock`, empty) never appear; the bar keeps the last real
app (`Finder` when empty). The open app menu rebuilds on name change.

## Menus

Triggers (octopus, app name, example titles except Help) toggle
`BarMenu` panels anchored below the bar. Hovering another trigger
while a menu is open switches to it (macOS behavior). Menu content
rebuilds on every open, so labels and enabled states stay current.

| Menu | Content |
|---|---|
| Octopus | About (SystemOverview), Settings, App Store, Recent, Force Quit (disabled on `Finder`), Sleep, Restart, Shut Down, Lock, Log Out (`{user}`) |
| App | Hide app, Hide others, About app, Quit (disabled on `Finder`) |
| Example | Static File/Edit/View/Go/Window/Help rows; Window rows act on the focused window |

```rust
pub enum MenuId { Octopus, App, Example(&'static str) }
pub fn build_menu(id: MenuId, app_name: &str, on_action: impl FnMut(Vec<usize>) + 'static) -> (BarMenu, MenuActions)
```

Paths dispatch through `handle_octopus`, `handle_app` and
`handle_example` (the latter with the zoomed-window set).

## Actions

| Function | Behavior |
|---|---|
| `launch_via_tapp(bundle, args)` | Spawns `/usr/bin/tapp`; `false` when missing or failing |
| `launch_system_overview()` | `/Applications`, `/System/Applications`, then `$HOME` fallback |
| `open_about_this_app()` | AboutThisApp for the selected bundle (or bare) |
| `hide_current_app()` / `hide_other_apps()` | Daemon minimize, X11 fallback |
| `quit_current_app()` | Daemon close, X11 fallback |
| `minimize_focused_window()` / `close_focused_window()` | Daemon, X11 fallback |
| `toggle_zoom_focused_window(zoomed)` | Daemon fullscreen with local zoom tracking, X11 fallback |
| `force_quit_current_app()` | `SIGKILL` via CoreWindows, never menubar/dock |

## Daemon

```rust
pub fn start_daemon_poller()
pub fn cached_windows() -> Option<Vec<WindowInfo>>
```

Background thread polls `WindowsProvider` every 800ms (fast probe
first, 10s-spaced warnings). The bar reads only the cache (fresh
within 5s), so a hung daemon never freezes the bar.

## Clock

```rust
pub fn clock_text(now: SystemTime) -> String
```

Formats like `date "+%a %b %e %-I:%M %p"` (`Mon Sep 28 2:05 PM`),
recomputed from system time and updated when the minute flips.

## Localization

`lang/en_us.json` and `lang/de_de.json` (Accessibility shape) load
through `LangStore`; locale follows `LANGUAGE`/`LANG`/`LC_ALL` and
`/etc/locale.conf`. `$MENUBAR_LANG_DIR` overrides the search dir.
Placeholders use `%name%` style (`menu.about_app`, `menu.hide_app`,
`menu.logout`).

## Cross References

- [MAIN.md](MAIN.md) – overview and quick start
- TontooUI `Renderer.md` – layer-shell backend (`run_layer`)
- TontooUI `Menu.md` – `BarMenu` transparency-only dropdowns
