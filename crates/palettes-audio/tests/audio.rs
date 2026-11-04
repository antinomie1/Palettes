//! Metadata parsing everywhere; the live graph only where PipeWire runs.

use palettes_audio::{Audio, AudioState, Target, default_name};
use std::{sync::mpsc, time::Duration};

#[test]
fn default_metadata_names_a_node() {
    assert_eq!(
        default_name(r#"{"name":"alsa_output.pci-0000"}"#),
        Some("alsa_output.pci-0000")
    );
    assert_eq!(default_name(r#"{ "name" : "x" }"#), Some("x"));
    assert_eq!(default_name("{}"), None);
}

#[test]
fn follows_the_default_source_and_toggles_its_mute() {
    let (send, states) = mpsc::channel::<AudioState>();
    let Ok(audio) = Audio::spawn(move |state| drop(send.send(state.clone()))) else {
        eprintln!("skipped: no PipeWire");
        return;
    };
    let wait = |want: &dyn Fn(&AudioState) -> bool| loop {
        match states.recv_timeout(Duration::from_secs(3)) {
            Ok(state) if want(&state) => return Some(state),
            Ok(_) => {}
            Err(_) => return None,
        }
    };
    let Some(state) = wait(&|s| s.sink.is_some() || s.source.is_some()) else {
        eprintln!("skipped: no default sink or source");
        return;
    };
    if let Some(sink) = &state.sink {
        eprintln!("sink {sink:?}");
        assert!((0.0..=1.5).contains(&sink.volume));
    }
    let Some(state) = state
        .source
        .is_some()
        .then_some(state)
        .or_else(|| wait(&|s| s.source.is_some()))
    else {
        eprintln!("skipped: no default source to toggle");
        return;
    };
    let source = state.source.unwrap();
    assert!((0.0..=1.5).contains(&source.volume));
    audio.toggle_mute(Target::Source);
    assert!(wait(&|s| s.source.as_ref().is_some_and(|e| e.muted != source.muted)).is_some());
    audio.set_muted(Target::Source, source.muted);
    assert!(wait(&|s| s.source.as_ref().is_some_and(|e| e.muted == source.muted)).is_some());
}

#[test]
fn lists_devices_and_measures_the_default_output() {
    let (send, states) = mpsc::channel::<AudioState>();
    let Ok(audio) = Audio::spawn(move |state| drop(send.send(state.clone()))) else {
        eprintln!("skipped: no PipeWire");
        return;
    };
    let mut latest = None;
    while let Ok(state) = states.recv_timeout(Duration::from_secs(2)) {
        let done = state.sink.is_some();
        latest = Some(state);
        if done {
            break;
        }
    }
    let Some(state) = latest.filter(|s| s.sink.is_some()) else {
        eprintln!("skipped: no default sink");
        return;
    };
    let sink = state.sink.clone().unwrap();
    assert!(state.sinks.contains(&sink), "the default is listed");
    assert!(state.sinks.iter().all(|e| !e.name.is_empty()));
    // Choosing the default again changes nothing but must be understood.
    audio.set_default(Target::Sink, &sink.name);
    audio.track_peak();
    while let Ok(state) = states.recv_timeout(Duration::from_millis(500)) {
        assert!((0.0..=1.0).contains(&state.peak));
        assert_eq!(state.sink.as_ref().map(|s| &s.name), Some(&sink.name));
    }
}
