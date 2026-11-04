//! Panel documents: `.aegle` files whose root is a `Panel`, opened on every
//! output (or the one they name), filled with shell states, and reloaded in
//! place when any of their files is saved.
//!
//! Imports resolve against the importing file; `use "palettes.aegle"` gets
//! the embedded prelude unless such a file exists. Relative image paths
//! resolve against the first document's directory, and fonts in any asset
//! root's `fonts/` are registered at start. Images, icons and shaders show
//! their files again when those change. A script beside a document
//! (`bar.luau` for `bar.aegle`) runs while it is open.

pub(crate) mod actions;
pub mod elements;
mod follow;
pub mod image;
pub(crate) mod link;
mod lock;
mod pixels;
mod popup;
mod shader;
pub mod states;
mod system;

use aegle_loader::{Elements, Program, View, markup::Type};
use aegle_ui::{Node, Result, Theme, Ui};
use palettes_assets::{Roots, Watcher};
use palettes_shell::{OutputInfo, Shell, Surface};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

struct Open {
    output: OutputInfo,
    surface: Surface,
    view: Rc<RefCell<View>>,
}

struct Document {
    path: PathBuf,
    /// The script running beside it.
    script: Option<PathBuf>,
    program: Program,
    /// Entry states, which scripts share.
    states: Vec<(String, Type)>,
    /// Where its `Panel` asked to go, as of the last build.
    placement: elements::Placement,
    /// Every file's text, to tell a real change from a duplicate event.
    sources: String,
    files: Vec<PathBuf>,
    panels: Vec<Open>,
}

struct Runner {
    shell: Shell,
    elements: Elements,
    roots: Roots,
    documents: Vec<Document>,
}

thread_local! {
    static RUNNER: RefCell<Option<Runner>> = const { RefCell::new(None) };
    /// Outside the runner, since views loading images while the runner
    /// builds them watch their files too.
    static WATCHER: RefCell<Option<Watcher>> = const { RefCell::new(None) };
}

/// Reports changes to `path` from now on, if hot reload runs.
pub(crate) fn watch(path: &Path) {
    WATCHER.with_borrow(|watcher| {
        if let Some(watcher) = watcher
            && let Err(e) = watcher.watch(path)
        {
            eprintln!("palettes: not watching {}: {e}", path.display());
        }
    });
}

/// The asset roots of the first document.
pub(crate) fn roots() -> Roots {
    RUNNER
        .with_borrow(|r| r.as_ref().map(|r| r.roots.clone()))
        .unwrap_or_else(|| Roots::new(None))
}

struct Loaded {
    program: Program,
    sources: String,
    files: Vec<PathBuf>,
    states: Vec<(String, Type)>,
}

/// Reads `path` and its imports, serving the prelude for `palettes.aegle`,
/// and lets its host calls reach script functions.
fn load(path: &Path, elements: &Elements, check: bool) -> Result<Loaded> {
    let entry = path.to_str().ok_or("document path is not UTF-8")?;
    let (mut files, mut sources) = (Vec::new(), String::new());
    let mut read = |name: &str| {
        let file = Path::new(name);
        match std::fs::read_to_string(file) {
            Ok(text) => {
                files.push(file.to_owned());
                sources.push_str(&text);
                Ok(text)
            }
            Err(_) if file.file_name().is_some_and(|n| n == "palettes.aegle") => {
                Ok(states::PRELUDE.into())
            }
            Err(e) => Err(format!("{name}: {e}")),
        }
    };
    let (checked, _) = aegle_loader::markup::compile(entry, &elements.specs(), &mut read)?;
    link::learn(&checked.records);
    actions::scripted(&checked.host_calls, check);
    let states = checked.templates[0]
        .states
        .iter()
        .map(|(name, ty, _)| (name.clone(), ty.clone()))
        .collect();
    Ok(Loaded {
        program: Program::from_checked(checked, elements),
        sources,
        files,
        states,
    })
}

