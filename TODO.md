# TODO

## Milestones

- [x] Repo, AGENTS.md, TODO.md, workspace
- [x] `palettes-wayland`: layer surfaces, popups, outputs, seat, fractional scale, cursor shape
- [x] `palettes-shell`: Shell, Surface (panels, popups), per-output panels, Vulkan presentation, channels, system theme
- [x] `palettes-clock`: formatting and realtime ticks
- [x] `palettes-workspace`: neutral model, Pastel IPC backend
- [x] Pastel: `pastel-ipc` crate, IPC socket with workspace/window events and actions
- [x] `palettes-audio`: PipeWire default sink/source volume and mute (through device routes)
- [x] `palettes-assets`: asset roots, icon themes and desktop entries, fonts dir, file watcher
- [x] `palettes`: facade, services on the UI thread, markup Panel element, shell states,
      host actions, per-output panels, hot reload, runner binary
- [x] Examples, README (en/zh), docs

## Later

- [x] ext-workspace-v1 backend for other compositors
- [x] Rounded, translucent surfaces and compositor blur (ext-background-effect-v1)
- [x] Input regions that follow a rounded shape, so clicks outside it reach what is below

### Effects, simplest first

Markup exposure of what aegle already draws or animates (aegle-loader, aegle-markup):

- [x] Spring and cubic-bezier easings in `transition` timings
- [x] Repeating and alternating animations
- [x] Shapes: `Shape` (SVG path data) and `Arc`; fill, stroke, caps
- [x] Image backgrounds: `Image { fit: cover; inset: 0dp }`, resolved like assets, shrunk to fit a surface
- [ ] Reload images and icons when their files change

New scene primitives, on every aegle renderer:

- [ ] Per-corner radius
- [ ] Inset shadow
- [ ] Cut: a shape that punches a hole through what lies beneath
- [ ] Custom fragment shaders (uv, size, time, values); frames only while `time` is used

Services beyond the ones below: bluetooth, CPU and memory, app launcher entries,
a palette picked from the wallpaper, blurred image backgrounds.

### Platform

- [ ] wlr-foreign-toplevel window list (focus, close) without Pastel
- [ ] text-input-v3 (IME), clipboard and compose (dead keys) in palettes-wayland
- [ ] Cursor without cursor-shape-v1 (cursor theme loading)
- [ ] Services: battery (UPower via aegle-dbus), network, MPRIS, notifications server,
      system tray (StatusNotifierHost), brightness, keyboard layout
- [ ] Session lock surface (ext-session-lock-v1)
- [ ] Wallpaper layer with images
- [ ] `palettes msg toggle <doc>` over a socket, so compositor keybindings open launchers
- [ ] Popups and hidden/shown panels from markup
- [ ] Icons rasterized at the surface's scale rather than a fixed 64 px
- [ ] A quickshell-like runtime: whole shells as pure aegle markup, maybe scripted with
      Luau; config directory, hot reload of every file, IPC to toggle panels
