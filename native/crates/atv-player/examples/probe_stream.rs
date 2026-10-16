//! Development-only live protocol/media probe; no video/audio output is simulated.
//! Reports media counts only. This is not an Android playback acceptance test.
use atv_core::{transport::Cancellation, ts::Codec};
use atv_player::{
    identity,
    playback::{self, Event, MediaProcessor},
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let pid = args.next().unwrap_or_else(|| "600001859".into());
    let stream = args.next().unwrap_or_else(|| "2024078201".into());
    let identity_dir = std::env::temp_dir().join("atv-native-stream-probe");
    std::fs::create_dir_all(&identity_dir)?;
    let guid = identity::load_or_create(&identity_dir)?;
    let config = atv_auth::ResolverConfig {
        device_guid: Some(guid),
        ..Default::default()
    };
    let resolver = Arc::new(atv_auth::LiveResolver::new(config)?);
    let cancel = Cancellation::default();
    let stop = cancel.clone();
    let (tx, rx) = mpsc::sync_channel(2);
    let worker =
        std::thread::spawn(move || playback::acquire(&resolver, &pid, &stream, &stop, &tx));
    let mut processor = None;
    let mut fragments = 0;
    let mut video = 0;
    let mut audio = 0;
    let mut bytes = 0;
    let mut preview = 0;
    let mut deadline = None;
    let mut error = None;
    while fragments < 3 {
        let event = match rx.recv_timeout(Duration::from_secs(45)) {
            Ok(event) => event,
            Err(_) => {
                error = Some("media acquisition deadline");
                break;
            }
        };
        match event {
            Event::Resolved {
                initial_url,
                preview_seconds,
                expires_unix_seconds,
            } => {
                preview = preview_seconds;
                deadline = expires_unix_seconds;
                processor = Some(MediaProcessor::new(initial_url));
            }
            Event::Fragment {
                bytes: data,
                discontinuity,
            } => {
                if discontinuity {
                    error = Some("probe encountered a discontinuity; replay required");
                    break;
                }
                let Some(processor) = processor.as_mut() else {
                    error = Some("missing media context");
                    break;
                };
                match processor.fragment(&data) {
                    Ok(packets) => {
                        for packet in packets {
                            match packet.codec {
                                Codec::Avc => video += 1,
                                Codec::Aac => audio += 1,
                                _ => {}
                            }
                            bytes += packet.data.len();
                        }
                        fragments += 1;
                    }
                    Err(_) => {
                        error = Some("native media processing failed");
                        break;
                    }
                }
            }
            Event::Failed(_) => {
                error = Some("native acquisition failed");
                break;
            }
        }
    }
    cancel.cancel();
    drop(rx);
    let _ = worker.join();
    println!(
        "{}",
        serde_json::json!({"evidence":"native-protocol-and-media-only","fragments":fragments,
        "videoAccessUnits":video,"audioPackets":audio,"elementaryBytes":bytes,
        "previewSeconds":preview,"paidDeadlinePresent":deadline.is_some(),"error":error})
    );
    if let Some(error) = error {
        return Err(error.into());
    }
    Ok(())
}
