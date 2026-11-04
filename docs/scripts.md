# Scripts

A [Luau](https://luau.org) file beside a document runs while the document is open: `bar.luau` with `bar.aegle`. Markup draws, the script supplies data and logic. Saving the script runs it again; `palettes --check bar.aegle` compiles it without running it.

```text
// bar.aegle
Panel {
    state cpu: float = 0.0
    state greeting: string = ""
    Text { text: greeting + " " + str(int(cpu * 100.0)) + "%" }
    Button { text: "Hi"; on clicked { host.greet("you") } }
}
```

```lua
-- bar.luau
state.greeting = "Hello"

function greet(name)
    state.greeting = `Hello, {name}`
end

every(2, function()
    local load = tonumber((read("/proc/loadavg") or ""):match("^[%d.]+"))
    state.cpu = (load or 0) / 8
end)
```

## States

`state.name` reads and assigns the state `name` of every open document that declares it. A value a script assigns reaches documents opened later too, so scripts share data with popups and with other documents. Reading gives the latest value, whether a script set it or the document changed it (typing into a `TextField` bound to it, for example).

Values convert by the state's declared type: numbers for `int` and `float` (rounded for `int`), strings, booleans, arrays for `list<T>`, and tables keyed by field name for records. A missing value or field is the type's default (`0`, `""`, `false`, empty).

`watch(name, function(value) ... end)` runs each time the state changes.

## Actions

A function a script defines at its top level is a host action: markup calls it as `host.name(...)`, other scripts as `host.name(...)`, and the command line as `palettes msg name ...`. Palettes' own actions (`host.spawn`, `host.toggle`, `host.set_volume`, … in [markup.md](markup.md)) are callable from scripts the same way. Palettes' actions take precedence over script functions of the same name.

From `palettes msg`, a script function gets each argument as a number or boolean when it reads as one, else as a string, and what it returns is printed: strings as they are, tables as JSON.

## Built-ins

| Function | Effect |
|---|---|
| `exec(command, function(output, status))` | Runs a command line through `sh`; calls back with its standard output and exit status once it exits. The callback is optional |
| `lines(command, function(line))` | Runs a command and calls back with each line it writes, such as `pactl subscribe` or `tail -f`. Returns a handle |
| `read(path)` | The file's text, or `nil` |
| `file(path, function(text))` | Calls back now and whenever the file is saved, with its text (`nil` if it is gone) |
| `after(seconds, function())` | Calls back once. Returns a handle |
| `every(seconds, function())` | Calls back every `seconds`. Returns a handle |
| `json.decode(text)`, `json.encode(value)` | JSON to Luau values and back |
| `print(...)` | Writes to standard error, prefixed with the script's name |

`handle:stop()` ends a command or timer early. Unloading a script, by closing its document or saving it, stops its commands (their whole process group), timers and watches. Relative paths start at the script's directory; `~/` is the home directory.

Luau's own libraries (`string`, `table`, `math`, `os.date`, `os.time`, …) are there; there is no `io` or `os.execute`, use `read`, `file` and `exec`.

Nothing runs between events: a script without timers, commands or watches costs nothing after its top-level code.

## From the command line

`palettes msg ACTION [ARGUMENT...]` runs an action in the running shell, so compositor keybindings can open launchers and drive scripts:

```sh
palettes msg toggle launcher.aegle
palettes msg set_volume 0.5
palettes msg greet you
```

The socket is `$XDG_RUNTIME_DIR/palettes-$WAYLAND_DISPLAY.sock` (or `$PALETTES_SOCKET`). A request is one line holding a JSON array of strings; the answer is one line, `{"ok":"text"}` or `{"error":"message"}`.

## In Rust

Rust shells use Rust directly; the runner and its scripts are the same engine (`palettes::markup::Documents`), so a Rust program can open documents with scripts beside them as well.
