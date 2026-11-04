//! A service's bus connection and thread: blocking calls that keep the
//! signals arriving meanwhile, and a loop that waits on the bus, a wake pipe
//! for commands, the service's next deadline and a file it watches.

use aegle_dbus::{Connection, Incoming, Kind, Message, Value};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, ErrorKind, Read, Write},
    os::{
        fd::{AsFd, BorrowedFd},
        unix::net::UnixStream,
    },
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// How long a call waits for its reply.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const BUS: &str = "org.freedesktop.DBus";
pub(crate) const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The bus a service connects to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bus {
    /// The user's session bus.
    Session,
    /// The system bus.
    System,
    /// A bus at an address in `DBUS_SESSION_BUS_ADDRESS` form, such as a
    /// private bus in tests.
    Address(String),
}

impl Bus {
    /// An authenticated connection to this bus.
    pub fn connect(&self) -> io::Result<Connection> {
        let timeout = Duration::from_secs(2);
        match self {
            Bus::Session => Connection::session(timeout),
            Bus::System => Connection::system(timeout),
            Bus::Address(address) => Connection::connect(address, timeout),
        }
    }
}

/// The error a remote error reply stands for.
pub(crate) fn remote_error(message: &Message) -> io::Error {
    let text = message.body.first().and_then(Value::as_str).unwrap_or("");
    io::Error::other(format!("{}: {text}", message.error))
}

/// A connection read by one service thread.
pub(crate) struct Client {
    connection: Arc<Connection>,
    incoming: Incoming,
    /// Messages that arrived while a call waited, and complete ones read.
    queue: VecDeque<Message>,
}

/// What [`Client::wait`] returned for.
pub(crate) enum Wake {
    Message(Message),
    /// Commands may be waiting.
    Commands,
    /// The deadline passed.
    Expired,
    /// The watched file changed.
    Ready,
}

impl Client {
    fn new(connection: Arc<Connection>) -> io::Result<Self> {
        let incoming = connection.incoming()?;
        // Reads only happen when the socket is readable or a call waits, so
        // the timeout bounds calls without affecting the loop.
        incoming.stream().set_read_timeout(Some(CALL_TIMEOUT))?;
        Ok(Self {
            connection,
            incoming,
            queue: VecDeque::new(),
        })
    }

    /// This connection's unique name.
    pub fn name(&self) -> &str {
        self.connection.name()
    }

    /// Sends without waiting; an error reply is logged by the loop.
    pub fn send(&self, message: &Message) -> io::Result<u32> {
        self.connection.send(message)
    }

