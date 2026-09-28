# Menubar – Wiki

Menubar is the TontooOS top bar: one transparent layer-shell surface per
Wayland output (top edge, exclusive zone, no keyboard focus), each showing
the octopus, the selected app name, example menus, status symbols and the
clock, with transparency-only dropdown menus.

- Repository: https://github.com/TontooOS/TontooOS
- License: TCL
- Version: 26.1.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| Menubar | [Menubar.md](Menubar.md) | Bar UI, menus, actions, daemon polling |

## Quick Start

Run the bar (needs a layer-shell compositor like TontooCompositor):

```bash
cargo run
```

One instance serves all monitors: `run_layer` builds a `MenubarApp` per
output (30 logical px, 42 on notch outputs).

See [Menubar.md](Menubar.md) for details.

## Changelog

- 2026-09-28: Port to the new TontooUI on Vello/WGPU (layer-shell backend,
  `BarMenu` dropdowns, 7 modules); no more GTK/UIKit/layer-shell crate.
