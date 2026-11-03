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

Layout properties (`padding`, `gap`, `background`, …) apply as on `Column`.

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

Workspaces and windows come from the compositor's IPC (Pastel). Elsewhere these states keep their defaults.

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