    /// Calls and waits for the reply's values; an error reply is an error.
    pub fn call(&mut self, message: Message) -> io::Result<Vec<Value>> {
        let serial = self.connection.send(&message)?;
        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            while let Some(reply) = self.incoming.take()? {
                if reply.reply_serial == serial && reply.kind != Kind::Call {
                    return match reply.kind {
                        Kind::Error => Err(remote_error(&reply)),
                        _ => Ok(reply.body),
                    };
                }
                self.queue.push_back(reply);
            }
            if Instant::now() >= deadline {
                let what = format!("{} timed out", message.member);
                return Err(io::Error::new(ErrorKind::TimedOut, what));
            }
            if !self.incoming.fill()? {
                return Err(ErrorKind::UnexpectedEof.into());
            }
        }
    }

    /// Calls a method and waits.
    pub fn method(
        &mut self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: Vec<Value>,
    ) -> io::Result<Vec<Value>> {
        self.call(Message::call(destination, path, interface, member, body))
    }

    /// Calls a method without waiting for its reply.
    pub fn invoke(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: Vec<Value>,
    ) {
        let message = Message::call(destination, path, interface, member, body);
        if let Err(e) = self.send(&message) {
            eprintln!("palettes: {destination} {member}: {e}");
        }
    }

    /// Every property of `interface` on an object.
    pub fn get_all(&mut self, destination: &str, path: &str, interface: &str) -> io::Result<Props> {
        let body = vec![Value::str(interface)];
        let reply = self.method(destination, path, PROPERTIES, "GetAll", body)?;
        Ok(reply.first().map(Props::from_dict).unwrap_or_default())
    }

    /// Sets a property without waiting.
    pub fn set(&self, destination: &str, path: &str, interface: &str, name: &str, value: Value) {
        let body = vec![
            Value::str(interface),
            Value::str(name),
            Value::variant(value),
        ];
        self.invoke(destination, path, PROPERTIES, "Set", body);
    }

    /// Adds a match rule without waiting.
    pub fn add_match(&self, rule: &str) -> io::Result<()> {
        self.connection.add_match(rule).map(drop)
    }

    /// Asks for a well-known name: true when this connection owns it now.
    /// With `queue`, it is granted later when its owner leaves
    /// (`NameAcquired`).
    pub fn request_name(&mut self, name: &str, queue: bool) -> io::Result<bool> {
        let flags = if queue { 0 } else { 4 };
        let body = vec![Value::str(name), Value::U32(flags)];
        let reply = self.method(BUS, "/org/freedesktop/DBus", BUS, "RequestName", body)?;
        Ok(matches!(reply.first().and_then(Value::as_u64), Some(1 | 4)))
    }

    /// The unique name owning `name`, if any.
    pub fn owner(&mut self, name: &str) -> Option<String> {
        if name.starts_with(':') {
            return Some(name.to_owned());
        }
        let body = vec![Value::str(name)];
        let reply = (self.method(BUS, "/org/freedesktop/DBus", BUS, "GetNameOwner", body)).ok()?;
        reply.first().and_then(Value::as_str).map(str::to_owned)
    }

    /// The next message, commands, the deadline or a change to `watched`,
    /// whichever is first.
    fn wait(
        &mut self,
        wake: &UnixStream,
        deadline: Option<Instant>,
        watched: Option<BorrowedFd<'_>>,
    ) -> io::Result<Wake> {
        loop {
            while let Some(message) = self.incoming.take()? {
                self.queue.push_back(message);
            }
            if let Some(message) = self.queue.pop_front() {
                return Ok(Wake::Message(message));
            }
            let timeout = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            if timeout == Some(Duration::ZERO) {
                return Ok(Wake::Expired);
            }
            let timeout = timeout.map(|t| Timespec::try_from(t).unwrap_or(Timespec::default()));
            // sysfs attributes announce changes as `POLLPRI`.
            let mut fds = [
                PollFd::new(self.incoming.stream(), PollFlags::IN),
                PollFd::new(wake, PollFlags::IN),
                PollFd::from_borrowed_fd(watched.unwrap_or(wake.as_fd()), PollFlags::PRI),
            ];
            let count = if watched.is_some() { 3 } else { 2 };
            match poll(&mut fds[..count], timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
            if count == 3 && !fds[2].revents().is_empty() {
                return Ok(Wake::Ready);
            }
            let (bus, woken) = (fds[0].revents(), fds[1].revents());
            if !woken.is_empty() {
                let mut bytes = [0; 64];
                while matches!((&*wake).read(&mut bytes), Ok(n) if n > 0) {}
                return Ok(Wake::Commands);
            }
            if !bus.is_empty() && !self.incoming.fill()? {
                return Err(ErrorKind::UnexpectedEof.into());
            }
        }
    }

    /// Whether messages are already waiting, so a batch is not over.
    fn busy(&self) -> bool {
        !self.queue.is_empty()
    }
}

/// A service running on its own thread with its own connection.
pub(crate) trait Service: Send + 'static {
    /// Requests from the UI thread.
    type Command: Send + 'static;

    /// Reads the first state and adds match rules.
    fn start(&mut self, bus: &mut Client) -> io::Result<()>;

    /// A signal or a call to this connection.
    fn message(&mut self, bus: &mut Client, message: Message);

    /// A request.
    fn command(&mut self, bus: &mut Client, command: Self::Command);

    /// When [`Self::expired`] should run next.
    fn deadline(&self) -> Option<Instant> {
        None
    }

    /// The deadline passed.
    fn expired(&mut self, _bus: &mut Client) {}

    /// A file to wait on beside the bus, such as a sysfs attribute.
    fn watched(&self) -> Option<BorrowedFd<'_>> {
        None
    }

    /// The watched file changed.
    fn ready(&mut self, _bus: &mut Client) {}

    /// After each batch of messages and commands: report what changed.
    fn settle(&mut self, bus: &mut Client);
}

/// Hands commands to a service thread; dropping it ends the thread.
pub(crate) struct Handle<C> {
    commands: mpsc::Sender<C>,
    wake: UnixStream,
}

