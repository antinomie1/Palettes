# Panel documents

A panel document is an aegle markup file whose root is a `Panel`. `palettes FILE...` (or `palettes::run`) opens each document on every output, or only on the one it names, and reloads it in place when any of its files is saved. Aegle's markup language itself (bindings, `if`, `for`, components, records) is described in aegle's `docs/markup.md`.

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
| `edge` | `top` `bottom` `left` `right` `center` `background` | `top` | Where it sits. `center` is an overlay; `background` covers the output below windows |
| `size` | length | `32dp` | Bar thickness; height of a `center` panel |
| `span` | length | `0` | Length along the edge (width of a `center` panel); `0` stretches. A bar with a span is a dock |
| `layer` | `background` `bottom` `top` `overlay` | by edge | Stacking layer |
| `exclusive` | bool | bars: true | Reserve its space so windows avoid it |
| `keyboard` | `none` `exclusive` `on_demand` | `none` | Keyboard focus policy; launchers use `exclusive` |
| `offset` | length | `0` | Distance from the anchored edges |
| `output` | line | `""` | Only this output (`DP-1`); empty for every output |
| `blur` | bool | `false` | The compositor blurs the desktop behind the panel's rounded shape |

Layout properties (`padding`, `gap`, `background`, …) apply as on `Column`.

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

`Image { source: "wall.png"; fit: cover }`. `source` resolves like other assets; `fit` is `fill`, `contain` or `cover`. Images larger than one megapixel are shrunk once when loaded, so a surface can upload them. Saving the file, or replacing it, shows the new image; icons follow their files the same way. `on failed` runs when a file does not load, with `self.error`.

## Icon

`Icon { name: "audio-volume-high"; size: 16dp }`. `name` is a theme icon name, an application id (`firefox`, `org.kde.dolphin`) or a path. Names resolve through the user's icon theme (`PALETTES_ICON_THEME`, KDE or GTK settings), its parents and `hicolor`; app ids through installed desktop entries. Paths resolve like other assets. SVG icons are rasterized. Icon paths in records (`Toplevel.icon`, `App.icon`) can be passed to `name` as they are.

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

Workspaces and windows come from Pastel's IPC. Other compositors are read through Wayland protocols, each where offered: ext-workspace-v1 (niri, sway, KWin, …) for workspaces and `host.focus_workspace`, wlr-foreign-toplevel-management (sway, Hyprland, labwc, …) for windows, `host.focus_window` and `host.close_window`. These windows have no workspace (`Toplevel.workspace` is 0, `Workspace.windows` 0), and `host.compositor` is reported as unsupported. Without any of them, these states keep their defaults.

## Records

```text
record Workspace { id: int; name: string; index: int; output: string; active: bool; focused: bool; windows: int }
record Toplevel { id: int; title: string; app_id: string; icon: string; workspace: int; focused: bool }
record App { id: string; name: string; icon: string }
```

`Workspace.active`: shown on its output. `focused`: shown on the focused output.

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
| `host.toggle(string)` | Open another document, or close it if open |
| `host.reload()` | Reload every document |
| `host.quit()` | Exit |

## Assets

- Paths resolve against the first document's directory (the working directory), then `$XDG_CONFIG_HOME/palettes`, `$XDG_DATA_HOME/palettes` and each `$XDG_DATA_DIRS/palettes`.
- Font files in any of those roots' `fonts/` register at start and are available by family name.
- Every file a document loads, imports included, is watched; saving one reloads the documents that use it. States keep their values across reloads when their name and type stay.

## Control libraries

Built with `--features am3`, am3's `Md*` elements are available. In Rust, `Documents::with(shell, elements)` takes any library's elements: `palettes::markup::elements::elements().with::<MyElement>()`.
