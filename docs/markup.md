# Panel documents

A panel document is an aegle markup file whose root is a `Panel`. `palettes FILE...` (or `palettes::run`) opens each document on every output, or only on the one it names, and reloads it in place when any of its files is saved. A Luau script beside it (`bar.luau` for `bar.aegle`) runs while it is open; see [scripts.md](scripts.md). Aegle's markup language itself (bindings, `if`, `for`, components, records) is described in aegle's `docs/markup.md`.

```text
use "palettes.aegle"

Panel {
    edge: top
    size: 32dp
    state time: string = ""
    Text { text: time }
}
```

`use "palettes.aegle"` imports the record types below. A real `palettes.aegle` next to the document takes precedence.

## Panel

A column whose properties place its surface. All of them can be bound.

| Property | Type | Default | Meaning |
|---|---|---|---|
| `edge` | `top` `bottom` `left` `right` `center` `background` `popup` | `top` | Where it sits. `center` is an overlay; `background` covers the output below windows; `popup` opens beside a control (below) |
| `size` | length | `32dp` | Bar thickness; height of a `center` panel or popup (a popup without one is as tall as its content) |
| `span` | length | `0` | Length along the edge (width of a `center` panel or popup); `0` stretches, and a popup is as wide as its content. A bar with a span is a dock |
| `layer` | `background` `bottom` `top` `overlay` | by edge | Stacking layer |
| `exclusive` | bool | bars: true | Reserve its space so windows avoid it |
| `keyboard` | `none` `exclusive` `on_demand` | `none` | Keyboard focus policy; launchers use `exclusive`. A focused `TextField` takes compose sequences (dead keys), input methods (text-input-v3) and the clipboard |
| `offset` | length | `0` | Distance from the anchored edges |
| `output` | line | `""` | Only this output (`DP-1`); empty for every output |
| `blur` | bool | `false` | The compositor blurs the desktop behind the panel's rounded shape |
| `passive` | bool | `false` | A popup that takes no keyboard and stays open on clicks elsewhere |

Layout properties (`padding`, `gap`, `background`, …) apply as on `Column`.

### Popups

A document with `edge: popup` opens when `host.toggle` or `host.open` names it, beside the control under the pointer: above it on a bottom bar, beside it on a side bar or another popup, below it otherwise. It takes the keyboard while open, and a click outside the shell closes it, stopping its script; a `passive` one does neither and closes only by `host.close`. A popup opened from another popup is a submenu. Without `size` or `span` a popup takes its content's height or width and follows it as states change, so a menu grows with its entries.

`tooltip: "..."` on any control shows a small popup beside it once the pointer rests there, outside the bar.

```text
// calendar.aegle, opened by `Button { text: time; on clicked { host.toggle("calendar.aegle") } }`
Panel {
    edge: popup
    size: 232dp
    span: 252dp
    radius: 14dp
    blur: true
    state month: string = ""
    Text { text: month }
}
```

Popups share data with the panel that opened them through scripts (`state.month = ...` reaches every document declaring `month`); see [`examples/stats.aegle`](../examples/stats.aegle) and [`examples/calendar.aegle`](../examples/calendar.aegle).

### Visual effects

The panel fills a transparent surface and paints only itself, so its own style properties shape what shows of the desktop:

```text
Panel {
    edge: top
    size: 36dp
    offset: 8dp             // float it off the edges
    radius: 18dp            // round corners; the desktop shows around them
    corners: [bottom_left, bottom_right]  // only these; a bar flush with an edge
    background: #1e1e2ea0   // translucent; the theme background by default
    opacity: 0.95           // the whole panel, contents included
    blur: true              // frosted glass behind its shape
}
```

