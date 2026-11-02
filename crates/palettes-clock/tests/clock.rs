//! The clock: periods, local formatting and realtime ticks.

use palettes_clock::{LocalTime, Period, Ticker, format};
use std::{
    sync::mpsc,
    time::{Duration, SystemTime},
};

#[test]
fn periods_follow_the_finest_conversion() {
    assert_eq!(Period::of("%H:%M"), Period::Minute);
    assert_eq!(Period::of("%a %d %b"), Period::Minute);
    assert_eq!(Period::of("%H:%M:%S"), Period::Second);
    assert_eq!(Period::of("%T"), Period::Second);
    assert_eq!(Period::of("%-S"), Period::Second);
    assert_eq!(Period::of("100%% %M"), Period::Minute);
}

#[test]
fn formats_in_the_local_zone() {
    // SAFETY: tests in this file never read TZ concurrently with this write.
    unsafe { std::env::set_var("TZ", "UTC") };
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 + 3_723);
    assert_eq!(format("%Y-%m-%d %H:%M:%S", at), "1970-01-02 01:02:03");
    assert_eq!(format("", at), "");
    let local = LocalTime::at(at);
    assert_eq!((local.year, local.month, local.day), (1970, 1, 2));
    assert_eq!(
        (local.hour, local.minute, local.second, local.weekday),
        (1, 2, 3, 5)
    );
}

#[test]
fn ticker_ticks_at_once_and_on_the_second() {
    let (send, receive) = mpsc::channel();
    let ticker = Ticker::spawn(Period::Second, move |at| {
        let _ = send.send(at);
    })
    .unwrap();
    let first = receive.recv_timeout(Duration::from_secs(1)).unwrap();
    let second = receive.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(second > first);
    let into_second = second
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .subsec_millis();
    assert!(into_second < 200, "ticked {into_second} ms into the second");
    drop(ticker);
}