/// Where the document's panel goes, from a throwaway build of it.
fn probe(shell: &Shell, program: &Program) -> Result<elements::Placement> {
    let ui = Ui::with_fonts(shell.fonts(), Theme::default())?;
    let view = program.build(&ui.root())?;
    let placement = elements::placement(view.root().node()).unwrap_or_default();
    ui.close();
    Ok(placement)
}

fn open_panel(
    shell: &Shell,
    program: &Program,
    states: &[(String, Type)],
    output: &OutputInfo,
    placement: &elements::Placement,
) -> Result<Open> {
    let surface = match placement.edge {
        elements::Edge::Lock => lock::surface(shell, output)?,
        _ => shell.panel(placement.options().on(output.id))?,
    };
    // The `Panel` fills the surface and paints itself; around its corners
    // the desktop shows.
    surface.set_background(aegle_ui::Color::TRANSPARENT);
    surface.set_padding(0.0);
    let view = match program.build(&surface) {
        Ok(view) => Rc::new(RefCell::new(view)),
        Err(e) => {
            surface.close()?;
            return Err(e);
        }
    };
    let open = Open {
        output: output.clone(),
        surface,
        view,
    };
    attach(shell, &open, states)?;
    if placement.edge == elements::Edge::Lock {
        lock::focus_field(&open.surface)?;
    }
    Ok(open)
}

/// Ties a freshly built view to its surface, the shell's states and scripts.
fn attach(shell: &Shell, open: &Open, entry: &[(String, Type)]) -> Result<()> {
    let root: Node = open.view.borrow().root().node().clone();
    elements::attach(&root, &open.surface);
    states::bind(shell, &open.view, &root, &open.output.name)?;
    link::link(&open.view, entry);
    Ok(())
}

impl Document {
    fn is_popup(&self) -> bool {
        self.placement.edge == elements::Edge::Popup
    }

    /// Opens panels on outputs that lack one and closes those of departed
    /// outputs. A popup opens only when asked to.
    fn sync(&mut self, shell: &Shell) -> Result<()> {
        let outputs = shell.outputs();
        self.panels.retain(|open| {
            let keep = open.surface.is_open() && outputs.iter().any(|o| o.id == open.output.id);
            if !keep && open.surface.is_open() {
                let _ = open.surface.close();
            }
            keep
        });
        let placement = probe(shell, &self.program)?;
        self.placement = placement.clone();
        if self.is_popup() {
            return Ok(());
        }
        for output in outputs {
            let wanted = placement.output.is_empty() || placement.output == output.name;
            if !wanted || self.panels.iter().any(|p| p.output.id == output.id) {
                continue;
            }
            let open = open_panel(shell, &self.program, &self.states, &output, &placement)?;
            self.panels.push(open);
        }
        Ok(())
    }

    /// Rebuilds every panel from the files on disk, keeping states and
    /// surfaces; an error keeps what is shown.
    fn reload(&mut self, shell: &Shell, elements: &Elements, force: bool) -> Result<()> {
        let loaded = load(&self.path, elements, false)?;
        if loaded.sources == self.sources && !force {
            return Ok(());
        }
        let placement = probe(shell, &loaded.program)?;
        let popup = placement.edge == elements::Edge::Popup;
        for open in &self.panels {
            open.view.borrow_mut().reload(&loaded.program)?;
            attach(shell, open, &loaded.states)?;
            if popup {
                popup::fit(open, &placement)?;
            } else if placement.edge != elements::Edge::Lock {
                open.surface
                    .set_placement(placement.options().on(open.output.id))?;
            }
        }
        // A changed `output` closes and opens panels.
        self.panels.retain(|open| {
            let keep = placement.output.is_empty() || placement.output == open.output.name;
            if !keep {
                let _ = open.surface.close();
            }
            keep
        });
        (self.program, self.sources, self.files, self.states) =
            (loaded.program, loaded.sources, loaded.files, loaded.states);
        self.sync(shell)
    }

