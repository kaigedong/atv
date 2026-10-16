//! Native desktop host. Same player/protocol/media logic as Android, no webpage.
mod output;
use atv_player::{Player, identity};
use output::{DesktopFactory, Host};
use sdl2::{audio::AudioSpecDesired, event::Event, keyboard::Keycode, video::FullscreenType};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut seconds = None;
    let mut report = None;
    let mut snapshot = None;
    let mut channel_name = "CCTV1".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "--seconds" | "--report" | "--snapshot" | "--channel" => {
                args.next().ok_or("missing option value")?
            }
            "--help" => {
                println!(
                    "atv-desktop [--channel CCTV1] [--seconds 60] [--report result.json] [--snapshot frame.bmp]\nLeft/Right: switch, Enter: fullscreen, Escape: exit fullscreen, P: pause/resume, R: retry, Q: quit"
                );
                return Ok(());
            }
            _ => return Err("unknown option; use --help".into()),
        };
        match arg.as_str() {
            "--seconds" => {
                let value: u64 = value.parse()?;
                if !(5..=3600).contains(&value) {
                    return Err("seconds must be 5..3600".into());
                }
                seconds = Some(value);
            }
            "--report" => report = Some(PathBuf::from(value)),
            "--snapshot" => snapshot = Some(PathBuf::from(value)),
            "--channel" => channel_name = value,
            _ => unreachable!(),
        }
    }
    let channels: Vec<_> = atv_core::directory::bundled()?
        .into_iter()
        .filter(|c| c.public())
        .collect();
    let mut selected = channels
        .iter()
        .position(|c| c.channel_name == channel_name)
        .ok_or("channel not found")?;
    // SDL supplies the per-user platform preference directory (not a temporary GUID).
    let storage = sdl2::filesystem::pref_path("bobo", "atv")?;
    let guid = identity::load_or_create(std::path::Path::new(&storage))?;
    let resolver = Arc::new(atv_auth::LiveResolver::new(atv_auth::ResolverConfig {
        device_guid: Some(guid),
        ..Default::default()
    })?);
    gstreamer::init()?;
    for plugin in [
        "appsrc",
        "appsink",
        "h264parse",
        "aacparse",
        "decodebin",
        "videoconvert",
        "audioconvert",
        "audioresample",
    ] {
        if gstreamer::ElementFactory::find(plugin).is_none() {
            return Err(format!("GStreamer plugin missing: {plugin}; install the base/good/bad/libav runtime plugins").into());
        }
    }
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let audio = sdl.audio()?;
    let window = video
        .window("ATV", 960, 600)
        .position_centered()
        .resizable()
        .build()?;
    let canvas = window.into_canvas().build()?;
    let audio = audio.open_queue::<i16, _>(
        None,
        &AudioSpecDesired {
            freq: Some(48000),
            channels: Some(2),
            samples: Some(1024),
        },
    )?;
    if audio.spec().freq != 48000 || audio.spec().channels != 2 {
        return Err("unsupported audio device format".into());
    }
    let host = Rc::new(RefCell::new(Host {
        canvas,
        audio,
        snapshot,
    }));
    let mut player = Player::new(DesktopFactory(host.clone()), resolver);
    player.select(
        channels[selected].pid.clone(),
        channels[selected].stream_id.clone(),
    )?;
    let mut events = sdl.event_pump()?;
    let started = Instant::now();
    let mut last_log = Instant::now();
    let mut first_playing = None;
    let mut samples = Vec::new();
    let mut fullscreen = false;
    let mut paused = false;
    println!("Left/Right 切台 · Enter 全屏 · Escape 返回 · P 暂停/恢复 · R 重试 · Q 退出");
    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Q),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Left | Keycode::Right),
                    repeat: false,
                    ..
                } => {
                    if matches!(
                        event,
                        Event::KeyDown {
                            keycode: Some(Keycode::Left),
                            ..
                        }
                    ) {
                        selected = (selected + channels.len() - 1) % channels.len();
                    } else {
                        selected = (selected + 1) % channels.len();
                    }
                    player.select(
                        channels[selected].pid.clone(),
                        channels[selected].stream_id.clone(),
                    )?;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    repeat: false,
                    ..
                } => {
                    fullscreen = !fullscreen;
                    host.borrow_mut()
                        .canvas
                        .window_mut()
                        .set_fullscreen(if fullscreen {
                            FullscreenType::Desktop
                        } else {
                            FullscreenType::Off
                        })?;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    fullscreen = false;
                    host.borrow_mut()
                        .canvas
                        .window_mut()
                        .set_fullscreen(FullscreenType::Off)?;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::P),
                    repeat: false,
                    ..
                } => {
                    paused = !paused;
                    if paused {
                        player.pause();
                    } else {
                        player.resume();
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::R),
                    repeat: false,
                    ..
                } => {
                    player.select(
                        channels[selected].pid.clone(),
                        channels[selected].stream_id.clone(),
                    )?;
                }
                _ => {}
            }
        }
        player.tick();
        let status = player.status();
        if status.state == "playing" {
            first_playing.get_or_insert(started.elapsed().as_secs_f64());
        }
        host.borrow_mut().canvas.window_mut().set_title(&format!(
            "ATV · {} · {} {} · ←/→ 切台 · Enter 全屏 · R 重试",
            channels[selected].channel_name, status.state, status.message
        ))?;
        if last_log.elapsed() >= Duration::from_secs(1) {
            let mut sample = serde_json::to_value(status)?;
            sample["observedWallSeconds"] = serde_json::json!(started.elapsed().as_secs_f64());
            sample["audioQueuedBytes"] = serde_json::json!(host.borrow().audio.size());
            println!("{sample}");
            samples.push(sample);
            last_log = Instant::now();
        }
        if seconds.is_some_and(|limit| started.elapsed().as_secs() >= limit) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let final_status = player.status().clone();
    player.stop();
    let result = serde_json::json!({"evidence":"native-desktop-live-output", "platform":std::env::consts::OS,
        "elapsedWallSeconds":started.elapsed().as_secs_f64(),"firstPlayingWallSeconds":first_playing,
        "finalStatus":final_status,"samples":samples,"afterStop":player.status(),
        "audioQueuedBytesAfterStop":host.borrow().audio.size(),
        "externalAudioSinkVerifiedByThisReport":false});
    if let Some(path) = report {
        std::fs::write(path, serde_json::to_string_pretty(&result)?)?;
    }
    if seconds.is_some()
        && (final_status.state != "playing"
            || final_status.video_frames == 0
            || final_status.audio_frames == 0)
    {
        return Err("desktop live output gate failed".into());
    }
    Ok(())
}