Input follows the same shape: clicks around a rounded panel's corners reach what is below. `blur` follows the panel's bounds, `radius`, `corners` and visibility. It needs a compositor with ext-background-effect-v1 (KWin 6, Pastel with `blur` on); elsewhere the panel is only translucent. Pastel blurs the wallpaper and layers below a panel, not windows, when `blur-xray` is on. `backdrop_blur` on a control blurs what the panel itself draws beneath that control. `shadow` draws inside the surface, so give it room with `padding`. See `examples/glass.aegle`.

### Holes and inner shadows

```text
Panel {
    inset_shadow: [0dp, 3dp, 6dp, 0dp, #00000080]   // pressed in; the blur must be positive
    Column { width: 48dp; height: 48dp; radius: 24dp; cut: true }  // the desktop shows through
}
```

`cut: true` paints a hole of the element's shape instead of its background; its border and content still draw. A shown hole is left out of the surface's input and blur regions, so clicks inside it reach what is below and the desktop shows through it unblurred; whatever the hole contains still draws there, but takes no input.

### Image backgrounds

An `Image` with `inset: 0dp` leaves the layout and fills the panel; `fit: cover` crops it to the panel's shape when it has the same `radius`:

```text
Panel {
    radius: 20dp
    Image { source: "wall.png"; fit: cover; inset: 0dp; radius: 20dp }
    Row { Text { text: "on the picture" } }
}
```

### Wallpapers and the palette

A `background` panel with an image is a wallpaper; images up to 3840 × 2400 show at full size, larger ones shrink once. `blur` blurs an image by a radius in its own pixels (cheaply, on a smaller copy). `palette: true` makes the image's main colors the palette: tokens `palette.1` (the most common) to `palette.5`, which any document's color properties can use, and the `palette` state. Until an image sets them they are the theme's accent.

```text
// wallpaper.aegle
Panel {
    edge: background
    Image { source: "wallpaper.jpg"; fit: cover; inset: 0dp; blur: 12; palette: true }
}
// bar.aegle
Panel { background: token("palette.1"); Text { text: time; foreground: token("palette.3") } }
```

### Shaders

A `Shader` fills itself with a WGSL fragment program, following its `radius` and `corners`. Like an `Image` it has no size of its own; with `inset: 0dp` in a `Stack` it is a panel's background:

```text
Stack {
    Shader { source: "shaders/aurora.wgsl"; values: [volume, 0.5]; inset: 0dp; radius: 20dp }
    Row { Text { text: time } }
}
```

```wgsl
// shaders/aurora.wgsl
fn shade(uv: vec2f, size: vec2f) -> vec4f {
    let band = smoothstep(0.3, 0.0, abs(uv.y - 0.5 - 0.2 * sin(uv.x * 6.0 + time)));
    return vec4f(0.54, 0.71, 0.98, band * values[0]);
}
```

- `shade` gets `uv`, from 0 to 1 across the element from its top-left corner, and `size` in dp, and returns an unpremultiplied sRGB color. It may read `time`, seconds since the program was shown, and `values`, an `array<f32, 8>` filled from `values: [...]` (up to eight numbers or expressions; the rest are zero).
- A program that never names `time` is drawn once and again only when `values` or its size change. One that names it keeps the panel drawing every frame while it is shown, and nothing else does; with reduced motion `time` stays at zero.
- `source` resolves like other assets, and saving the file compiles it again. A program that does not compile empties the element, logs `palettes: shader file:line:column: reason` and runs `on failed`, with `self.error`.

See `examples/aurora.aegle`.

### Shapes and motion

```text
Stack {                                     // children overlap in one cell
    Arc { width: 32dp; height: 32dp; stroke: #45475a; stroke_width: 4dp }
    Arc { width: 32dp; height: 32dp; sweep: volume * 360.0; stroke: #89b4fa
          stroke_width: 4dp; cap: round }
}
Shape { width: 16dp; height: 16dp; path: "M 2 8 L 6 12 L 14 4"; stroke: #a6e3a1; stroke_width: 2dp }
Icon { name: "view-refresh"; rotation_animation: [[0, 360], 1000ms, linear, forever] }
Text { text: time; opacity_animation: [[0.5, 1], 800ms, ease_in_out, alternate] }
```

