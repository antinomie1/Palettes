//! Media players through MPRIS: every `org.mpris.MediaPlayer2.*` name on the
//! session bus, and the current one, the player that last started playing.

use crate::{
    Report, System,
    client::{self, BUS, Bus, Client, Handle, Props, properties_changed},
    pixmap::local_path,
};
use aegle_dbus::{Message, Value};
use std::{
    io,
    time::{Duration, Instant},
};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// A media player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    /// Its bus name without `org.mpris.MediaPlayer2.`, such as `firefox`.
    pub name: String,
    /// Its own name for itself, such as `Mozilla Firefox`.
    pub identity: String,
    /// Its desktop entry id, for an icon.
    pub desktop_entry: String,
    /// `Playing`, `Paused` or `Stopped`.
    pub status: String,
    /// The track's title.
    pub title: String,
    /// Its artists, joined with commas.
    pub artist: String,
    /// Its album.
    pub album: String,
    /// The cover art as a local file, if it is one.
    pub art: String,
    /// The track's length in seconds; 0 when unknown.
    pub length: f64,
    /// Playback position in seconds, as of this report.
    pub position: f64,
    /// Whether it can go to the next track.
    pub can_next: bool,
    /// Whether it can go to the previous track.
    pub can_previous: bool,
    /// Whether it can play or pause now.
    pub can_play: bool,
    /// Whether it can seek.
    pub can_seek: bool,
}

/// Every player and the current one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaState {
    /// Players, by name.
    pub players: Vec<Player>,
    /// The current player's index: the last to start playing, else the
    /// first.
    pub current: Option<usize>,
}

impl MediaState {
    /// The current player.
    pub fn current(&self) -> Option<&Player> {
        self.players.get(self.current?)
    }
}

/// What to ask a player.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// Play or pause.
    PlayPause,
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// Stop.
    Stop,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
    /// Go to a position in seconds.
    Seek(f64),
}

enum Command {
    Control(Option<String>, Control),
    Track,
}

struct Known {
    name: String,
    owner: String,
    root: Props,
    player: Props,
    /// Position in microseconds at `at`.
    position: i64,
    at: Instant,
    /// When it last started playing, to choose the current player.
    started: Option<Instant>,
    /// Its position needs reading again.
    stale: bool,
}

impl Known {
    fn playing(&self) -> bool {
        self.player.str("PlaybackStatus") == "Playing"
    }

    /// The position now, in microseconds.
    fn position(&self) -> i64 {
        match self.playing() {
            true => {
                let rate = self
                    .player
                    .get("Rate")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0);
                let elapsed = self.at.elapsed().as_micros() as f64 * rate;
                self.position + elapsed as i64
            }
            false => self.position,
        }
    }

    fn report(&self) -> Player {
        let p = &self.player;
        let meta = p.get("Metadata");
        let field = |key: &str| meta.and_then(|m| m.get(key));
        let text = |key: &str| field(key).and_then(Value::as_str).unwrap_or("").to_owned();
        let artists = field("xesam:artist").map(Value::items).unwrap_or(&[]);
        let artist = (artists.iter())
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let length = field("mpris:length").and_then(Value::as_f64).unwrap_or(0.0);
        let art = field("mpris:artUrl").and_then(Value::as_str).unwrap_or("");
        Player {
            name: self
                .name
                .strip_prefix(PREFIX)
                .unwrap_or(&self.name)
                .to_owned(),
            identity: self.root.str("Identity").to_owned(),
            desktop_entry: self.root.str("DesktopEntry").to_owned(),
            status: p.str("PlaybackStatus").to_owned(),
            title: text("xesam:title"),
            artist: artist.join(", "),
            album: text("xesam:album"),
            art: local_path(art).unwrap_or_default(),
            length: length / 1e6,
            position: self.position().max(0) as f64 / 1e6,
            can_next: p.bool("CanGoNext"),
            can_previous: p.bool("CanGoPrevious"),
            can_play: p.bool("CanPlay") || p.bool("CanPause"),
            can_seek: p.bool("CanSeek"),
        }
    }
}

struct Service {
    changed: Report<MediaState>,
    players: Vec<Known>,
    /// Report the position every second while the current player plays.
    track: bool,
    tick: Option<Instant>,
    last: Option<MediaState>,
}

impl Service {
    fn add(&mut self, bus: &mut Client, name: &str, owner: Option<String>) {
        let Some(owner) = owner.or_else(|| bus.owner(name)) else {
            return;
        };
        let player = bus.get_all(name, PATH, PLAYER).unwrap_or_default();
        let root = bus.get_all(name, PATH, ROOT).unwrap_or_default();
        self.players.retain(|p| p.name != name);
        let started = (player.str("PlaybackStatus") == "Playing").then(Instant::now);
        self.players.push(Known {
            name: name.to_owned(),
            owner,
            root,
            player,
            position: 0,
            at: Instant::now(),
            started,
            stale: true,
        });
        self.players.sort_by(|a, b| a.name.cmp(&b.name));
    }

    fn current(&self) -> Option<usize> {
        let latest = (self.players.iter().enumerate())
            .filter(|(_, p)| p.playing())
            .max_by_key(|(_, p)| p.started);
        latest
            .or_else(|| (self.players.iter().enumerate()).max_by_key(|(_, p)| p.started))
            .map(|(i, _)| i)
    }

    fn by_owner(&mut self, owner: &str) -> Option<&mut Known> {
        self.players.iter_mut().find(|p| p.owner == owner)
    }
}

impl client::Service for Service {
    type Command = Command;