    /// Closes its surfaces and stops its script; a lock document unlocks.
    fn close(self, shell: &Shell) {
        if let Some(script) = &self.script {
            crate::script::unload(script);
        }
        if self.placement.edge == elements::Edge::Lock
            && let Err(e) = shell.unlock()
        {
            eprintln!("palettes: unlocking: {e}");
        }
        for open in self.panels {
            if open.surface.is_open() {
                let _ = open.surface.close();
            }
        }
    }
}

fn with_runner<T>(f: impl FnOnce(&mut Runner) -> Result<T>) -> Result<T> {
    // The runner leaves the slot while it works: panels it builds may run
    // code that looks the runner up again.
    let mut runner = RUNNER
        .with_borrow_mut(Option::take)
        .ok_or("no document runner on this thread")?;
    let result = f(&mut runner);
    RUNNER.with_borrow_mut(|slot| *slot = Some(runner));
    result
}

fn open_document(runner: &mut Runner, path: &Path) -> Result<()> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // The script runs first: the document may call its functions and show
    // what it sets.
    let script = crate::script::beside(&path);
    if let Some(script) = &script {
        watch(script);
        if let Err(e) = crate::script::load(script) {
            eprintln!("palettes: {e}");
        }
    }
    let opened = load(&path, &runner.elements, false).and_then(|loaded| {
        loaded.files.iter().for_each(|file| watch(file));
        let mut document = Document {
            path,
            script: script.clone(),
            program: loaded.program,
            states: loaded.states,
            sources: loaded.sources,
            files: loaded.files,
            placement: elements::Placement::default(),
            panels: Vec::new(),
        };
        let opened = probe(&runner.shell, &document.program).and_then(|placement| {
            document.placement = placement;
            if document.is_popup() {
                popup::open(runner, &mut document)
            } else {
                document.sync(&runner.shell)
            }
        });
        match opened {
            Ok(()) => Ok(document),
            Err(e) => {
                document.close(&runner.shell);
                Err(e)
            }
        }
    });
    match opened {
        Ok(document) => {
            runner.documents.push(document);
            Ok(())
        }
        Err(e) => {
            if let Some(script) = &script {
                crate::script::unload(script);
            }
            Err(e)
        }
    }
}

fn changed(path: &Path) -> Result<()> {
    image::changed(path);
    shader::changed(path);
    crate::script::file_changed(path);
    let script = crate::script::is_script(path);
    if script && let Err(e) = crate::script::load(path) {
        eprintln!("palettes: {e}");
    }
    with_runner(|runner| {
        let Runner {
            shell,
            elements,
            documents,
            ..
        } = runner;
        for document in documents
            .iter_mut()
            .filter(|d| d.files.iter().any(|f| f == path) || d.script.as_deref() == Some(path))
        {
            // A script may now define the functions the document calls.
            if let Err(e) = document.reload(shell, elements, script) {
                eprintln!("palettes: keeping {}: {e}", document.path.display());
            }
        }
        // Imports may have changed.
        documents
            .iter()
            .flat_map(|d| &d.files)
            .for_each(|file| watch(file));
        Ok(())
    })
}

/// Parses and checks a document and its imports without opening it.
pub fn check(path: &Path) -> Result<()> {
    actions::declare();
    image::start(None)?;
    let script = crate::script::beside(path);
    if let Some(script) = &script {
        crate::script::check(script)?;
    }
    let loaded = load(path, &elements::elements(), script.is_some())?;
    // Host calls are only checked when a view is built.
    let fonts = Rc::new(RefCell::new(aegle_ui::TextSystem::new()));
    let ui = Ui::with_fonts(fonts, Theme::default())?;
    loaded.program.build(&ui.root())?;
    ui.close();
    Ok(())
}

/// The document file `name` names, resolved like any asset.
fn document_path(name: &str) -> Result<PathBuf> {
    let path = roots()
        .resolve(name)
        .ok_or_else(|| format!("no document `{name}`"))?;
    Ok(std::fs::canonicalize(&path)?)
}

