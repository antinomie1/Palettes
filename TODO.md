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
- [x] Reload images and icons when their files change

New scene primitives, on every aegle renderer:

- [x] Per-corner radius: `corners: [bottom_left, bottom_right]`; input and blur regions follow
- [x] Inset shadow: `inset_shadow: [x, y, blur, spread, color]`
- [x] Cut: `cut: true` punches a hole through what lies beneath
- [x] Input and blur regions leave out cut holes
- [x] Custom fragment shaders (uv, size, time, values); frames only while `time` is used

Every effect above is done (2026-11-04); shaders recompile when their file is saved.

Beyond the services below: blurred image backgrounds.

### Platform

- [x] wlr-foreign-toplevel window list (focus, close) without Pastel
- [x] text-input-v3 (IME), clipboard and compose (dead keys) in palettes-wayland
- [x] Cursor without cursor-shape-v1 (cursor theme loading, KDE/GTK settings)
- [x] Drag and drop of text and files between surfaces and clients
- [ ] Primary selection (middle-click paste): needs an aegle platform API first
- [x] D-Bus services: battery and power profiles, MPRIS, notification server, system tray
      with DBusMenu, NetworkManager Wi-Fi, BlueZ; `dbus` for scripts
- [ ] Brightness, keyboard layout, CPU and memory, wallpaper palette as built-in states
- [ ] PipeWire: device lists, default device switching, per-app volume, peak levels
- [ ] Session lock surface (ext-session-lock-v1)
- [ ] Wallpaper layer with images
- [x] `palettes msg <action>` over a socket, so compositor keybindings open launchers
- [x] Popups from markup (`edge: popup`), `host.open` / `host.close`
- [x] Icons rasterized at the surface's scale rather than a fixed 64 px
- [x] Luau scripts beside documents: shared states, actions, commands, timers, file watches
- [x] Passive popups (tooltips) and popups sized to their content
