# Architecture

## Layers

```
palettes            facade: services ↔ UI thread, markup, runner
├── palettes-shell  Shell, Surface, Vulkan presentation, channels
│   └── palettes-wayland   aegle_platform::{Platform, SurfacePlatform}
├── palettes-clock, palettes-workspace, palettes-audio, palettes-assets
└── aegle: ui, widgets, loader, render-vulkan (git, branch main)
```

Service crates have no aegle or Wayland dependency. They report through callbacks on their own threads; the facade forwards values through `Shell::channel` to the UI thread.

## Event loop

One UI thread. `Shell::dispatch`:

1. Runs control callbacks, then refreshes every tree (layout, scenes).
2. Blocks in `poll` on the Wayland socket, a wake `eventfd` and the settings portal, bounded only by a control's own timer (tooltips) or a held key's repeat.
3. Routes events to trees; drains channel values on wake; resyncs per-output panels on output changes.
4. Presents surfaces the compositor asked to draw: each frame is requested with a frame callback, so drawing follows the compositor's pace and stops when nothing changes.

Background threads, each blocked until it has something to say: clock (`timerfd` on the realtime clock, armed for the next second or minute, cancelled by clock changes), compositor IPC reader and command writer, PipeWire loop, inotify watcher.

## Rendering

Every surface has an aegle `WindowRenderer` on one shared Vulkan device and glyph cache, with a translucent swapchain. Scenes are recorded in logical pixels and scaled to the surface's fractional scale; a viewport maps the buffer back to logical size. Damage from the tree limits what is redrawn. Frames clear to transparent; the root paints the theme background (a token binding, so it follows dark mode), and documents make the root transparent so the `Panel` paints itself. Input and blur follow shapes: a surface keeps the nodes that take its input (none means the root; a document's `Panel` is one) and the nodes it blurs behind. After each refresh their rounded bounds become whole-pixel rectangles (`palettes_wayland::rounded_rects`); the shown cut nodes of the tree, found in one walk, become holes subtracted from both regions. They are sent as the `wl_surface` input region and through ext-background-effect-v1 when they change, and carried by the next frame's commit. A shape covering the whole surface, without holes, sends no input region. Custom fragment shaders compile once per surface renderer on first use; a surface draws frames for one only while it is shown and names `time`.

## Surfaces

Only layer surfaces and xdg popups. A panel's placement (`PanelOptions`) can change in place; popups attach to a panel or another popup with a grab for menus. Removing a surface first drops its presenter, then its popups, then the Wayland objects.

## Subscriptions

Services keep the latest value and a list of followers, each owned by a control. A follower whose control was removed is dropped at the next update, so closing a panel (for example when its output disappears) ends everything it followed without bookkeeping.

## Markup

The facade registers `Panel` and `Icon` elements and host actions on aegle's loader. Opening a document first builds it into a throwaway tree to read its `Panel` placement, then opens a surface per wanted output and builds it there. Shell states are matched by name; records come from the embedded prelude served for `use "palettes.aegle"`. A reload rebuilds each view in place (`View::reload`), re-attaches states, and moves surfaces to the new placement. Images, icons and shaders watch their own files: a change loads the file again into every view showing it, without rebuilding the document.

## Pastel

Pastel exposes `$XDG_RUNTIME_DIR/pastel.$WAYLAND_DISPLAY.sock` (`PASTEL_SOCKET` for its children): JSON lines defined in Pastel's `pastel-ipc` crate. A subscriber receives the whole state (outputs, workspaces, windows) after each change, published from dirty frames only. `palettes-workspace` maps it to a neutral model. Its second backend speaks the Wayland protocols other compositors offer, on one connection and thread: ext-workspace-v1 for workspaces and switching, wlr-foreign-toplevel-management for windows, focusing and closing; neither tells which workspace a window is on. The desktop is reported once per dispatched batch, so a window's burst of title, state and `done` events is one update. `Compositor::connect` prefers Pastel's IPC and falls back to it.

## Audio

Volume and mute are read from the default node's `Props` and written through its device's active `Route` (saved), like desktop mixers, so all mixers agree. Volumes use the cubic scale mixers display.
