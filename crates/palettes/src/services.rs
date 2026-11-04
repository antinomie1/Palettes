//! Shell services on the UI thread: clock, desktop (workspaces and windows),
//! audio, the D-Bus system services, icons and applications. Each starts on first use, and every
//! subscription is owned by a control: once that control is gone (its panel
//! closed with its output, say) the subscription ends with it.

use aegle_ui::{Node, Result};
use palettes_assets::{Applications, Icons};
use palettes_audio::{Audio, AudioState, Target};
use palettes_clock::{Period, Ticker};
use palettes_shell::Shell;
use palettes_system::System;
use palettes_workspace::{Compositor, Desktop};
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::SystemTime,
};

/// A follower and the control that owns it.
type Follower<T> = (Node, Box<dyn FnMut(&T)>);

/// Latest value of a service and the controls that follow it.
struct Topic<T> {
    latest: Option<T>,
    followers: Vec<Follower<T>>,
}

impl<T> Default for Topic<T> {
    fn default() -> Self {
        Self {
            latest: None,
            followers: Vec::new(),
        }
    }
}

/// Shared by every subscription; runs followers without holding a borrow, so
/// a follower may subscribe again.
type Shared<T> = Rc<RefCell<Topic<T>>>;

fn publish<T>(topic: &Shared<T>, value: T) {
    let mut followers = {
        let mut topic = topic.borrow_mut();
        topic.latest = Some(value);
        std::mem::take(&mut topic.followers)
    };
    let latest = topic.borrow_mut().latest.take().expect("just set");
    followers.retain_mut(|(owner, follow)| {
        let alive = owner.is_alive();
        if alive {
            follow(&latest);
        }
        alive
    });
    let mut topic = topic.borrow_mut();
    topic.latest = Some(latest);
    followers.append(&mut topic.followers);
    topic.followers = followers;
}

fn follow<T: Clone>(topic: &Shared<T>, owner: Node, mut follower: Box<dyn FnMut(&T)>) {
    let latest = topic.borrow().latest.clone();
    if let Some(latest) = latest {
        follower(&latest);
    }
    topic.borrow_mut().followers.push((owner, follower));
}

/// Follows the clock for `owner`; see [`Services::clock`].
pub(crate) fn follow_clock(
    shell: &Shell,
    owner: &Node,
    format: &str,
    mut f: impl FnMut(&str) + 'static,
) -> Result<()> {
    start_clock(shell, Period::of(format))?;
    let text = palettes_clock::format(format, SystemTime::now());
    f(&text);
    let follower = ClockFollower {
        owner: owner.clone(),
        format: format.to_owned(),
        last: text,
        run: Box::new(f),
    };
    HUB.with_borrow(|hub| hub.clock.borrow_mut().followers.push(follower));
    Ok(())
}

/// Follows the desktop for `owner`; see [`Services::desktop`].
pub(crate) fn follow_desktop(
    shell: &Shell,
    owner: &Node,
    f: impl FnMut(&Desktop) + 'static,
) -> Result<()> {
    let (_, topic) = desktop_topic(shell)?;
    follow(&topic, owner.clone(), Box::new(f));
    Ok(())
}

/// Follows audio for `owner`; see [`Services::audio`].
pub(crate) fn follow_audio(
    shell: &Shell,
    owner: &Node,
    f: impl FnMut(&AudioState) + 'static,
) -> Result<()> {
    let (_, topic) = audio_topic(shell)?;
    follow(&topic, owner.clone(), Box::new(f));
    Ok(())
}

struct ClockFollower {
    owner: Node,
    format: String,
    last: String,
    run: Box<dyn FnMut(&str)>,
}

#[derive(Default)]
struct Clock {
    ticker: Option<(Period, Ticker)>,
    followers: Vec<ClockFollower>,
}

#[derive(Default)]
struct Hub {
    clock: Rc<RefCell<Clock>>,
    desktop: Option<(Rc<Compositor>, Shared<Desktop>)>,
    audio: Option<(Rc<Audio>, Shared<AudioState>)>,
    /// `(Rc<S>, Shared<S::State>)` by the system service type `S`.
    system: HashMap<TypeId, Rc<dyn Any>>,
    icons: Option<Rc<Icons>>,
    applications: Option<Rc<Applications>>,
}

