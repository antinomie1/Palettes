//! A private bus daemon and fake services on it.
#![allow(dead_code)]

use palettes_system::{Bus, Kind, Message, Report, Value};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// A `dbus-daemon` of its own; killed when dropped.
pub struct Daemon {
    child: Child,
    dir: PathBuf,
    pub bus: Bus,
    pub address: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A private bus for test `name`, or `None` without `dbus-daemon`.
pub fn daemon(name: &str) -> Option<Daemon> {
    let dir = std::env::temp_dir().join(format!("palettes-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let Ok(mut child) = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .arg(format!("--address=unix:path={}/bus", dir.display()))
        .stdout(Stdio::piped())
        .spawn()
    else {
        eprintln!("no dbus-daemon; skipped");
        return None;
    };
    let mut address = String::new();
    BufReader::new(child.stdout.take()?)
        .read_line(&mut address)
        .ok()?;
    let address = address.trim().to_owned();
    Some(Daemon {
        child,
        dir,
        bus: Bus::Address(address.clone()),
        address,
    })
}

/// A fake service: owns names and answers calls with `answer`, which
/// returns the reply values or `None` for an unknown method. Every call is
/// also sent to `calls`.
pub struct Fake {
    pub connection: Arc<aegle_dbus::Connection>,
    pub calls: mpsc::Receiver<Message>,
}

impl Fake {
    pub fn serve(
        daemon: &Daemon,
        names: &[&str],
        answer: impl Fn(&Message) -> Option<Vec<Value>> + Send + 'static,
    ) -> Self {
        let connection = Arc::new(
            aegle_dbus::Connection::connect(&daemon.address, Duration::from_secs(2)).unwrap(),
        );
        let mut incoming = connection.incoming().unwrap();
        for name in names {
            let bus = "org.freedesktop.DBus";
            let body = vec![Value::str(*name), Value::U32(4)];
            let serial = connection
                .send(&Message::call(
                    bus,
                    "/org/freedesktop/DBus",
                    bus,
                    "RequestName",
                    body,
                ))
                .unwrap();
            while incoming.recv().unwrap().reply_serial != serial {}
        }
        let (sender, calls) = mpsc::channel();
        let replier = connection.clone();
        std::thread::spawn(move || {
            while let Ok(message) = incoming.recv() {
                if message.kind != Kind::Call {
                    continue;
                }
                let reply = match answer(&message) {
                    Some(body) => message.reply(body),
                    None => message.error("org.freedesktop.DBus.Error.UnknownMethod", "unknown"),
                };
                if !message.no_reply {
                    let _ = replier.send(&reply);
                }
                let _ = sender.send(message);
            }
        });
        Self { connection, calls }
    }

    pub fn signal(&self, path: &str, interface: &str, member: &str, body: Vec<Value>) {
        self.connection
            .send(&Message::signal(path, interface, member, body))
            .unwrap();
    }

    /// Waits for a call of `member`.
    pub fn called(&self, member: &str) -> Message {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.calls.recv_timeout(left) {
                Ok(call) if call.member == member => return call,
                Ok(_) => {}
                Err(_) => panic!("no call of {member}"),
            }
        }
    }

    /// Leaves the bus.
    pub fn close(&self) {
        let incoming = self.connection.incoming().unwrap();
        incoming
            .stream()
            .shutdown(std::net::Shutdown::Both)
            .unwrap();
    }
}

/// One object's interface and its properties.
pub type Object<'a> = (&'a str, &'a str, Vec<(&'a str, Value)>);

/// Answers `GetAll` and `Get` from `props`: `(path, interface, entries)`.
pub fn properties(call: &Message, props: &[Object<'_>]) -> Option<Vec<Value>> {
    let interface = call.body.first()?.as_str()?;
    let (_, _, entries) = props
        .iter()
        .find(|(p, i, _)| *p == call.path && *i == interface)?;
    match call.member.as_str() {
        "GetAll" => Some(vec![Value::dict(
            entries.iter().map(|(k, v)| (*k, v.clone())),
        )]),
        "Get" => {
            let name = call.body.get(1)?.as_str()?;
            let (_, value) = entries.iter().find(|(k, _)| *k == name)?;
            Some(vec![Value::variant(value.clone())])
        }
        _ => None,
    }
}

/// A `PropertiesChanged` body.
pub fn props_changed(interface: &str, entries: Vec<(&str, Value)>) -> Vec<Value> {
    vec![
        Value::str(interface),
        Value::dict(entries),
        Value::strings([]),
    ]
}

/// The first state from `states` that passes `test`, within five seconds.
pub fn until<T: std::fmt::Debug>(states: &mpsc::Receiver<T>, test: impl Fn(&T) -> bool) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match states.recv_timeout(left) {
            Ok(state) if test(&state) => return state,
            Ok(state) => last = Some(state),
            Err(_) => panic!("no such state; last {last:#?}"),
        }
    }
}

/// A callback sending each state to the returned receiver.
pub fn reports<T: Clone + Send + 'static>() -> (Report<T>, mpsc::Receiver<T>) {
    let (sender, receiver) = mpsc::channel();
    (
        Box::new(move |state: &T| drop(sender.send(state.clone()))),
        receiver,
    )
}

/// A client of the daemon, reading its own replies and signals.
pub struct Peer {
    pub connection: aegle_dbus::Connection,
    incoming: aegle_dbus::Incoming,
}

impl Peer {
    pub fn new(daemon: &Daemon) -> Self {
        let connection =
            aegle_dbus::Connection::connect(&daemon.address, Duration::from_secs(2)).unwrap();
        let incoming = connection.incoming().unwrap();
        Self {
            connection,
            incoming,
        }
    }

    /// Calls and waits for the reply.
    pub fn call(&mut self, message: Message) -> Message {
        let serial = self.connection.send(&message).unwrap();
        loop {
            let reply = self.incoming.recv().unwrap();
            if reply.reply_serial == serial {
                return reply;
            }
        }
    }

    /// Waits for the signal `member`.
    pub fn signal(&mut self, member: &str) -> Message {
        loop {
            let m = self.incoming.recv().unwrap();
            if m.kind == Kind::Signal && m.member == member {
                return m;
            }
        }
    }
}
