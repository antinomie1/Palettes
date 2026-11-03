# Palettes

**English** · [简体中文](README.zh-CN.md)

A Wayland desktop shell and widget library in Rust: bars, docks, launchers, popups and desktop widgets, drawn with Vulkan. Built on [aegle](https://github.com/antinomie1/aegle) (retained UI, layout, text, markup, Vulkan renderer) with its own Wayland client for shell surfaces. Works on any compositor with wlr-layer-shell; with [Pastel](https://github.com/antinomie1/Pastel) it also shows and drives workspaces and windows.

- **Markup panels**: one `.aegle` file per panel, opened on every output, reloaded in place on save
- **Shell state by name**: declare `state time`, `state workspaces`, `state volume`… and it stays current
- **Automatic assets**: theme icons by name or app id (SVG included), fonts from `fonts/`, paths relative to the document
- **Services**: clock, workspaces and windows (Pastel IPC), PipeWire volume and mute, installed applications
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

States, records, actions and placement are listed in [docs/markup.md](docs/markup.md).

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

`palettes::run("bar.aegle")` runs a document from Rust. More in [`crates/palettes/examples`](crates/palettes/examples) and [`examples`](examples).

## Building

Rust ≥ 1.88, libxkbcommon, libpipewire-0.3, clang (bindgen), a Vulkan driver. A sibling checkout of [Pastel](https://github.com/antinomie1/Pastel) at `../Pastel` provides `pastel-ipc`.

```sh
cargo build --release
cargo run --release -p palettes -- examples/bar.aegle
cargo run --release -p palettes --example bar
```

## Crates

| Crate | Purpose |
|---|---|
| `palettes-wayland` | Wayland client for layer surfaces, popups, outputs and input; aegle's platform contract |
| `palettes-shell` | `Shell` and `Surface`: aegle UI trees on shell surfaces, Vulkan presentation |
| `palettes-clock` | Local time formatting, realtime ticks |
| `palettes-workspace` | Compositor-neutral workspaces and windows; Pastel backend |
| `palettes-audio` | PipeWire default sink and source |
| `palettes-assets` | Asset roots, icon themes, desktop entries, file watching |
| `palettes` | Facade, services, markup integration, the `palettes` command |

Design notes: [docs/architecture.md](docs/architecture.md). Contributing: [AGENTS.md](AGENTS.md). Roadmap: [TODO.md](TODO.md).

## License

BSD 2-Clause, see [LICENSE](LICENSE).