- `Shape` draws SVG path data in dp from its top-left corner; `Arc` draws `sweep` degrees from `start`, clockwise from the top, on the largest circle that fits. Both take `fill`, `stroke`, `stroke_width` and `cap` (`butt`, `round`, `square`) and have no size of their own.
- `scale_animation`, `rotation_animation` and `opacity_animation` take `[[values...], duration]`, optionally an easing, then `forever` or `alternate`. They start when the element is built; a running one keeps the panel drawing every frame, and nothing else does.
- Every `*_transition` takes a duration, `[duration, easing]` or `spring(stiffness, damping)`; an easing is `linear`, `ease_in`, `ease_out`, `ease_in_out`, `cubic_bezier(x1, y1, x2, y2)` or a spring.

## Image

`Image { source: "wall.png"; fit: cover }`. `source` resolves like other assets; `fit` is `fill`, `contain` or `cover`; `blur` and `palette` are described under [Wallpapers and the palette](#wallpapers-and-the-palette). Images larger than 3840 × 2400 are shrunk once when loaded, so a surface can upload them. Saving the file, or replacing it, shows the new image; icons follow their files the same way. `on failed` runs when a file does not load, with `self.error`.

## Icon

`Icon { name: "audio-volume-high"; size: 16dp }`. `name` is a theme icon name, an application id (`firefox`, `org.kde.dolphin`) or a path. Names resolve through the user's icon theme (`PALETTES_ICON_THEME`, KDE or GTK settings), its parents and `hicolor`; app ids through installed desktop entries. Paths resolve like other assets. Icons are picked and SVGs rasterized for their size on the densest output (`size: 18dp` at scale 2 is 36 px), again when that scale changes. Icon paths in records (`Toplevel.icon`, `App.icon`) can be passed to `name` as they are.

## Shell states

Declare a state with one of these names and types; Palettes fills it and keeps it current. A service starts only when a state needs it. A state of the wrong type is reported and left alone.

| State | Type | Value |
|---|---|---|
| `output` | `string` | The panel's output name |
| `time` | `string` | Local time formatted by `time_format` (`%H:%M` if empty) |
| `date` | `string` | Local date formatted by `date_format` (`%a %d %b` if empty) |
| `time_format`, `date_format` | `string` | `strftime` patterns, read when the panel opens; seconds make the clock tick every second |
| `workspaces` | `list<Workspace>` | Every workspace, by output |
| `output_workspaces` | `list<Workspace>` | This panel's output's workspaces |
| `windows` | `list<Toplevel>` | Every window, by workspace |
| `focused_title`, `focused_app` | `string` | The focused window's title and app id |
| `focused_icon` | `string` | Its icon file |
| `volume`, `mic_volume` | `float` | Default output / input volume, `1.0` = 100 % |
| `muted`, `mic_muted` | `bool` | |
| `audio_device` | `string` | Default output's description |
| `applications` | `list<App>` | Launchable applications, sorted by name |
| `palette` | `list<string>` | The palette as `#rrggbb`, from the latest `Image { palette: true }`, most common first |
| `battery` | `Battery` | UPower's display device and whether the system runs on battery |
| `power_profile` | `string` | `power-saver`, `balanced` or `performance` (power-profiles-daemon) |
| `power_profiles` | `list<string>` | The profiles to choose from |
| `media` | `Player` | The MPRIS player that last started playing, else the first |
| `players` | `list<Player>` | Every player, by name |
| `media_position` | `float` | `media`'s position in seconds; declaring it updates it each second while playing |
| `notifications` | `list<Notification>` | Notifications, newest first; Palettes is the notification server |
| `notification_popups` | `list<Notification>` | Those still shown as popups: new, for 5 s unless critical |
| `notification_actions` | `list<NotificationAction>` | Their buttons, by notification id |
| `tray` | `list<TrayItem>` | Status notifier items, in registration order |
| `tray_menu` | `list<MenuEntry>` | The menu `host.tray_menu` read, submenus flattened by `depth` |
| `network` | `Network` | NetworkManager's primary connection and Wi-Fi |
| `wifi_networks` | `list<WifiNetwork>` | Networks in range, the active one first, then by strength |
| `bluetooth` | `Bluetooth` | BlueZ's first adapter |
| `bluetooth_devices` | `list<BluetoothDevice>` | Named or paired devices, connected first, then paired |

Workspaces and windows come from Pastel's IPC. Other compositors are read through Wayland protocols, each where offered: ext-workspace-v1 (niri, sway, KWin, …) for workspaces and `host.focus_workspace`, wlr-foreign-toplevel-management (sway, Hyprland, labwc, …) for windows, `host.focus_window` and `host.close_window`. These windows have no workspace (`Toplevel.workspace` is 0, `Workspace.windows` 0), and `host.compositor` is reported as unsupported. Without any of them, these states keep their defaults.

System states each start their D-Bus service on first use, on its own thread and connection, and change only on the service's signals (the position of a playing track aside). The notification server owns `org.freedesktop.Notifications`; while another server owns it, Palettes waits in the bus's queue and takes over when it leaves. The tray owns `org.kde.StatusNotifierWatcher`, or registers as a host with the watcher already running. Icons in these records are files, looked up from the names services report; tray and notification pixels are written as PNG under `$XDG_RUNTIME_DIR/palettes`. A missing service (no BlueZ, say) leaves its states at their defaults. Records are built positionally, one value per field. See [`examples/status.aegle`](../examples/status.aegle), [`traymenu.aegle`](../examples/traymenu.aegle) and [`notifications.aegle`](../examples/notifications.aegle).

```text
use "palettes.aegle"
Panel {
    state battery: Battery = Battery(false, 0.0, "", false, 0, "", false)
    state tray: list<TrayItem> = []
    Row {
        Text { text: str(int(battery.percent * 100.0)) + "%" }
        for item in tray key item.id {
            Icon { name: item.icon; size: 18dp }
            Button { text: item.title; on clicked { host.tray_activate(item.id) } }
        }
    }
}
```

## Records

```text
record Workspace { id: int; name: string; index: int; output: string; active: bool; focused: bool; windows: int }
record Toplevel { id: int; title: string; app_id: string; icon: string; workspace: int; focused: bool }
record App { id: string; name: string; icon: string }
record Battery { present: bool; percent: float; state: string; charging: bool; time: int; icon: string; on_battery: bool }
record Player { name: string; identity: string; icon: string; status: string; playing: bool; title: string; artist: string; album: string; art: string; length: float; can_next: bool; can_previous: bool; can_play: bool; can_seek: bool }
record Notification { id: int; app: string; icon: string; summary: string; body: string; urgency: int; popup: bool; actions: int }
record NotificationAction { id: int; key: string; label: string }
record TrayItem { id: string; title: string; icon: string; status: string; tooltip: string; menu: bool }
record MenuEntry { id: int; label: string; icon: string; enabled: bool; toggle: string; checked: bool; separator: bool; depth: int; submenu: bool }
record Network { kind: string; name: string; connected: bool; strength: int; wifi: bool; wifi_available: bool }
record WifiNetwork { ssid: string; strength: int; secure: bool; active: bool; saved: bool }
record Bluetooth { available: bool; powered: bool; scanning: bool }
record BluetoothDevice { address: string; name: string; icon: string; paired: bool; connected: bool; battery: int }
```

`Workspace.active`: shown on its output. `focused`: shown on the focused output.

`Battery.percent`: `1.0` = 100 %. `state`: `charging`, `discharging`, `full`, `empty`, `pending` or empty. `time`: seconds until empty or full, 0 when unknown. `Player.name`: the bus name after `org.mpris.MediaPlayer2.`; `status`: `Playing`, `Paused` or `Stopped`; `art`: the cover as a local file; `length` in seconds. `Notification.urgency`: 0 low, 1 normal, 2 critical; `icon`: its image, else the app's icon; `actions`: how many buttons it has in `notification_actions`. `TrayItem.status`: `Active`, `Passive` (may be hidden) or `NeedsAttention`. `MenuEntry.toggle`: `checkmark`, `radio` or empty. `Network.kind`: `wifi`, `ethernet`, `vpn`, `other` or empty; `connected`: full connectivity; `strength` 0–100. `BluetoothDevice.battery`: 0–100, or -1 when it reports none.

## Host actions

| Action | Effect |
|---|---|
| `host.spawn(string)` | Run a command line through `sh`, detached |
| `host.launch(string)` | Start an application by desktop id or app id |
| `host.focus_workspace(int)` | Show a workspace (`Workspace.id`) |
| `host.focus_window(int)` | Focus a window (`Toplevel.id`) |
| `host.close_window(int)` | Ask a window to close |
| `host.compositor(string)` | A compositor action, such as Pastel's `"focus-workspace-down"` |
| `host.set_volume(float)`, `host.set_mic_volume(float)` | `0.0`–`1.5` |
| `host.toggle_mute()`, `host.toggle_mic_mute()` | |
| `host.set_power_profile(string)` | One of `power_profiles` |
| `host.media_play_pause()`, `host.media_next()`, `host.media_previous()` | Ask `media` |
| `host.media_seek(float)` | Go to a position in seconds |
| `host.dismiss_notification(int)`, `host.dismiss_notifications()` | Dismiss one or all |
| `host.notification_action(int, string)` | Invoke a notification's action by key (`default`: a click on it); dismisses it unless resident |
| `host.tray_activate(string)`, `host.tray_secondary(string)` | Click or middle-click an item (`TrayItem.id`); an item that is only a menu opens it |
| `host.tray_scroll(string, int)` | Scroll over an item |
| `host.tray_menu(string)` | Read an item's menu into `tray_menu`; then `host.open` a popup showing it |
| `host.tray_select(int)` | Choose a `tray_menu` entry (`MenuEntry.id`), which closes the menu |
| `host.toggle_wifi()`, `host.wifi_scan()`, `host.wifi_disconnect()` | |
| `host.wifi_connect(string, string)` | SSID and password: an empty password uses a saved connection, or joins an open network |
| `host.toggle_bluetooth()`, `host.bluetooth_scan(bool)` | Power the adapter; look for devices |
| `host.bluetooth_connect(string)`, `host.bluetooth_disconnect(string)` | By `BluetoothDevice.address` |
| `host.bluetooth_pair(string)`, `host.bluetooth_forget(string)` | Pair and trust (devices that need a code typed are refused); remove |
| `host.toggle(string)` | Open another document, or close it if open |
| `host.open(string)`, `host.close(string)` | Open a document unless open; close it if open |
| `host.reload()` | Reload every document |
| `host.quit()` | Exit |

Functions a document's script defines are actions too: `host.greet("you")` calls `function greet(name)` in the script. `palettes msg ACTION [ARGUMENT...]` runs any action from the command line, such as `palettes msg toggle launcher.aegle` from a compositor keybinding; see [scripts.md](scripts.md).

## Assets

- Paths resolve against the first document's directory (the working directory), then `$XDG_CONFIG_HOME/palettes`, `$XDG_DATA_HOME/palettes` and each `$XDG_DATA_DIRS/palettes`.
- Font files in any of those roots' `fonts/` register at start and are available by family name.
- Every file a document loads, imports included, is watched; saving one reloads the documents that use it. States keep their values across reloads when their name and type stay.

## Control libraries

Built with `--features am3`, am3's `Md*` elements are available. In Rust, `Documents::with(shell, elements)` takes any library's elements: `palettes::markup::elements::elements().with::<MyElement>()`.
