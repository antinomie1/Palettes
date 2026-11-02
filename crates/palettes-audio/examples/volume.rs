//! Prints the default sink and source; with an argument, sets the sink volume.
//! `cargo run -p palettes-audio --example volume -- 0.5`
use palettes_audio::{Audio, Target};
use std::{sync::mpsc, time::Duration};

fn main() -> std::io::Result<()> {
    let (send, states) = mpsc::channel();
    let audio = Audio::spawn(move |state| drop(send.send(state.clone())))?;
    if let Some(volume) = std::env::args().nth(1).and_then(|v| v.parse().ok()) {
        std::thread::sleep(Duration::from_millis(200));
        audio.set_volume(Target::Sink, volume);
    }
    while let Ok(state) = states.recv_timeout(Duration::from_millis(500)) {
        println!("{state:?}");
    }
    Ok(())
}
