//! The default output's level: a passive capture of its monitor, mixed to
//! one channel, whose loudest sample is reported about 25 times a second.

use pipewire::{
    self as pw,
    core::CoreRc,
    properties::properties,
    spa::{self, pod::Pod},
    stream::{StreamListener, StreamRc},
};
use std::time::{Duration, Instant};

/// How often the level is reported at most.
const EVERY: Duration = Duration::from_millis(40);

/// A running capture; dropping it stops it.
pub struct Capture {
    _stream: StreamRc,
    _listener: StreamListener<Option<Instant>>,
}

/// Starts capturing what the default output plays; `level` runs on the
/// PipeWire thread with each new peak, rounded to hundredths.
pub fn capture(core: CoreRc, mut level: impl FnMut(f32) + 'static) -> Result<Capture, pw::Error> {
    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Monitor",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
        // Keeps nothing awake: an idle sink suspends as usual.
        *pw::keys::NODE_PASSIVE => "true",
        *pw::keys::NODE_NAME => "palettes-peak",
    };
    let stream = StreamRc::new(core, "palettes-peak", props)?;
    let mut last = 0.0f32;
    let listener = stream
        .add_local_listener_with_user_data(None::<Instant>)
        .process(move |stream, reported| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else { return };
            let peak = (bytes[..size.min(bytes.len())].as_chunks::<4>().0.iter())
                .map(|b| f32::from_le_bytes(*b).abs())
                .fold(0.0f32, f32::max)
                .min(1.0);
            let peak = (peak * 100.0).round() / 100.0;
            let due = reported.is_none_or(|at| at.elapsed() >= EVERY);
            if peak != last && due {
                last = peak;
                *reported = Some(Instant::now());
                level(peak);
            }
        })
        .register()?;
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_channels(1);
    let format = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    let (cursor, _) = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format),
    )
    .map_err(|_| pw::Error::CreationFailed)?;
    let bytes = cursor.into_inner();
    let mut params = [Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    Ok(Capture {
        _stream: stream,
        _listener: listener,
    })
}