    fn start(&mut self, bus: &mut Client) -> io::Result<()> {
        bus.add_match(&format!(
            "type='signal',sender='{BUS}',member='NameOwnerChanged',arg0namespace='{ROOT}'"
        ))?;
        bus.add_match(&format!(
            "type='signal',interface='{}',member='PropertiesChanged',path='{PATH}'",
            client::PROPERTIES
        ))?;
        bus.add_match(&format!(
            "type='signal',interface='{PLAYER}',member='Seeked',path='{PATH}'"
        ))?;
        let names = bus.method(BUS, "/org/freedesktop/DBus", BUS, "ListNames", vec![])?;
        let names = names.first().map(Value::items).unwrap_or(&[]);
        for name in names.iter().filter_map(Value::as_str) {
            if name.starts_with(PREFIX) {
                self.add(bus, name, None);
            }
        }
        Ok(())
    }

    fn message(&mut self, bus: &mut Client, message: Message) {
        if message.is_signal(BUS, "NameOwnerChanged") {
            let text = |i: usize| message.body.get(i).and_then(Value::as_str).unwrap_or("");
            let (name, new) = (text(0), text(2));
            if !name.starts_with(PREFIX) {
                return;
            }
            self.players.retain(|p| p.name != name);
            if !new.is_empty() {
                self.add(bus, name, Some(new.to_owned()));
            }
        } else if message.is_signal(PLAYER, "Seeked") {
            let position = message.body.first().and_then(Value::as_i64);
            if let (Some(position), Some(p)) = (position, self.by_owner(&message.sender)) {
                (p.position, p.at) = (position, Instant::now());
            }
        } else if let Some((interface, changed, _)) = properties_changed(&message)
            && let Some(p) = (self.players.iter_mut()).find(|p| p.owner == message.sender)
        {
            if interface == ROOT {
                p.root.merge(changed);
                return;
            }
            let was_playing = p.playing();
            p.position = p.position();
            p.at = Instant::now();
            p.player.merge(changed);
            if p.playing() && !was_playing {
                p.started = Some(Instant::now());
            }
            if changed.get("Metadata").is_some() || changed.get("PlaybackStatus").is_some() {
                p.stale = true;
            }
        }
    }

    fn command(&mut self, bus: &mut Client, command: Command) {
        let (name, control) = match command {
            Command::Track => {
                self.track = true;
                return;
            }
            Command::Control(name, control) => (name, control),
        };
        let index = match name {
            Some(name) => {
                (self.players.iter()).position(|p| p.name.strip_prefix(PREFIX) == Some(&name))
            }
            None => self.current(),
        };
        let Some(p) = index.map(|i| &self.players[i]) else {
            return;
        };
        let (member, body) = match control {
            Control::PlayPause => ("PlayPause", vec![]),
            Control::Play => ("Play", vec![]),
            Control::Pause => ("Pause", vec![]),
            Control::Stop => ("Stop", vec![]),
            Control::Next => ("Next", vec![]),
            Control::Previous => ("Previous", vec![]),
            Control::Seek(seconds) => {
                let meta = p.player.get("Metadata");
                let track = meta
                    .and_then(|m| m.get("mpris:trackid"))
                    .and_then(Value::as_str);
                let position = (seconds.max(0.0) * 1e6) as i64;
                let track = Value::Path(track.unwrap_or("/").to_owned());
                ("SetPosition", vec![track, Value::I64(position)])
            }
        };
        bus.invoke(&p.name, PATH, PLAYER, member, body);
    }

    fn deadline(&self) -> Option<Instant> {
        self.tick
    }

    fn expired(&mut self, _: &mut Client) {
        self.tick = None;
    }

    fn settle(&mut self, bus: &mut Client) {
        for p in self.players.iter_mut().filter(|p| p.stale) {
            p.stale = false;
            let body = vec![Value::str(PLAYER), Value::str("Position")];
            if let Ok(reply) = bus.method(&p.name, PATH, client::PROPERTIES, "Get", body) {
                p.position = reply.first().and_then(Value::as_i64).unwrap_or(0);
                p.at = Instant::now();
            }
        }
        let current = self.current();
        let state = MediaState {
            players: self.players.iter().map(Known::report).collect(),
            current,
        };
        self.tick = None;
        if self.track
            && let Some(p) = current.map(|i| &self.players[i]).filter(|p| p.playing())
        {
            // The next whole second of the track.
            let into = p.position().rem_euclid(1_000_000) as u64;
            self.tick = Some(Instant::now() + Duration::from_micros(1_000_000 - into));
        }
        if self.last.as_ref() != Some(&state) {
            (self.changed)(&state);
            self.last = Some(state);
        }
    }
}

/// The running media service.
pub struct Media(Handle<Command>);

impl Media {
    /// Asks `player` (a [`Player::name`]), or the current player, to act.
    pub fn control(&self, player: Option<&str>, control: Control) {
        self.0
            .send(Command::Control(player.map(str::to_owned), control));
    }

    /// Reports the position every second while the current player plays;
    /// otherwise only when the state changes otherwise.
    pub fn track_position(&self) {
        self.0.send(Command::Track);
    }
}

impl System for Media {
    type State = MediaState;
    const BUS: Bus = Bus::Session;

    fn spawn(bus: &Bus, changed: Report<MediaState>) -> io::Result<Self> {
        let service = Service {
            changed,
            players: Vec::new(),
            track: false,
            tick: None,
            last: None,
        };
        client::spawn("media", bus, service).map(Self)
    }
}
