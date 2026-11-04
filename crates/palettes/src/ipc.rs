//! `palettes msg`: host actions from other programs, such as compositor
//! keybindings, over a Unix socket. A request is one line holding a JSON
//! array of strings, the action and its arguments; the answer is one line,
//! `{"ok":"text"}` or `{"error":"message"}`.

use crate::Result;
use palettes_shell::Shell;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

/// `$PALETTES_SOCKET`, else `palettes-$WAYLAND_DISPLAY.sock` in
/// `$XDG_RUNTIME_DIR`.
pub fn socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PALETTES_SOCKET") {
        return Some(path.into());
    }
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    // A display given as a path names its socket file.
    let display = display.rsplit('/').next().unwrap_or(&display).to_owned();
    Some(dir.join(format!("palettes-{display}.sock")))
}

type Reply = mpsc::Sender<std::result::Result<String, String>>;

/// Answers requests on the socket, running each action on the shell's
/// thread. Another running instance keeps the socket.
pub(crate) fn serve(shell: &Shell) {
    let Some(path) = socket() else { return };
    if UnixStream::connect(&path).is_ok() {
        eprintln!(
            "palettes: {} belongs to another instance; not taking messages",
            path.display()
        );
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("palettes: no messages at {}: {e}", path.display());
            return;
        }
    };
    let requests = shell.channel(|_, (request, reply): (Vec<String>, Reply)| {
        let answer = match request.split_first() {
            Some((name, arguments)) => {
                crate::markup::actions::call_text(name, arguments).map_err(|e| e.to_string())
            }
            None => Err("an empty request".into()),
        };
        let _ = reply.send(answer);
    });
    let spawned = std::thread::Builder::new()
        .name("palettes-ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let requests = requests.clone();
                // A slow client holds only its own thread.
                let _ = std::thread::Builder::new()
                    .name("palettes-ipc-client".into())
                    .spawn(move || {
                        answer(stream, |request| {
                            let (reply, answer) = mpsc::channel();
                            requests.send((request, reply));
                            answer
                                .recv()
                                .unwrap_or_else(|_| Err("the shell stopped".into()))
                        })
                    });
            }
        });
    if let Err(e) = spawned {
        eprintln!("palettes: no messages: {e}");
    }
}

fn answer(
    stream: UnixStream,
    mut run: impl FnMut(Vec<String>) -> std::result::Result<String, String>,
) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let answer = match serde_json::from_str::<Vec<String>>(&line) {
            Ok(request) => run(request),
            Err(e) => Err(format!("not a JSON array of strings: {e}")),
        };
        let json = match answer {
            Ok(text) => serde_json::json!({ "ok": text }),
            Err(message) => serde_json::json!({ "error": message }),
        };
        if writeln!(writer, "{json}").is_err() {
            return;
        }
    }
}

/// Runs the action `request[0]` with the rest as arguments in the running
/// shell; returns what it answered.
pub fn send(request: &[String]) -> Result<String> {
    let path = socket().ok_or("XDG_RUNTIME_DIR is not set")?;
    let mut stream =
        UnixStream::connect(&path).map_err(|e| format!("no shell at {}: {e}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    writeln!(stream, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let answer: serde_json::Value = serde_json::from_str(&line)?;
    match (answer.get("ok"), answer.get("error")) {
        (Some(ok), _) => Ok(ok.as_str().unwrap_or_default().to_owned()),
        (_, Some(error)) => Err(error.as_str().unwrap_or("failed").into()),
        _ => Err("an answer without ok or error".into()),
    }
}
