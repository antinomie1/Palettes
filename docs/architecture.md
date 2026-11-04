# Architecture

## Layers

```
palettes            facade: services ↔ UI thread, markup, runner
├── palettes-shell  Shell, Surface, Vulkan presentation, channels
│   └── palettes-wayland   aegle_platform::{Platform, SurfacePlatform}
├── palettes-clock, palettes-workspace, palettes-audio, palettes-assets, palettes-system
└── aegle: ui, widgets, loader, render-vulkan (git, branch main)
```

Service crates have no aegle or Wayland dependency. They report through callbacks on their own threads; the facade forwards values through `Shell::channel` to the UI thread.

## Event loop

One UI thread. `Shell::dispatch`:

1. Runs control callbacks, then refreshes every tree (layout, scenes).
2. Blocks in `poll` on the Wayland socket, a wake `eventfd` and the settings portal, bounded only by a control's own timer (tooltips) or a held key's repeat.
3. Routes events to trees; drains channel values on wake; resyncs per-output panels on output changes.
4. Presents surfaces the compositor asked to draw: each frame is requested with a frame callback, so drawing follows the compositor's pace and stops when nothing changes.

Background threads, each blocked until it has something to say: clock (`timerfd` on the realtime clock, armed for the next second or minute, cancelled by clock changes), compositor IPC reader and command writer, PipeWire loop, inotify watcher, one per D-Bus service, and a reader per script bus. Clipboard pipes are read and written on short-lived threads; a finished read comes back through a channel and the wake `eventfd`, and pasting this client's own selection skips the pipe.

## Input

Keys go through xkbcommon and the locale's compose table, so dead keys and the Compose key type what they finish; a held key repeats the text of its first press. Each seat has a text-input-v3 object, enabled while the focused tree has an editor (`Ui::take_ime_state`); answers to older commits are applied but hold back new state until the input method catches up. Each seat has a `wl_data_device` for text selections, authorized by the seat's latest press. Cursors use cursor-shape-v1; without it, images come from the desktop's xcursor theme (`XCURSOR_THEME` and `XCURSOR_SIZE`, else KDE's `kcminputrc`, else GTK's `settings.ini`), loaded on first use per integer scale. Drag and drop shares the data devices: a drag over a window becomes `Event::Drag`, which the shell answers with whether a control there takes drops; a drop is read like a paste, as a file list before text, and the offer finished. Drags a control starts are copies, authorized by the held press.

## Rendering

Every surface has an aegle `WindowRenderer` on one shared Vulkan device and glyph cache, with a translucent swapchain. Its memory and upload budgets are limits sized for one wallpaper (3840 × 2400) beside its text; images take only what they need. Scenes are recorded in logical pixels and scaled to the surface's fractional scale; a viewport maps the buffer back to logical size. Damage from the tree limits what is redrawn. Frames clear to transparent; the root paints the theme background (a token binding, so it follows dark mode), and documents make the root transparent so the `Panel` paints itself. Input and blur follow shapes: a surface keeps the nodes that take its input (none means the root; a document's `Panel` is one) and the nodes it blurs behind. After each refresh their rounded bounds become whole-pixel rectangles (`palettes_wayland::rounded_rects`); the shown cut nodes of the tree, found in one walk, become holes subtracted from both regions. They are sent as the `wl_surface` input region and through ext-background-effect-v1 when they change, and carried by the next frame's commit. A shape covering the whole surface, without holes, sends no input region. Custom fragment shaders compile once per surface renderer on first use; a surface draws frames for one only while it is shown and names `time`.

## Surfaces

Only layer surfaces and xdg popups. A panel's placement (`PanelOptions`) can change in place; popups attach to a panel or another popup with a grab for menus. A fitted popup (`Surface::fit`) lets one node keep its content's size; after each refresh a different size is asked for with `xdg_popup.reposition` (xdg-shell 3), and frames wait for the configure that applies it. Tooltips do not draw inside their window: aegle reports them (`aegle_widgets::host_tooltips`) and the shell shows each in a passive popup fitted to its text, below the control or flipped above it. Removing a surface first drops its presenter, then its popups, then the Wayland objects. `Surface::on_close` callbacks run after the event that closed the surface, outside the runtime's borrow, whether the shell or the compositor closed it.

## Subscriptions

Services keep the latest value and a list of followers, each owned by a control. A follower whose control was removed is dropped at the next update, so closing a panel (for example when its output disappears) ends everything it followed without bookkeeping.

## Markup

