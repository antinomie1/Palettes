use palettes_workspace::{Command, Compositor, Desktop};
use pastel_ipc::{Reply, Request, decode, encode};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::Duration;

fn state() -> pastel_ipc::State {
    pastel_ipc::State {
        outputs: vec![pastel_ipc::Output {
            name: "DP-1".into(),
            focused: true,
            rect: [0, 0, 1920, 1080],
        }],
        workspaces: vec![
            pastel_ipc::Workspace {
                id: 3,
                index: 1,
                output: "DP-1".into(),
                active: true,
                focused: true,
                windows: 1,
            },
            pastel_ipc::Workspace {
                id: 4,
                index: 2,
                output: "DP-1".into(),
                active: false,
                focused: false,
                windows: 0,
            },
        ],
        windows: vec![pastel_ipc::Window {
            id: 9,
            title: "htop".into(),
            app_id: "foot".into(),
            workspace: 3,
            focused: true,
            floating: false,
            fullscreen: false,
        }],
    }
}

/// A fake Pastel: answers a subscription with one state, records requests.
#[test]
fn follows_and_drives_pastel() {
    let dir = std::env::temp_dir().join(format!("palettes-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pastel.sock");
    let listener = UnixListener::bind(&path).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var(pastel_ipc::SOCKET_ENV, &path) };
    let (requests, received) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let requests = requests.clone();
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines() {
                    let request: Request = decode(&line.unwrap()).unwrap();
                    let reply = match &request {
                        Request::Subscribe => Reply::State(state()),
                        _ => Reply::Ok,
                    };
                    writer.write_all(encode(&reply).as_bytes()).unwrap();
                    if request == Request::Subscribe {
                        return; // then hang up: the compositor went away
                    }
                    requests.send(request).unwrap();
                }
            });
        }
    });

    let compositor = Compositor::connect().unwrap();
    let (desktops, seen) = mpsc::channel();
    compositor
        .subscribe(move |desktop| desktops.send(desktop).unwrap())
        .unwrap();
    let desktop: Desktop = seen.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(desktop.focused_window().unwrap().title, "htop");
    assert_eq!(desktop.focused_workspace().unwrap().name, "1");
    assert_eq!(desktop.workspaces_on("DP-1").count(), 2);
    // Hanging up ends with an empty desktop.
    assert_eq!(
        seen.recv_timeout(Duration::from_secs(2)).unwrap(),
        Desktop::default()
    );

    compositor.focus_workspace(4);
    compositor.send(Command::Action("spawn foot".into()));
    let timeout = Duration::from_secs(2);
    assert_eq!(
        received.recv_timeout(timeout).unwrap(),
        Request::FocusWorkspace(4)
    );
    assert_eq!(
        received.recv_timeout(timeout).unwrap(),
        Request::Action("spawn foot".into())
    );
    let _ = std::fs::remove_dir_all(dir);
}
