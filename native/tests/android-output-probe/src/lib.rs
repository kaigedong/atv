//! Test-APK-only output integration probe. No source URL or auth bypass path.
#[cfg(target_os = "android")]
mod probe {
    use atv_core::ts::{Codec, Demux};
    use atv_media::{MediaOutput, OutputError, OutputFactory};
    use jni::{
        JNIEnv,
        objects::{JByteArray, JClass, JObject},
        sys::jstring,
    };
    use std::{
        cell::RefCell,
        panic::{AssertUnwindSafe, catch_unwind},
        time::{Duration, Instant},
    };
    thread_local! { static HELD:RefCell<Option<Box<dyn MediaOutput>>>=RefCell::default(); }

    fn run(
        env: &mut JNIEnv,
        surface: &JObject,
        transport: JByteArray,
    ) -> Result<serde_json::Value, String> {
        let bytes = env
            .convert_byte_array(transport)
            .map_err(|_| "test fixture read failed")?;
        if bytes.len() > 1024 * 1024 {
            return Err("test fixture too large".into());
        }
        let packets = Demux::default()
            .push_fragment(&bytes)
            .map_err(|_| "fixture TS demux failed")?;
        let base = packets
            .iter()
            .map(|p| p.pts_90khz)
            .min()
            .ok_or("no fixture PES")?;
        let mut factory =
            atv_native::output::Window::from_surface(env, surface).map_err(|e| e.to_string())?;
        let mut output = factory.create().map_err(|e| e.to_string())?;
        // Exercise the public output's independent Surface ownership too.
        drop(factory);
        let started = Instant::now();
        for packet in &packets {
            let pts = (packet.pts_90khz as i64 - base as i64) * 1_000_000 / 90_000;
            loop {
                if started.elapsed() > Duration::from_secs(15) {
                    return Err("fixture decoder input stalled".into());
                }
                let result = match packet.codec {
                    Codec::Avc => output.push_video(&packet.data, pts),
                    Codec::Aac => output.push_audio(&packet.data, pts),
                    _ => return Err("unsupported fixture codec".into()),
                };
                match result {
                    Ok(()) => break,
                    Err(OutputError::Backpressure) => {
                        output.drain().map_err(|e| e.to_string())?;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
        loop {
            let stats = output.drain().map_err(|e| e.to_string())?;
            if stats.video_frames >= 110
                && stats.audio_frames >= 198_000
                && stats.position_us >= 4_500_000
            {
                let value = serde_json::json!({"evidence":"test-only-generated-Baseline-AVC-and-AAC",
                    "liveAuthorizationVerifiedByThisProbe":false,"videoFrames":stats.video_frames,
                    "audioFrames":stats.audio_frames,"positionUs":stats.position_us,
                    "width":stats.width,"height":stats.height,"sampleRate":stats.audio_sample_rate,
                    "channels":stats.audio_channels,"nonzeroPcmSamples":stats.audio_nonzero_samples,
                    "audioPeak":stats.audio_peak,"elapsedMs":started.elapsed().as_millis()});
                HELD.with(|held| *held.borrow_mut() = Some(output));
                return Ok(value);
            }
            if started.elapsed() > Duration::from_secs(15) {
                return Err("fixture AV output stalled".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_cn_bobo_atv_nativeplayer_OutputProbe_run(
        mut env: JNIEnv,
        _class: JClass,
        surface: JObject,
        transport: JByteArray,
    ) -> jstring {
        let result = catch_unwind(AssertUnwindSafe(|| run(&mut env, &surface, transport)))
            .unwrap_or_else(|_| Err("fixture probe panic".into()));
        let value = match result {
            Ok(value) => value,
            Err(reason) => serde_json::json!({"failure":reason}),
        };
        env.new_string(value.to_string())
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_cn_bobo_atv_nativeplayer_OutputProbe_close(
        _env: JNIEnv,
        _class: JClass,
    ) {
        HELD.with(|held| held.borrow_mut().take());
    }
}
