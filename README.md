# Palettes

**English** · [简体中文](README.zh-CN.md)

A Wayland desktop shell and widget library in Rust: bars, docks, launchers, popups and desktop widgets, drawn with Vulkan. Built on [aegle](https://github.com/antinomie1/aegle) (retained UI, layout, text, markup, Vulkan renderer) with its own Wayland client for shell surfaces. Works on any compositor with wlr-layer-shell; with [Pastel](https://github.com/antinomie1/Pastel) it also shows and drives workspaces and windows.

- **Markup panels**: one `.aegle` file per panel, opened on every output, reloaded in place on save
- **Shell state by name**: declare `state time`, `state workspaces`, `state volume`… and it stays current
- **Luau scripts**: `bar.luau` beside `bar.aegle` fills states, defines actions, runs commands, timers and file watches; saving reloads it
- **Popups**: `Panel { edge: popup }` opens beside the clicked control, sized to its content, and closes with a click elsewhere; tooltips leave the bar as popups
- **Command line**: `palettes msg toggle launcher.aegle` or any action, for compositor keybindings
- **Automatic assets**: theme icons by name or app id (SVG included), fonts from `fonts/`, paths relative to the document
- **Services**: clock, workspaces and windows (Pastel IPC; on other compositors through ext-workspace-v1 and wlr-foreign-toplevel-management), PipeWire volume and mute, processor and memory use, installed applications
- **System services over D-Bus**: battery and power profiles (UPower), media players (MPRIS), a notification server, the system tray with menus (StatusNotifierItem, DBusMenu), Wi-Fi (NetworkManager), bluetooth (BlueZ), the backlight (through logind): `state battery`, `state tray`, `host.wifi_connect(...)`
- **D-Bus in scripts**: `dbus.system(name, path)` calls methods and follows properties and signals, without blocking the shell
- **Glass and curves**: rounded, translucent panels with the desktop blurred behind them (`radius`, `background: #1e1e2ea0`, `opacity`, `blur: true`), on compositors with ext-background-effect-v1 such as KWin and Pastel
- **Wallpapers**: `Panel { edge: background }` with an image up to 4K, optionally blurred; its colors become the palette tokens `palette.1`–`palette.5`
- **Idle means idle**: nothing runs between changes; services start only when used
- **Composable**: any aegle control library, such as [am3](https://github.com/antinomie1/am3) (`--features am3`)

## Markup

`bar.aegle`:

```text
use "palettes.aegle"

Panel {
    edge: top
    size: 32dp
    state output_workspaces: list<Workspace> = []
    state time: string = ""
    Row {
        for ws in output_workspaces key ws.id {
            Button { text: ws.name; on clicked { host.focus_workspace(ws.id) } }
        }
        Text { text: time; grow: 1 }
    }
}
```

```sh
palettes bar.aegle            # or put it in ~/.config/palettes/shell.aegle and run `palettes`
palettes --check bar.aegle    # check without opening
```

`bar.luau` beside it, for data no service provides:

```lua
-- Fills `state disk: string` in every document that declares it.
every(60, function()
    exec("df --output=pcent / | tail -1", function(out)
        state.disk = (out:gsub("%s", ""))
    end)
end)

function lock() host.spawn("swaylock") end   -- markup: host.lock()
```

States, records, actions and placement are listed in [docs/markup.md](docs/markup.md); scripts in [docs/scripts.md](docs/scripts.md).

## Rust

```rust
use palettes::prelude::*;

fn main() -> Result<()> {
    let shell = Shell::new()?;
    let bar = shell.panel(PanelOptions::top(32))?;
    let time = bar.text("");
    shell.clock(&time, "%H:%M", |time, now| time.set_text(now))?;
    shell.run()
}
```

A surface is its root container: `bar.set_radius(12.0)`, `bar.set_background(color)` and `bar.set_opacity(0.9)` shape and tint it, and `bar.set_blur(&bar, true)` blurs the desktop behind it. Input follows the rounded shape; `bar.set_input(&pill, true)` limits it to chosen nodes.

Markup draws vector shapes (`Shape` with SVG path data, `Arc` for rings and gauges), animates scale, rotation and opacity, eases transitions with springs and Bézier curves, puts images behind a panel (`Image { fit: cover; inset: 0dp }`), and fills elements with WGSL fragment shaders that draw frames only while they use `time` (`Shader { source: "aurora.wgsl" }`); see [`examples/rings.aegle`](examples/rings.aegle), [`examples/aurora.aegle`](examples/aurora.aegle) and [`docs/markup.md`](docs/markup.md).

`palettes::run("bar.aegle")` runs a document from Rust. More in [`crates/palettes/examples`](crates/palettes/examples) and [`examples`](examples).

## Building

Rust ≥ 1.88, libxkbcommon, libpipewire-0.3, clang (bindgen), a Vulkan driver.

```sh
cargo build --release
cargo run --release -p palettes -- examples/bar.aegle
cargo run --release -p palettes --example bar
```

## Crates

| Crate | Purpose |
|---|---|
| `palettes-wayland` | Wayland client for layer surfaces, popups, outputs and input (compose, input methods, clipboard, drag and drop); aegle's platform contract |
| `palettes-shell` | `Shell` and `Surface`: aegle UI trees on shell surfaces, Vulkan presentation |
| `palettes-clock` | Local time formatting, realtime ticks |
| `palettes-workspace` | Compositor-neutral workspaces and windows; Pastel backend |
| `palettes-audio` | PipeWire default sink and source |
| `palettes-assets` | Asset roots, icon themes, desktop entries, file watching |
| `palettes-system` | D-Bus services: UPower, MPRIS, notifications, tray, NetworkManager, BlueZ |
| `palettes` | Facade, services, markup integration, Luau scripts, the `palettes` command |

Design notes: [docs/architecture.md](docs/architecture.md). Contributing: [AGENTS.md](AGENTS.md). Roadmap: [TODO.md](TODO.md).

## License

BSD 2-Clause, see [LICENSE](LICENSE).
