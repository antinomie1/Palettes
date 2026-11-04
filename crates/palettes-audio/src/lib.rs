//! Audio devices and application streams through PipeWire.
//!
//! [`Audio::spawn`] runs a PipeWire loop on its own thread. It follows every
//! sink, source and playing stream, the session manager's default sink and
//! source (the `default` metadata) and their `Props` (channel volumes,
//! mute), and calls back with an [`AudioState`] only when what a shell shows
//! changes. Requests such as [`Audio::set_volume`] are handed to that
//! thread. Volumes are on the cubic scale mixers show: 1.0 is 100 %, above
//! it amplifies.

mod peak;
mod pipewire_loop;

use pipewire::channel;

/// What a shell shows about audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    /// The default output.
    pub sink: Option<Endpoint>,
    /// The default input.
    pub source: Option<Endpoint>,
    /// Every output, by description.
    pub sinks: Vec<Endpoint>,
    /// Every input, by description; monitors of outputs are left out.
    pub sources: Vec<Endpoint>,
    /// Applications playing sound, by application.
    pub streams: Vec<Stream>,
    /// The loudest sample of the default output's last buffer, 0–1, about
    /// 25 times a second once [`Audio::track_peak`] was called; else 0.
    pub peak: f32,
}

/// An application's playback stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    /// PipeWire's id for its node.
    pub id: u32,
    /// The application's name.
    pub app: String,
    /// The application's icon name, if it gave one.
    pub icon: String,
    /// What it plays, such as a track's title.
    pub title: String,
    /// Average channel volume on the cubic scale.
    pub volume: f32,
    /// Muted.
    pub muted: bool,
}

/// A sink or source.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    /// PipeWire `node.name`.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Average channel volume on the cubic scale, 1.0 being 100 %.
    pub volume: f32,
    /// Muted.
    pub muted: bool,
}

/// The default output or the default input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The default sink.
    Sink,
    /// The default source.
    Source,
}

/// What a volume or mute request changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    Default(Target),
    Stream(u32),
}

pub(crate) enum Command {
    Volume(Node, f32),
    Mute(Node, Option<bool>),
    /// Makes the sink (or source) of this `node.name` the default.
    Default(Target, String),
    Peak,
}

fn volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.5)
    } else {
        0.0
    }
}

/// A running audio service; dropping it leaves the thread running for the
/// rest of the process, like the PipeWire connection it holds.
pub struct Audio {
    commands: channel::Sender<Command>,
}

impl Audio {
    /// Connects to PipeWire on a new thread; `changed` runs there with the
    /// state once known and after each change.
    pub fn spawn(changed: impl FnMut(&AudioState) + Send + 'static) -> std::io::Result<Self> {
        let (commands, receiver) = channel::channel();
        let (ready, connected) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("palettes-audio".into())
            .spawn(move || pipewire_loop::run(receiver, changed, ready))?;
        match connected.recv() {
            Ok(Ok(())) => Ok(Self { commands }),
            Ok(Err(e)) => Err(std::io::Error::other(e)),
            Err(_) => Err(std::io::Error::other("the PipeWire thread ended")),
        }
    }

    /// Sets the volume of `target`, clamped to 0–1.5.
    pub fn set_volume(&self, target: Target, level: f32) {
        let _ = (self.commands).send(Command::Volume(Node::Default(target), volume(level)));
    }

    /// Mutes or unmutes `target`.
    pub fn set_muted(&self, target: Target, muted: bool) {
        let _ = (self.commands).send(Command::Mute(Node::Default(target), Some(muted)));
    }

    /// Flips `target`'s mute.
    pub fn toggle_mute(&self, target: Target) {
        let _ = (self.commands).send(Command::Mute(Node::Default(target), None));
    }

    /// Makes the sink or source named `name` ([`Endpoint::name`]) the default.
    pub fn set_default(&self, target: Target, name: &str) {
        let _ = (self.commands).send(Command::Default(target, name.to_owned()));
    }

    /// Sets the volume of stream `id`, clamped to 0–1.5.
    pub fn set_stream_volume(&self, id: u32, level: f32) {
        let _ = (self.commands).send(Command::Volume(Node::Stream(id), volume(level)));
    }

    /// Mutes or unmutes stream `id`; `None` flips it.
    pub fn set_stream_muted(&self, id: u32, muted: Option<bool>) {
        let _ = (self.commands).send(Command::Mute(Node::Stream(id), muted));
    }

    /// Starts measuring [`AudioState::peak`]: a capture of the default
    /// output, which runs from then on.
    pub fn track_peak(&self) {
        let _ = self.commands.send(Command::Peak);
    }
}

/// The value of `"name"` in a metadata value such as `{"name":"alsa_output.x"}`.
pub fn default_name(json: &str) -> Option<&str> {
    let start = json.find("\"name\"")? + 6;
    let rest = json[start..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    rest.split('"').next()
}