impl<C> Handle<C> {
    pub fn send(&self, command: C) {
        if self.commands.send(command).is_ok() {
            // One byte is enough; a full pipe already wakes the thread.
            let _ = (&self.wake).write(&[1]);
        }
    }
}

/// Connects to `bus` and runs `service` on a thread named after it.
pub(crate) fn spawn<S: Service>(
    name: &'static str,
    bus: &Bus,
    service: S,
) -> io::Result<Handle<S::Command>> {
    let client = Client::new(Arc::new(bus.connect()?))?;
    let (wake, woken) = UnixStream::pair()?;
    wake.set_nonblocking(true)?;
    woken.set_nonblocking(true)?;
    let (commands, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("palettes-{name}"))
        .spawn(move || run(name, service, client, woken, receiver))?;
    Ok(Handle { commands, wake })
}

fn run<S: Service>(
    name: &str,
    mut service: S,
    mut bus: Client,
    woken: UnixStream,
    commands: mpsc::Receiver<S::Command>,
) {
    if let Err(e) = service.start(&mut bus) {
        eprintln!("palettes: {name}: {e}");
        return;
    }
    service.settle(&mut bus);
    loop {
        match bus.wait(&woken, service.deadline(), service.watched()) {
            Ok(Wake::Message(message)) => match message.kind {
                Kind::Signal | Kind::Call => service.message(&mut bus, message),
                Kind::Error => eprintln!("palettes: {name}: {}", remote_error(&message)),
                Kind::Return => {}
            },
            Ok(Wake::Commands) => loop {
                match commands.try_recv() {
                    Ok(command) => service.command(&mut bus, command),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            },
            Ok(Wake::Expired) => service.expired(&mut bus),
            Ok(Wake::Ready) => service.ready(&mut bus),
            Err(e) => {
                eprintln!("palettes: {name}: {e}");
                return;
            }
        }
        if !bus.busy() {
            service.settle(&mut bus);
        }
    }
}

/// The properties of one interface of an object, variants unwrapped.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Props(HashMap<String, Value>);

impl Props {
    /// From an `a{sv}` dictionary.
    pub fn from_dict(dict: &Value) -> Self {
        let mut props = Self::default();
        props.merge(dict);
        props
    }

    /// Takes the entries of an `a{sv}` dictionary.
    pub fn merge(&mut self, dict: &Value) {
        for item in dict.items() {
            if let Value::Entry(entry) = item
                && let Some(key) = entry.0.as_str()
            {
                let value = match &entry.1 {
                    Value::Variant(inner) => (**inner).clone(),
                    value => value.clone(),
                };
                self.0.insert(key.to_owned(), value);
            }
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn str(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or("")
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn u64(&self, key: &str) -> u64 {
        self.get(key).and_then(Value::as_u64).unwrap_or(0)
    }

    pub fn i64(&self, key: &str) -> i64 {
        self.get(key).and_then(Value::as_i64).unwrap_or(0)
    }

    pub fn f64(&self, key: &str) -> f64 {
        self.get(key).and_then(Value::as_f64).unwrap_or(0.0)
    }

    /// The items of an array property.
    pub fn items(&self, key: &str) -> &[Value] {
        self.get(key).map(Value::items).unwrap_or(&[])
    }
}

/// A `PropertiesChanged` signal's interface, changed values and
/// invalidated names.
pub(crate) fn properties_changed(message: &Message) -> Option<(&str, &Value, Vec<&str>)> {
    if !message.is_signal(PROPERTIES, "PropertiesChanged") {
        return None;
    }
    let [interface, changed, invalidated] = &message.body[..] else {
        return None;
    };
    let invalidated = invalidated.items().iter().filter_map(Value::as_str);
    Some((interface.as_str()?, changed, invalidated.collect()))
}

/// The text of a byte array, such as a Wi-Fi SSID.
pub(crate) fn bytes_text(value: Option<&Value>) -> String {
    let bytes: Vec<u8> = value
        .map(Value::items)
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| match b {
            Value::Byte(b) => Some(*b),
            _ => None,
        })
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A byte array value.
pub(crate) fn byte_array(bytes: &[u8]) -> Value {
    Value::Array("y".into(), bytes.iter().map(|b| Value::Byte(*b)).collect())
}
