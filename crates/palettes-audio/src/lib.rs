//! The default audio output and input through PipeWire.
//!
//! [`Audio::spawn`] runs a PipeWire loop on its own thread. It follows the
//! session manager's default sink and source (the `default` metadata) and their
//! `Props` (channel volumes, mute), and calls back with an [`AudioState`] only
//! when what a shell shows changes. Requests such as [`Audio::set_volume`] are
//! handed to that thread. Volumes are on the cubic scale mixers show: 1.0 is
//! 100 %, above it amplifies.

mod pipewire_loop;

use pipewire::channel;

/// What a shell shows about audio.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    /// The default output.
    pub sink: Option<Endpoint>,
    /// The default input.
    pub source: Option<Endpoint>,
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

pub(crate) enum Command {
    Volume(Target, f32),
    Mute(Target, Option<bool>),
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
    pub fn set_volume(&self, target: Target, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.5)
        } else {
            0.0
        };
        let _ = self.commands.send(Command::Volume(target, volume));
    }

    /// Mutes or unmutes `target`.
    pub fn set_muted(&self, target: Target, muted: bool) {
        let _ = self.commands.send(Command::Mute(target, Some(muted)));
    }

    /// Flips `target`'s mute.
    pub fn toggle_mute(&self, target: Target) {
        let _ = self.commands.send(Command::Mute(target, None));
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
