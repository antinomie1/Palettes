//! [`Shell`]: the handle applications hold, and the event loop.

use crate::{
    PanelOptions, ShellOptions, Surface,
    channel::{Drain, Sender},
    runtime::Runtime,
};
use aegle_platform::{Event, OutputId, OutputInfo, PixelSize, Platform, WakeHandle, WindowOptions};
use aegle_render_vulkan::SharedDevice;
use aegle_ui::{HandlerResult, Result, TextSystem, Ui, UiError};
use palettes_wayland::Wayland;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

type ErrorHandler = Box<dyn FnMut(Box<dyn std::error::Error>) -> Result<()>>;
type Build = Box<dyn FnMut(&Shell, &OutputInfo) -> Result<Surface>>;
type OutputsHandler = Rc<RefCell<dyn FnMut(&Shell) -> Result<()>>>;

/// Surfaces built per output by [`Shell::each_output`].
struct PerOutput {
    build: Rc<RefCell<Build>>,
    built: Vec<(OutputId, Surface)>,
}

pub(crate) struct Inner {
    pub runtime: RefCell<Runtime>,
    dispatching: Cell<bool>,
    quit: Cell<bool>,
    on_error: RefCell<Option<ErrorHandler>>,
    on_outputs: RefCell<Vec<OutputsHandler>>,
    per_output: RefCell<Vec<PerOutput>>,
    drains: RefCell<Vec<Rc<dyn Drain>>>,
    wake: WakeHandle,
}

/// A desktop shell: panels and popups on one Wayland connection, sharing
/// fonts and a Vulkan device. Clones are handles to the same shell; use it on
/// the thread that created it, and reach it from others with [`Self::channel`].
///
/// ```no_run
/// use palettes_shell::{PanelOptions, Shell};
/// use aegle_widgets::Widgets;
///
/// let shell = Shell::new()?;
/// let bar = shell.panel(PanelOptions::top(32))?;
/// bar.text("Hello from Palettes");
/// shell.run()
/// # ; Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct Shell(pub(crate) Rc<Inner>);

struct Guard<'a>(&'a Cell<bool>);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl Shell {
    /// Connects with default options and system fonts.
    pub fn new() -> Result<Self> {
        Self::with_options(ShellOptions::default())
    }

    /// Connects with `options` and system fonts.
    pub fn with_options(options: ShellOptions) -> Result<Self> {
        let mut platform = Wayland::connect()?;
        let wake = platform.wake_handle()?;
        let runtime = Runtime {
            entries: Vec::new(),
            device: None,
            preferences: platform.preferences(),
            platform,
            fonts: Rc::new(RefCell::new(TextSystem::system())),
            options,
            clock: Instant::now(),
        };
        Ok(Self(Rc::new(Inner {
            runtime: RefCell::new(runtime),
            dispatching: Cell::new(false),
            quit: Cell::new(false),
            on_error: RefCell::new(None),
            on_outputs: RefCell::new(Vec::new()),
            per_output: RefCell::new(Vec::new()),
            drains: RefCell::new(Vec::new()),
            wake,
        })))
    }

    /// Opens a panel: a layer surface placed by `options`.
    pub fn panel(&self, options: PanelOptions) -> Result<Surface> {
        let mut runtime = self.0.runtime.borrow_mut();
        let namespace = runtime.options.namespace.clone();
        let size = PixelSize {
            width: options.width,
            height: options.height,
        };
        let (id, ui) = runtime.open(
            WindowOptions {
                title: "",
                app_id: &namespace,
                size,
                layer: Some(options.layer),
                transparent: true,
                ..Default::default()
            },
            size,
        )?;
        Ok(Surface::new(self, id, &ui))
    }

    /// Calls `build` for every connected output, and again for each output
    /// connected later; a surface closes with its output. This is how a bar
    /// appears on every monitor.
    pub fn each_output(
        &self,
        build: impl FnMut(&Shell, &OutputInfo) -> Result<Surface> + 'static,
    ) -> Result<()> {
        self.0.per_output.borrow_mut().push(PerOutput {
            build: Rc::new(RefCell::new(Box::new(build))),
            built: Vec::new(),
        });
        self.sync_outputs()
    }

