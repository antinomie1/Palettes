# TODO

## Milestones

- [x] Repo, AGENTS.md, TODO.md, workspace
- [x] `palettes-wayland`: layer surfaces, popups, outputs, seat, fractional scale, cursor shape
- [ ] `palettes-shell`: Shell, Panel, Popup, Vulkan presentation, timers, channels
- [x] `palettes-clock`: formatting and realtime ticks
- [x] `palettes-workspace`: neutral model, Pastel IPC backend
- [x] Pastel: `pastel-ipc` crate, IPC socket with workspace/window events and actions
- [ ] `palettes-audio`: PipeWire default sink/source volume and mute
- [ ] `palettes-assets`: asset roots, icon themes and desktop entries, fonts dir, file watcher
- [ ] `palettes`: facade, services on the UI thread, markup Panel element, shell states,
      host actions, per-output panels, hot reload, runner binary
- [ ] Examples, README (en/zh), docs

## Later

- [ ] ext-workspace-v1 backend for other compositors
- [ ] wlr-foreign-toplevel / ext-foreign-toplevel-list window list without Pastel
- [ ] text-input-v3 (IME), clipboard and compose (dead keys) in palettes-wayland
- [ ] Cursor without cursor-shape-v1 (cursor theme loading)
- [ ] Switch `pastel-ipc` to a git dependency once Pastel is pushed
- [ ] Services: battery (UPower via aegle-dbus), network, MPRIS, notifications server,
      system tray (StatusNotifierHost), brightness, keyboard layout
- [ ] Session lock surface (ext-session-lock-v1)
- [ ] Wallpaper layer with images
- [ ] A quickshell-like runtime: whole shells as pure aegle markup, maybe scripted with
      Luau; config directory, hot reload of every file, IPC to toggle panels
