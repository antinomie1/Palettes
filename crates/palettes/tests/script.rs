//! Scripts beside documents: checking, shared states, script actions,
//! commands, timers, file watches and `palettes msg`, live where a Wayland
//! session exists.

use palettes::{Shell, ipc, markup};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("palettes-script-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

const COUNTER: &str = r#"record Item { name: string; size: int }
Panel {
    state label: string = ""
    state count: int = 0
    state items: list<Item> = []
    Text { text: label + " " + str(count) }
    Button { text: "+"; on clicked { host.bump(2) } }
}
"#;

#[test]
fn documents_call_the_functions_their_script_defines() {
    let dir = dir("check");
    let doc = write(&dir, "counter.aegle", COUNTER);
    // Without a script, `host.bump` is no action.
    assert!(markup::check(&doc).is_err());
    write(
        &dir,
        "counter.luau",
        "function bump(n) state.count += n end\n",
    );
    markup::check(&doc).unwrap();
    // A script that does not compile fails the check.
    write(
        &dir,
        "counter.luau",
        "function bump(n) state.count += end\n",
    );
    let error = markup::check(&doc).unwrap_err().to_string();
    assert!(error.contains("counter.luau"), "{error}");
}

#[test]
fn scripts_share_states_run_commands_and_answer_messages() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("skipped: no Wayland session");
        return;
    }
    let dir = dir("live");
    let socket = dir.join("ipc.sock");
    // SAFETY: the only test in this binary that reads the environment.
    unsafe { std::env::set_var("PALETTES_SOCKET", &socket) };
    let doc = write(&dir, "counter.aegle", COUNTER);
    write(&dir, "data.json", r#"{"name": "a", "size": 3}"#);
    write(
        &dir,
        "counter.luau",
        r#"
state.label = "hi"
local seen = 0
watch("count", function(value) seen += 1 end)
function bump(n) state.count += n end
function report()
    return `{state.label}|{state.count}|{seen}|{ticks or 0}|{#state.items}`
end
exec("echo out; exit 3", function(text, status)
    state.label = `{text:gsub("%s+$", "")}{status}`
end)
local timer
timer = every(0.02, function()
    ticks = (ticks or 0) + 1
    if ticks == 3 then timer:stop() end
end)
file("data.json", function(text)
    if text then state.items = { json.decode(text) } end
end)
"#,
    );
    let shell = Shell::new().unwrap();
    let documents = markup::Documents::with(shell.clone(), markup::elements::elements())
        .unwrap()
        .open(&doc)
        .unwrap();
    let ask = |request: &[&str]| {
        let request: Vec<String> = request.iter().map(|s| s.to_string()).collect();
        let (shell, deadline) = (shell.clone(), Instant::now() + Duration::from_secs(5));
        let asking = std::thread::spawn(move || ipc::send(&request).map_err(|e| e.to_string()));
        while !asking.is_finished() && Instant::now() < deadline {
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        }
        asking.join().unwrap()
    };
    let settle = |time: u64| {
        let end = Instant::now() + Duration::from_millis(time);
        while Instant::now() < end {
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        }
    };
    settle(300);
    assert_eq!(ask(&["bump", "5"]), Ok(String::new()));
    // The command's output, the bump and its watch, three ticks, one item.
    assert_eq!(ask(&["report"]).unwrap(), "out3|5|1|3|1");
    // Palettes actions parse their arguments by type.
    let error = ask(&["set_volume", "loud"]).unwrap_err();
    assert!(error.contains("not float"), "{error}");
    assert!(ask(&["nothing"]).unwrap_err().contains("no action"));
    // Documents open and close by name; a popup needs a pointer to open by.
    write(
        &dir,
        "menu.aegle",
        "Panel { edge: popup; size: 120dp; Text { text: \"menu\" } }",
    );
    let error = ask(&["toggle", "menu.aegle"]).unwrap_err();
    assert!(error.contains("pointer"), "{error}");
    assert_eq!(ask(&["close", "counter.aegle"]), Ok(String::new()));
    assert_eq!(ask(&["open", "counter.aegle"]), Ok(String::new()));
    // Saving the script runs it again; states stay.
    write(
        &dir,
        "counter.luau",
        "function report() return `{state.count}` end\n",
    );
    settle(300);
    assert_eq!(ask(&["report"]).unwrap(), "5");
    drop(documents);
}
