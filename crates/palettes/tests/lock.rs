//! A lock document locks the session on every output, and closing it
//! unlocks. It locks the session, so it is ignored by default; run it
//! against a headless Pastel: `cargo test -p palettes --test lock -- --ignored`.

use palettes::{Shell, ipc, markup};
use std::time::{Duration, Instant};

#[test]
#[ignore = "locks the session; run against a headless Pastel"]
fn a_lock_document_locks_until_it_closes() {
    let dir = std::env::temp_dir().join(format!("palettes-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("PALETTES_SOCKET", dir.join("ipc.sock")) };
    let lock = dir.join("lock.aegle");
    std::fs::write(
        &lock,
        "use \"palettes.aegle\"\nPanel {\n    edge: lock\n    state user: string = \"\"\n    state lock_error: string = \"\"\n    Text { text: user + lock_error }\n}\n",
    )
    .unwrap();
    markup::check(&lock).unwrap();
    let shell = Shell::new().unwrap();
    let documents = markup::Documents::with(shell.clone(), markup::elements::elements())
        .unwrap()
        .open(&lock)
        .unwrap();
    let until = |what: &str, done: &dyn Fn() -> bool| {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "timed out: {what}");
            shell.dispatch(Some(Duration::from_millis(10))).unwrap();
        }
    };
    until("locked", &|| shell.is_locked());
    let request = vec!["close".to_owned(), lock.to_string_lossy().into_owned()];
    let asking = std::thread::spawn(move || ipc::send(&request).map_err(|e| e.to_string()));
    until("answered", &|| asking.is_finished());
    assert_eq!(asking.join().unwrap(), Ok(String::new()));
    until("unlocked", &|| !shell.is_locked());
    drop(documents);
    let _ = std::fs::remove_dir_all(dir);
}
