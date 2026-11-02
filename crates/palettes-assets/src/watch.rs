//! Files changing on disk, from an inotify thread.

use rustix::fs::inotify::{self, CreateFlags, WatchFlags};
use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    mem::MaybeUninit,
    os::{fd::OwnedFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Reports changes to watched files. Directories are watched rather than
/// files, so editors that replace a file on save are seen too.
#[derive(Clone)]
pub struct Watcher {
    fd: Arc<OwnedFd>,
    /// Watch descriptor → directory and the file names wanted in it.
    watched: Arc<Mutex<HashMap<i32, (PathBuf, Vec<PathBuf>)>>>,
}

impl Watcher {
    /// Starts the thread; `changed` runs on it with each changed file's path.
    pub fn spawn(mut changed: impl FnMut(PathBuf) + Send + 'static) -> io::Result<Self> {
        let fd = Arc::new(inotify::init(CreateFlags::CLOEXEC)?);
        let watched: Arc<Mutex<HashMap<i32, (PathBuf, Vec<PathBuf>)>>> = Arc::default();
        let (reader, names) = (fd.clone(), watched.clone());
        std::thread::Builder::new()
            .name("palettes-watch".into())
            .spawn(move || {
                let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
                let mut reader_events = inotify::Reader::new(&*reader, &mut buffer);
                loop {
                    let event = match reader_events.next() {
                        Ok(event) => event,
                        Err(rustix::io::Errno::INTR) => continue,
                        Err(_) => return,
                    };
                    let Some(name) = event.file_name() else {
                        continue;
                    };
                    let name = Path::new(OsStr::from_bytes(name.to_bytes()));
                    let hit = {
                        let names = names.lock().unwrap_or_else(|e| e.into_inner());
                        names.get(&event.wd()).and_then(|(dir, files)| {
                            files.iter().any(|f| f == name).then(|| dir.join(name))
                        })
                    };
                    if let Some(path) = hit {
                        changed(path);
                    }
                }
            })?;
        Ok(Self { fd, watched })
    }

    /// Watches `path` from now on. Watching a file twice is harmless.
    pub fn watch(&self, path: &Path) -> io::Result<()> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        let flags = WatchFlags::CLOSE_WRITE | WatchFlags::MOVED_TO | WatchFlags::CREATE;
        let wd = inotify::add_watch(&*self.fd, dir, flags)?;
        let mut watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        let (_, files) = watched
            .entry(wd)
            .or_insert_with(|| (dir.to_owned(), Vec::new()));
        let name = PathBuf::from(name);
        if !files.contains(&name) {
            files.push(name);
        }
        Ok(())
    }
}