    /// Connected outputs.
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.0.runtime.borrow().platform.output_list()
    }

    /// The connected output named `name`, such as `DP-1`.
    pub fn output(&self, name: &str) -> Option<OutputInfo> {
        self.outputs().into_iter().find(|o| o.name == name)
    }

    /// Calls `handler` after outputs connect, disconnect or change.
    pub fn on_outputs(&self, mut handler: impl FnMut(&Shell) -> Result<()> + 'static) {
        let handler = Rc::new(RefCell::new(move |shell: &Shell| handler(shell)));
        self.0.on_outputs.borrow_mut().push(handler);
    }

    /// A sender other threads use to hand values to `handler`, which runs on
    /// this thread. Values arrive in order; the loop wakes for them.
    pub fn channel<T: Send + 'static, R: HandlerResult>(
        &self,
        handler: impl FnMut(&Shell, T) -> R + 'static,
    ) -> Sender<T> {
        let (sender, drain) = crate::channel::new(self.0.wake.clone(), handler);
        self.0.drains.borrow_mut().push(drain);
        sender
    }

    /// Receives errors returned by callbacks. Returning `Ok` keeps running;
    /// returning an error ends [`Self::run`] with it. Without a handler errors
    /// are printed and the shell keeps running.
    pub fn on_error(
        &self,
        handler: impl FnMut(Box<dyn std::error::Error>) -> Result<()> + 'static,
    ) {
        *self.0.on_error.borrow_mut() = Some(Box::new(handler));
    }

    /// The shared text system, to register fonts.
    pub fn fonts(&self) -> Rc<RefCell<TextSystem>> {
        self.0.runtime.borrow().fonts.clone()
    }

    /// System appearance preferences.
    pub fn preferences(&self) -> aegle_ui::Preferences {
        self.0.runtime.borrow().preferences
    }

    /// The Vulkan device every surface shares, once a surface exists; for
    /// registering textures.
    pub fn vulkan(&self) -> Option<SharedDevice> {
        self.0.runtime.borrow().device.clone()
    }

    /// Ends [`Self::run`] after the current turn.
    pub fn quit(&self) {
        self.0.quit.set(true);
        self.0.wake.wake();
    }

    /// Runs until [`Self::quit`], or until the last surface closes while no
    /// [`Self::each_output`] waits for outputs.
    pub fn run(&self) -> Result<()> {
        while self.dispatch(None)? {}
        Ok(())
    }

    /// One turn of the loop: applies changes, waits at most `timeout` for
    /// events, handles them and presents. Returns whether the shell goes on.
    pub fn dispatch(&self, timeout: Option<Duration>) -> Result<bool> {
        if self.0.dispatching.replace(true) {
            return Err(UiError::ReentrantAccess.into());
        }
        let _guard = Guard(&self.0.dispatching);
        self.callbacks()?;
        self.0.runtime.borrow_mut().refresh()?;
        if self.finished() {
            return Ok(false);
        }
        let (mut pending, mut wake) = (false, None::<Instant>);
        for ui in self.uis() {
            pending |= ui.has_pending_callbacks();
            if let Some(at) = ui.next_wake() {
                wake = Some(wake.map_or(at, |w| w.min(at)));
            }
        }
        let until = wake.map(|at| at.saturating_duration_since(Instant::now()));
        let wait = if pending {
            Some(Duration::ZERO)
        } else {
            match (timeout, until) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        self.0.runtime.borrow_mut().platform.dispatch(wait)?;
        let (mut outputs, mut stale) = (false, false);
        loop {
            let event = self.0.runtime.borrow_mut().platform.next_event();
            let Some(event) = event else { break };
            let motion = matches!(
                event,
                Event::Pointer {
                    kind: aegle_platform::PointerKind::Move,
                    ..
                }
            );
            match event {
                Event::Wake => self.drain()?,
                Event::Outputs => outputs = true,
                event => {
                    let result = self.event(event);
                    self.report(result)?;
                }
            }
            self.callbacks()?;
            // Queued motion shares one layout; anything else refreshes at once.
            stale = motion;
            if !motion {
                self.0.runtime.borrow_mut().refresh()?;
            }
        }
        if stale {
            self.0.runtime.borrow_mut().refresh()?;
        }
        if outputs {
            self.outputs_changed()?;
            self.callbacks()?;
            self.0.runtime.borrow_mut().refresh()?;
        }
        self.timers()?;
        self.0.runtime.borrow_mut().present()?;
        Ok(!self.finished())
    }

    fn finished(&self) -> bool {
        self.0.quit.get()
            || (self.0.runtime.borrow().entries.is_empty() && self.0.per_output.borrow().is_empty())
    }

    fn uis(&self) -> Vec<Rc<Ui>> {
        self.0
            .runtime
            .borrow()
            .entries
            .iter()
            .map(|e| e.ui.clone())
            .collect()
    }

    fn event(&self, event: Event) -> Result<()> {
        let mut runtime = self.0.runtime.borrow_mut();
        match event {
            Event::Error(error) => Err(error.into()),
            Event::Preferences(preferences) => {
                runtime.set_preferences(preferences);
                Ok(())
            }
            Event::Close { window } => {
                // The compositor closed it, as it does for a popup dismissed by
                // a click elsewhere or a layer on an unplugged output.
                if runtime.entry(window).is_some() {
                    runtime.close(window)?;
                }
                Ok(())
            }
            event => {
                let Some(id) = event.window() else {
                    return Ok(());
                };
                match runtime.entries.iter_mut().find(|e| e.id == id) {
                    Some(entry) => entry.event(event),
                    None => Ok(()),
                }
            }
        }
    }

    /// Runs control callbacks outside every borrow.
    fn callbacks(&self) -> Result<()> {
        for ui in self.uis() {
            self.report(ui.dispatch_callbacks())?;
        }
        Ok(())
    }

    /// Delayed control work (tooltips) and per-frame callbacks.
    fn timers(&self) -> Result<()> {
        let now = Instant::now();
        let ready: Vec<_> = {
            let runtime = self.0.runtime.borrow();
            runtime
                .entries
                .iter()
                .map(|e| (e.ui.clone(), e.ready))
                .collect()
        };
        let mut changed = false;
        for (ui, ready) in ready {
            if ui.next_wake().is_some_and(|at| at <= now) {
                ui.wake(now)?;
                changed = true;
            }
            if ready && ui.wants_frames() {
                ui.run_frame(now);
                changed = true;
            }
        }
        if changed {
            self.callbacks()?;
            self.0.runtime.borrow_mut().refresh()?;
        }
        Ok(())
    }

    fn drain(&self) -> Result<()> {
        let drains = self.0.drains.borrow().clone();
        for drain in drains {
            let result = drain.drain(self);
            self.report(result)?;
        }
        Ok(())
    }

    pub(crate) fn report(&self, result: Result<()>) -> Result<()> {
        let Err(error) = result else { return Ok(()) };
        match self.0.on_error.borrow_mut().as_mut() {
            Some(handler) => handler(error),
            None => {
                eprintln!("palettes: {error}");
                Ok(())
            }
        }
    }

    /// Builds surfaces for new outputs and closes those of departed ones.
    fn sync_outputs(&self) -> Result<()> {
        let outputs = self.outputs();
        let count = self.0.per_output.borrow().len();
        for index in 0..count {
            let (build, missing) = {
                let mut per_output = self.0.per_output.borrow_mut();
                let entry = &mut per_output[index];
                let mut gone = Vec::new();
                entry.built.retain(|(id, surface)| {
                    let keep = surface.is_open() && outputs.iter().any(|o| o.id == *id);
                    if !keep {
                        gone.push(surface.clone());
                    }
                    keep
                });
                for surface in gone {
                    if surface.is_open() {
                        surface.close()?;
                    }
                }
                let missing: Vec<OutputInfo> = outputs
                    .iter()
                    .filter(|o| !entry.built.iter().any(|(id, _)| *id == o.id))
                    .cloned()
                    .collect();
                (entry.build.clone(), missing)
            };
            for output in missing {
                let built = (build.borrow_mut())(self, &output);
                match built {
                    Ok(surface) => self.0.per_output.borrow_mut()[index]
                        .built
                        .push((output.id, surface)),
                    Err(error) => self.report(Err(error))?,
                }
            }
        }
        Ok(())
    }

    fn outputs_changed(&self) -> Result<()> {
        self.sync_outputs()?;
        let handlers = self.0.on_outputs.borrow().clone();
        for handler in handlers {
            let result = (handler.borrow_mut())(self);
            self.report(result)?;
        }
        Ok(())
    }
}
