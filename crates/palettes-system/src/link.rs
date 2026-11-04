//! A plain bus connection whose messages go to one callback, for scripts
//! that call and follow services themselves.

use crate::client::Bus;
use aegle_dbus::{Connection, Kind, Message};
use std::{io, sync::Arc};

/// A connection read on its own thread. Every reply, error and signal it
/// receives goes to the callback given to [`Link::open`]; method calls to it
/// are answered as unknown.
#[derive(Clone)]
pub struct Link(Arc<Connection>);

impl Link {
    /// Connects to `bus`; `received` runs on the reading thread.
    pub fn open(bus: &Bus, mut received: impl FnMut(Message) + Send + 'static) -> io::Result<Self> {
        let connection = Arc::new(bus.connect()?);
        let mut incoming = connection.incoming()?;
        let replier = connection.clone();
        std::thread::Builder::new()
            .name("palettes-dbus".into())
            .spawn(move || {
                while let Ok(message) = incoming.recv() {
                    if message.kind == Kind::Call {
                        if !message.no_reply {
                            let error = "org.freedesktop.DBus.Error.UnknownMethod";
                            let _ = replier.send(&message.error(error, "no such method"));
                        }
                        continue;
                    }
                    received(message);
                }
            })?;
        Ok(Self(connection))
    }

    /// Sends a message; its serial, which its reply answers.
    pub fn send(&self, message: &Message) -> io::Result<u32> {
        self.0.send(message)
    }

    /// Asks the bus for signals matching `rule`.
    pub fn add_match(&self, rule: &str) -> io::Result<()> {
        self.0.add_match(rule).map(drop)
    }

    /// Stops the signals of a rule added before.
    pub fn remove_match(&self, rule: &str) {
        let bus = "org.freedesktop.DBus";
        let body = vec![aegle_dbus::Value::str(rule)];
        let remove = Message::call(bus, "/org/freedesktop/DBus", bus, "RemoveMatch", body);
        let _ = self.0.send(&remove);
    }
}
