# AGENTS.md

Guidance for agents working in Palettes.

## What this is

Palettes is a Wayland desktop shell and widget library in Rust 2024: panels, bars,
docks, popups and overlays drawn with Vulkan. It is built on
[aegle](https://github.com/antinomie1/aegle) (retained UI, layout, text, glyphs,
markup, Vulkan renderer) but brings its own Wayland client, written for shell
surfaces. It works with any wlr-layer-shell compositor and integrates deeply with
[Pastel](https://github.com/antinomie1/Pastel) through its IPC socket.

## Commands

```sh
cargo build                          # needs libxkbcommon, libpipewire-0.3, clang (bindgen), a Vulkan loader
cargo test                           # every crate's tests/; live parts skip without WAYLAND_DISPLAY / PipeWire
cargo test -p palettes-shell --test input -- --ignored   # typing, IME, clipboard, drag; headless Pastel only
cargo run -p palettes --example bar  # the Rust bar on every output
cargo run -p palettes -- examples/bar.aegle    # the markup runner; save the file to reload
cargo run -p palettes -- --check examples/bar.aegle
cargo clippy --all-targets -- -D warnings
```

Screenshots need a compositor with screencopy. KDE has none; use a headless Pastel:
`LUMINARIA_BACKEND=headless PASTEL_EXIT_MS=10000 ../Pastel/target/debug/pastel --config <empty>`,
then run against its `WAYLAND_DISPLAY` and capture with `grim`.

Read `docs/architecture.md` before changing the loop, surfaces or services, and keep
`docs/markup.md` in step with states, records, actions and `Panel` properties.

## Layout of the code

Dependencies only point downwards:

```
crates/
  palettes-clock/      Local time, strftime formatting, next-tick deadlines. Pure.
  palettes-workspace/  Compositor-neutral workspace/window model; Pastel IPC and
                       Wayland (ext-workspace-v1, wlr-foreign-toplevel) backends.
  palettes-audio/      PipeWire default sink/source: volume, mute. Own thread.
  palettes-assets/     Asset roots, freedesktop icon and desktop-entry lookup, fonts
                       directory, inotify watcher for hot reload. Pure.
  palettes-wayland/    Wayland client for shell surfaces: layer surfaces, popups,
                       outputs, seat input (compose, text-input-v3, clipboard,
                       drag and drop),
                       fractional scale, cursor shape or xcursor theme.
                       Implements aegle_platform::{Platform, SurfacePlatform}.
  palettes-shell/      The host: Shell, Panel, Popup, Vulkan presentation, timers,
                       cross-thread channels. aegle-ui trees on palettes-wayland.
  palettes/            Facade: services wired into the shell, markup integration
                       (Panel element, shell states, host actions), the runner binary.
```

Service crates (`clock`, `workspace`, `audio`, `assets`) know nothing about aegle or Wayland.
They report through plain callbacks; the facade moves values onto the UI thread.

## Rules

- **One source file stays under 1000 lines.** Split by responsibility before that.
- **Tests live in each crate's `tests/`, never in `src/`.**
- **Idle means idle.** No polling, no periodic timers. A clock wakes at the next
  visible change (minute or second), services block on their fds or threads, the
  loop blocks in `Platform::dispatch(None)` otherwise.
- **Services start lazily.** Nothing connects to PipeWire or Pastel until something
  subscribes.
- **Reuse aegle.** Never use `aegle-platform-wayland`, `aegle-platform-sdl3` or
  `aegle-app`. Everything else (ui, widgets, loader, render-vulkan, text, glyph) is
  fair game. Follow aegle's `aegle_platform::Platform` contract in palettes-wayland.
- **One aegle.** Depend on aegle through git `branch = "main"` like am3 does, so
  control libraries (am3) share the same aegle-ui types. To hack on both, add
  locally and don't commit:
  ```toml
  [patch."https://github.com/antinomie1/aegle"]
  aegle-ui = { path = "../aegle/crates/aegle-ui" }
  # ...one line per aegle crate in Cargo.lock
  ```
- **Keep APIs small.** A minimal panel stays under ten lines of Rust. Markup and Rust
  see the same shell state with the same names.
- **Markup interop is by convention.** A document declares `state <name>` with a
  known name and type; the facade fills it. Actions are `host.<name>(...)`. Record
  types live in the embedded `palettes.aegle` prelude. Adding a capability means:
  a service, a state name, a record (if any), actions, docs in `docs/markup.md`.
- **Assets resolve themselves.** Paths in markup resolve against the document's
  directory, then `$XDG_CONFIG_HOME/palettes`, then `$XDG_DATA_DIRS/palettes`. Icons are
  names, never paths: an app id or icon name resolves through desktop entries and the
  icon theme, so records carry ready-to-load paths. Fonts in an asset root's `fonts/`
  register themselves. Every loaded file is watched; a change reloads what used it.
- **Pastel's protocol is Pastel's.** Wire types live in Pastel's `pastel-ipc` crate,
  pinned by `rev` in the root `Cargo.toml`; palettes-workspace adapts them to the
  neutral model. To change both together, add locally and don't commit:
  ```toml
  [patch."https://github.com/antinomie1/Pastel"]
  pastel-ipc = { path = "../Pastel/crates/pastel-ipc" }
  ```
- **Don't over-engineer.** A trait exists because two implementations exist now.
- **Comments say why**, in English. Docs are English; READMEs are bilingual.
- **Commits**: coherent milestones, signed, with the co-author trailer.