/// Opens or closes the document `name`, or flips it when `want` is `None`.
fn show(name: &str, want: Option<bool>) -> Result<()> {
    let path = document_path(name)?;
    with_runner(|runner| {
        let index = runner.documents.iter().position(|d| d.path == path);
        match (index, want) {
            (Some(index), None | Some(false)) => {
                runner.documents.remove(index).close(&runner.shell);
                Ok(())
            }
            (None, None | Some(true)) => open_document(runner, &path),
            _ => Ok(()),
        }
    })
}

/// Opens the document at `name` (resolved like any asset), or closes it if open.
pub fn toggle(name: &str) -> Result<()> {
    show(name, None)
}

/// Opens the document at `name` unless it is open.
pub(crate) fn open_named(name: &str) -> Result<()> {
    show(name, Some(true))
}

/// Closes the document at `name` if it is open.
pub(crate) fn close_named(name: &str) -> Result<()> {
    show(name, Some(false))
}

/// Reloads every document from disk.
pub fn reload_all() -> Result<()> {
    with_runner(|runner| {
        let Runner {
            shell,
            elements,
            documents,
            ..
        } = runner;
        for document in documents {
            document.reload(shell, elements, true)?;
        }
        Ok(())
    })
}

/// Runs panel documents: the engine behind the `palettes` command.
///
/// ```no_run
/// palettes::markup::Documents::new()?.open("bar.aegle")?.run()
/// # ; Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Documents {
    shell: Shell,
}

impl Documents {
    /// Connects a shell for documents, with Aegle's and Palettes' elements.
    pub fn new() -> Result<Self> {
        Self::with(Shell::new()?, elements::elements())
    }

    /// Uses `shell` and `elements`, such as Palettes' plus a control
    /// library's: `elements::elements()` extended with `.with::<E>()`.
    pub fn with(shell: Shell, elements: Elements) -> Result<Self> {
        actions::register(&shell);
        image::start(Some(&shell))?;
        crate::script::start(&shell)?;
        crate::ipc::serve(&shell);
        let reloads = shell.channel(|_, path: PathBuf| changed(&path));
        let watcher = Watcher::spawn(move |path| reloads.send(path))
            .inspect_err(|e| eprintln!("palettes: no hot reload: {e}"))
            .ok();
        WATCHER.with_borrow_mut(|slot| *slot = watcher);
        let densest = |outputs: &[OutputInfo]| outputs.iter().map(|o| o.scale).fold(1.0, f32::max);
        elements::rescale(densest(&shell.outputs()));
        let resync = shell.clone();
        shell.on_outputs(move |shell| {
            elements::rescale(densest(&shell.outputs()));
            with_runner(|runner| {
                for document in &mut runner.documents {
                    document.sync(&resync)?;
                }
                Ok(())
            })
        });
        let runner = Runner {
            shell: shell.clone(),
            elements,
            roots: Roots::new(None),
            documents: Vec::new(),
        };
        RUNNER.with_borrow_mut(|slot| *slot = Some(runner));
        Ok(Self { shell })
    }

    /// Opens a panel document. The first one decides the asset roots: its
    /// directory becomes the working directory, and fonts there register.
    pub fn open(self, path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        with_runner(|runner| {
            if runner.documents.is_empty() {
                first_document(runner, &path)?;
            }
            open_document(runner, &path)
        })?;
        Ok(self)
    }

    /// The shell, for Rust panels beside the documents.
    pub fn shell(&self) -> &Shell {
        &self.shell
    }

    /// Runs until `host.quit()` or [`Shell::quit`].
    pub fn run(self) -> Result<()> {
        self.shell.run()
    }
}

fn first_document(runner: &mut Runner, path: &Path) -> Result<()> {
    runner.roots = Roots::new(Some(path));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::env::set_current_dir(dir)?;
    }
    let fonts = runner.shell.fonts();
    for font in runner.roots.fonts() {
        let registered = std::fs::read(&font)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                fonts
                    .borrow_mut()
                    .register_fonts(Arc::new(bytes))
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = registered {
            eprintln!("palettes: font {}: {e}", font.display());
        }
    }
    Ok(())
}