thread_local! {
    static HUB: RefCell<Hub> = RefCell::default();
}

fn tick(clock: &Rc<RefCell<Clock>>, at: SystemTime) {
    let mut followers = std::mem::take(&mut clock.borrow_mut().followers);
    followers.retain_mut(|f| {
        if !f.owner.is_alive() {
            return false;
        }
        let text = palettes_clock::format(&f.format, at);
        if text != f.last {
            (f.run)(&text);
            f.last = text;
        }
        true
    });
    let mut clock = clock.borrow_mut();
    followers.append(&mut clock.followers);
    clock.followers = followers;
}

/// Services on a [`Shell`]. Followers are owned by a control handle `H`
/// (a label, a button, any handle that dereferences to a `Node`), which they
/// receive back, and stop when it is removed.
pub trait Services {
    /// Follows the local time formatted with a `strftime` pattern, such as
    /// `"%H:%M"`. Wakes once a minute, or a second when the pattern shows
    /// seconds, and only calls `f` when the text changes.
    fn clock<H>(&self, owner: &H, format: &str, f: impl FnMut(&H, &str) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static;

    /// Follows workspaces and windows from the compositor's IPC.
    fn desktop<H>(&self, owner: &H, f: impl FnMut(&H, &Desktop) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static;

    /// Follows the default audio sink and source.
    fn audio<H>(&self, owner: &H, f: impl FnMut(&H, &AudioState) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static;

    /// Follows a system service, such as `palettes_system::Battery`:
    /// `shell.follow::<Battery, _>(&label, |label, battery| ...)`.
    fn follow<S: System, H>(&self, owner: &H, f: impl FnMut(&H, &S::State) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static;

    /// A system service, to act on it: `shell.system::<Media>()?`.
    fn system<S: System>(&self) -> Result<Rc<S>>;

    /// The compositor, to switch workspaces and focus or close windows.
    fn compositor(&self) -> Result<Rc<Compositor>>;

    /// The audio mixer, to set volumes and mute.
    fn mixer(&self) -> Result<Rc<Audio>>;
}

fn start_clock(shell: &Shell, period: Period) -> Result<()> {
    let clock = HUB.with_borrow(|hub| hub.clock.clone());
    let current = clock.borrow().ticker.as_ref().map(|(p, _)| *p);
    // A finer ticker serves coarser followers too.
    if current.is_some_and(|p| p <= period) {
        return Ok(());
    }
    let target = clock.clone();
    let sender = shell.channel(move |_, at: SystemTime| tick(&target, at));
    let ticker = Ticker::spawn(period, move |at| sender.send(at))?;
    clock.borrow_mut().ticker = Some((period, ticker));
    Ok(())
}

fn desktop_topic(shell: &Shell) -> Result<(Rc<Compositor>, Shared<Desktop>)> {
    if let Some(found) = HUB.with_borrow(|hub| hub.desktop.clone()) {
        return Ok(found);
    }
    let compositor = Rc::new(Compositor::connect()?);
    let topic: Shared<Desktop> = Rc::default();
    let target = topic.clone();
    let sender = shell.channel(move |_, desktop: Desktop| publish(&target, desktop));
    compositor.subscribe(move |desktop| sender.send(desktop))?;
    let found = (compositor, topic);
    HUB.with_borrow_mut(|hub| hub.desktop = Some(found.clone()));
    Ok(found)
}

fn audio_topic(shell: &Shell) -> Result<(Rc<Audio>, Shared<AudioState>)> {
    if let Some(found) = HUB.with_borrow(|hub| hub.audio.clone()) {
        return Ok(found);
    }
    let topic: Shared<AudioState> = Rc::default();
    let target = topic.clone();
    let sender = shell.channel(move |_, state: AudioState| publish(&target, state));
    let audio = Rc::new(Audio::spawn(move |state| sender.send(state.clone()))?);
    let found = (audio, topic);
    HUB.with_borrow_mut(|hub| hub.audio = Some(found.clone()));
    Ok(found)
}

type SystemTopic<S> = (Rc<S>, Shared<<S as System>::State>);

/// A system service and its topic, started on first use.
fn system_topic<S: System>(shell: &Shell) -> Result<SystemTopic<S>> {
    let id = TypeId::of::<S>();
    let found = HUB.with_borrow(|hub| hub.system.get(&id).cloned());
    if let Some(found) = found.and_then(|f| f.downcast::<SystemTopic<S>>().ok()) {
        return Ok((*found).clone());
    }
    let topic: Shared<S::State> = Rc::default();
    let target = topic.clone();
    let sender = shell.channel(move |_, state: S::State| publish(&target, state));
    let service = Rc::new(S::spawn(
        &S::BUS,
        Box::new(move |s| sender.send(s.clone())),
    )?);
    let found = (service, topic);
    HUB.with_borrow_mut(|hub| hub.system.insert(id, Rc::new(found.clone())));
    Ok(found)
}

/// Follows a system service for `owner`; see [`Services::follow`].
pub(crate) fn follow_system<S: System>(
    shell: &Shell,
    owner: &Node,
    f: impl FnMut(&S::State) + 'static,
) -> Result<()> {
    let (_, topic) = system_topic::<S>(shell)?;
    follow(&topic, owner.clone(), Box::new(f));
    Ok(())
}

impl Services for Shell {
    fn clock<H>(&self, owner: &H, format: &str, mut f: impl FnMut(&H, &str) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static,
    {
        let handle = owner.clone();
        follow_clock(self, owner, format, move |text| f(&handle, text))
    }

    fn desktop<H>(&self, owner: &H, mut f: impl FnMut(&H, &Desktop) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static,
    {
        let handle = owner.clone();
        follow_desktop(self, owner, move |d| f(&handle, d))
    }

    fn audio<H>(&self, owner: &H, mut f: impl FnMut(&H, &AudioState) + 'static) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static,
    {
        let handle = owner.clone();
        follow_audio(self, owner, move |a| f(&handle, a))
    }

    fn follow<S: System, H>(
        &self,
        owner: &H,
        mut f: impl FnMut(&H, &S::State) + 'static,
    ) -> Result<()>
    where
        H: std::ops::Deref<Target = Node> + Clone + 'static,
    {
        let handle = owner.clone();
        follow_system::<S>(self, owner, move |s| f(&handle, s))
    }

    fn system<S: System>(&self) -> Result<Rc<S>> {
        Ok(system_topic::<S>(self)?.0)
    }

    fn compositor(&self) -> Result<Rc<Compositor>> {
        Ok(desktop_topic(self)?.0)
    }

    fn mixer(&self) -> Result<Rc<Audio>> {
        Ok(audio_topic(self)?.0)
    }
}

/// The icon theme of this desktop, SVG included when the `svg` feature is on.
pub fn icons() -> Rc<Icons> {
    HUB.with_borrow_mut(|hub| {
        hub.icons
            .get_or_insert_with(|| Rc::new(Icons::new().with_svg(cfg!(feature = "svg"))))
            .clone()
    })
}

/// Installed applications, read once.
pub fn applications() -> Rc<Applications> {
    HUB.with_borrow_mut(|hub| {
        hub.applications
            .get_or_insert_with(|| Rc::new(Applications::load()))
            .clone()
    })
}

/// Runs a command line through `sh`, detached from the shell: it survives a
/// shell restart and leaves no zombie.
pub fn spawn(command: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    std::process::Command::new("sh")
        .args(["-c", &format!("{command} &")])
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .status()?;
    Ok(())
}

/// Launches the application of desktop entry `id` (or app id) with its `Exec`.
pub fn launch(id: &str) -> Result<()> {
    let apps = applications();
    let entry = apps
        .find(id)
        .ok_or_else(|| format!("no application `{id}`"))?;
    spawn(&entry.exec)
}

pub(crate) fn audio_target(sink: bool) -> Target {
    if sink { Target::Sink } else { Target::Source }
}