The facade registers `Panel` and `Icon` elements and host actions on aegle's loader. Opening a document first builds it into a throwaway tree to read its `Panel` placement, then opens a surface per wanted output and builds it there. Shell states are matched by name; records come from the embedded prelude served for `use "palettes.aegle"`. A reload rebuilds each view in place (`View::reload`), re-attaches states, and moves surfaces to the new placement. Images, icons and shaders watch their own files: a change loads the file again into every view showing it, without rebuilding the document.

A popup document (`edge: popup`) opens on the surface whose engine reports the pointer, anchored to the hovered node's bounds (the pointer itself over the panel's background), on the side away from that panel's edge. Its surface's close callback removes the document, so a dismissal stops its script.

## Scripts

One Luau VM per UI thread (mlua, vendored Luau). Each script gets its own globals table over a table of built-ins over Luau's libraries, so what it defines at top level is found with a raw lookup and is its set of host actions. Markup host calls that name no Palettes action are registered with aegle, with the call's argument types, as calls into whichever script defines the function then; documents are therefore checked after their script ran, and reloaded when it is saved.

Shared states: views link their entry states (`markup::link`). A script assignment converts the Luau value by each linked view's declared type and sets it there; it is also kept, so views linked later start with it. Each linked state has a change handler that hands the new value to the runtime, which skips its own assignments and runs watches. Commands, timers and line readers run on their own threads and report through a shell channel to the UI thread; each belongs to its script and stops (commands by process group) when the script unloads.

## Messages

`palettes msg` connects to `$XDG_RUNTIME_DIR/palettes-$WAYLAND_DISPLAY.sock`. An accept thread hands each connection to a thread that reads JSON lines and sends each request through a shell channel; the UI thread runs the action (Palettes' own, arguments parsed by its types, else a script function) and replies over a one-shot channel. A second instance finds the socket answering and leaves it alone.

## Pastel

Pastel exposes `$XDG_RUNTIME_DIR/pastel.$WAYLAND_DISPLAY.sock` (`PASTEL_SOCKET` for its children): JSON lines defined in Pastel's `pastel-ipc` crate. A subscriber receives the whole state (outputs, workspaces, windows) after each change, published from dirty frames only. `palettes-workspace` maps it to a neutral model. Its second backend speaks the Wayland protocols other compositors offer, on one connection and thread: ext-workspace-v1 for workspaces and switching, wlr-foreign-toplevel-management for windows, focusing and closing; neither tells which workspace a window is on. The desktop is reported once per dispatched batch, so a window's burst of title, state and `done` events is one update. `Compositor::connect` prefers Pastel's IPC and falls back to it.

## Audio

Volume and mute are read from the default node's `Props` and written through its device's active `Route` (saved), like desktop mixers, so all mixers agree. Volumes use the cubic scale mixers display.

## System services

`palettes-system` runs each D-Bus service (battery, media, notifications, tray, network, bluetooth) on its own aegle-dbus connection and thread. The thread `poll`s the bus socket, a socket pair that wakes it for commands from the UI thread, and the service's next deadline (notification expiry, the next whole second of a playing track's position when `media_position` is followed). A blocking call made on that thread queues the signals arriving meanwhile; after each batch of messages the service rebuilds its state and reports it only if it changed, so a burst of `PropertiesChanged` is one update. Commands are sent without waiting for their reply; error replies are logged.

NetworkManager and BlueZ are mirrored through `org.freedesktop.DBus.ObjectManager` (`GetManagedObjects`, then `InterfacesAdded`, `InterfacesRemoved` and `PropertiesChanged`); UPower and MPRIS players through `GetAll` and `PropertiesChanged`; tray items are read again whole on their `New*` signals. The notification server and the tray's watcher answer calls on their connections. Pixels that arrive over the bus become PNG files under `$XDG_RUNTIME_DIR/palettes`, rewritten only when they change, so the watcher reloads what shows them.

The facade keeps one topic per service type (`Services::follow::<S>`), started for the first document declaring one of its states or the first action using it. Script buses (`dbus.session`, `dbus.system`) are plain connections whose reader thread forwards every reply and signal through the script runtime's channel; the UI thread matches replies to pending callbacks by serial and signals to watches by path, interface, member and first argument.

The backlight service reads the chosen device's `actual_brightness` (firmware before platform before raw, as systemd chooses) and waits on it for `POLLPRI` beside its bus: the kernel notifies the attribute on every change, whoever made it. It sets the level with logind's `SetBrightness`. Processor and memory use are the one exception to waiting: the kernel announces neither, so a thread samples `/proc/stat` and `/proc/meminfo` every two seconds once a document declares `cpu` or `memory`.
