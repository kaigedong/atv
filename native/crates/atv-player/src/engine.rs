//! Portable live playback policy; platform outputs are created and owned on one thread.
use crate::Quality;
use crate::playback::{
    self, AccessUnit, BufferStats, Event, MediaFragment, MediaProcessor, PlaybackError, Status,
};
use atv_auth::LiveResolver;
use atv_core::{transport::Cancellation, ts::Codec};
use atv_media::{MediaOutput, OutputError as CodecError, OutputFactory, OutputStats};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Authorization/download provider. Implementations must honor cancellation and
/// bounded sends, and must never publish a stream after server permission denial.
pub trait LiveSource: Send + Sync + 'static {
    fn acquire_quality(
        &self,
        pid: &str,
        stream_id: &str,
        _quality: Quality,
        cancel: &Cancellation,
        events: &mpsc::SyncSender<Event>,
    ) {
        self.acquire(pid, stream_id, cancel, events);
    }
    fn acquire(
        &self,
        pid: &str,
        stream_id: &str,
        cancel: &Cancellation,
        events: &mpsc::SyncSender<Event>,
    );
}
impl LiveSource for LiveResolver {
    fn acquire_quality(
        &self,
        pid: &str,
        stream_id: &str,
        quality: Quality,
        cancel: &Cancellation,
        events: &mpsc::SyncSender<Event>,
    ) {
        playback::acquire_quality(self, pid, stream_id, quality, cancel, events);
    }
    fn acquire(
        &self,
        pid: &str,
        stream_id: &str,
        cancel: &Cancellation,
        events: &mpsc::SyncSender<Event>,
    ) {
        playback::acquire(self, pid, stream_id, cancel, events);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("invalid channel identifier")]
    InvalidChannel,
    #[error("player worker stopped")]
    Stopped,
}
fn validate_channel(pid: &str, stream_id: &str) -> Result<(), ControlError> {
    let valid = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_digit());
    if valid(pid) && valid(stream_id) {
        Ok(())
    } else {
        Err(ControlError::InvalidChannel)
    }
}

/// Main-thread/host-driven player. Call tick frequently without blocking the UI.
/// The factory, outputs, rendering and teardown all stay on this owning thread;
/// only bounded authorization/download work is moved to background threads.
pub struct Player<F: OutputFactory> {
    factory: F,
    source: Arc<dyn LiveSource>,
    selected: Option<(String, String)>,
    paused: bool,
    run: Option<Run>,
    pending_start: bool,
    count: Arc<AtomicUsize>,
    status: Status,
    quality: Quality,
}
impl<F: OutputFactory> Player<F> {
    pub fn new(factory: F, source: Arc<dyn LiveSource>) -> Self {
        Self {
            factory,
            source,
            selected: None,
            paused: false,
            run: None,
            pending_start: false,
            count: NETWORK_COUNT
                .get_or_init(|| Arc::new(AtomicUsize::new(0)))
                .clone(),
            status: Status::default(),
            quality: Quality::Auto,
        }
    }
    pub fn select(&mut self, pid: String, stream_id: String) -> Result<(), ControlError> {
        validate_channel(&pid, &stream_id)?;
        self.run.take();
        self.status = Status::loading(pid.clone());
        self.status.requested_quality = self.quality.id();
        if self.paused {
            self.status.state = "paused";
        }
        self.selected = Some((pid, stream_id));
        self.pending_start = !self.paused;
        Ok(())
    }
    pub fn set_quality(&mut self, quality: Quality) {
        if self.quality != quality {
            self.quality = quality;
            if let Some((pid, stream)) = self.selected.clone() {
                let _ = self.select(pid, stream);
            }
            self.status.requested_quality = quality.id();
        }
    }
    pub fn pause(&mut self) {
        self.paused = true;
        self.pending_start = false;
        self.run.take();
        self.status.state = "paused";
        self.status.buffers = BufferStats::default();
    }
    pub fn resume(&mut self) {
        if self.paused {
            self.paused = false;
            self.pending_start = self.selected.is_some();
            if let Some((pid, _)) = &self.selected {
                self.status = Status::loading(pid.clone());
                self.status.requested_quality = self.quality.id();
            }
        }
    }
    pub fn stop(&mut self) {
        self.run.take();
        self.selected = None;
        self.pending_start = false;
        self.paused = false;
        self.status = Status::default();
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn tick(&mut self) {
        if self.pending_start
            && let Some(guard) = WorkerCount::try_acquire(&self.count)
        {
            self.pending_start = false;
            if let Some((pid, stream_id)) = &self.selected {
                match Run::new(
                    self.source.clone(),
                    pid.clone(),
                    stream_id.clone(),
                    self.quality,
                    guard,
                ) {
                    Ok(run) => self.run = Some(run),
                    Err(error) => self.status.fail(&error),
                }
            }
        }
        if let Some(active) = &mut self.run {
            match active.advance(&mut self.factory) {
                Ok(()) => active.publish(&mut self.status),
                Err(error) => {
                    active.publish(&mut self.status);
                    self.status.fail(&error);
                    self.run.take();
                }
            }
        }
    }
}
impl<F: OutputFactory> Drop for Player<F> {
    fn drop(&mut self) {
        // Outputs must be destroyed before factory-owned windows/devices.
        self.run.take();
    }
}

enum Command {
    Quality(Quality),
    Select { pid: String, stream_id: String },
    Pause,
    Resume,
    Stop,
}
/// Worker-driven wrapper for platforms whose output may be created off the UI
/// thread. Outputs themselves are never sent between threads.
pub struct Session {
    commands: Sender<Command>,
    status: Arc<Mutex<Status>>,
}
impl Session {
    pub fn new<F: OutputFactory + Send + 'static>(
        factory: F,
        source: Arc<dyn LiveSource>,
    ) -> Result<Self, std::io::Error> {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let visible = status.clone();
        thread::Builder::new()
            .name("atv-render".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut player = Player::new(factory, source);
                    loop {
                        loop {
                            match rx.try_recv() {
                                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return,
                                Ok(Command::Select { pid, stream_id }) => {
                                    let _ = player.select(pid, stream_id);
                                }
                                Ok(Command::Pause) => player.pause(),
                                Ok(Command::Quality(quality)) => player.set_quality(quality),
                                Ok(Command::Resume) => player.resume(),
                                Err(TryRecvError::Empty) => break,
                            }
                        }
                        player.tick();
                        if let Ok(mut state) = visible.lock() {
                            *state = player.status().clone();
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }));
                if result.is_err()
                    && let Ok(mut state) = visible.lock()
                {
                    state.fail(&PlaybackError::Codec);
                }
            })?;
        Ok(Self {
            commands: tx,
            status,
        })
    }
    pub fn select(&self, pid: String, stream_id: String) -> Result<(), ControlError> {
        validate_channel(&pid, &stream_id)?;
        self.commands
            .send(Command::Select { pid, stream_id })
            .map_err(|_| ControlError::Stopped)
    }
    pub fn pause(&self) -> Result<(), ControlError> {
        self.commands
            .send(Command::Pause)
            .map_err(|_| ControlError::Stopped)
    }
    pub fn set_quality(&self, quality: Quality) -> Result<(), ControlError> {
        self.commands
            .send(Command::Quality(quality))
            .map_err(|_| ControlError::Stopped)
    }
    pub fn resume(&self) -> Result<(), ControlError> {
        self.commands
            .send(Command::Resume)
            .map_err(|_| ControlError::Stopped)
    }
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

struct WorkerCount(Arc<AtomicUsize>);
static NETWORK_COUNT: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
impl WorkerCount {
    fn try_acquire(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .ok()?;
        Some(Self(count.clone()))
    }
}
impl Drop for WorkerCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Run {
    effective_quality: &'static str,
    available_qualities: Vec<&'static str>,
    cancel: Cancellation,
    events: Receiver<Event>,
    initial_url: String,
    processor: Option<MediaProcessor>,
    output: Option<Box<dyn MediaOutput>>,
    packets: VecDeque<AccessUnit>,
    fragment: Option<MediaFragment>,
    buffers: BufferStats,
    preview_seconds: u64,
    expires_unix_seconds: Option<u64>,
    first_output: Option<Instant>,
    output_started: Instant,
    last_video: Option<Instant>,
    last_audio: Option<Instant>,
    last_progress: Instant,
    stats: OutputStats,
    video_base: u64,
    audio_base: u64,
    position_base: i64,
}
impl Run {
    fn new(
        source: Arc<dyn LiveSource>,
        pid: String,
        stream_id: String,
        quality: Quality,
        guard: WorkerCount,
    ) -> Result<Self, PlaybackError> {
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let (tx, events) = mpsc::sync_channel(2);
        thread::Builder::new()
            .name("atv-network".into())
            .spawn(move || {
                let _guard = guard;
                source.acquire_quality(&pid, &stream_id, quality, &worker_cancel, &tx);
            })
            .map_err(|_| PlaybackError::Network)?;
        Ok(Self {
            effective_quality: "",
            available_qualities: Vec::new(),
            cancel,
            events,
            initial_url: String::new(),
            processor: None,
            output: None,
            packets: VecDeque::new(),
            fragment: None,
            buffers: BufferStats::default(),
            preview_seconds: 0,
            expires_unix_seconds: None,
            first_output: None,
            output_started: Instant::now(),
            last_video: None,
            last_audio: None,
            last_progress: Instant::now(),
            stats: OutputStats::default(),
            video_base: 0,
            audio_base: 0,
            position_base: 0,
        })
    }
    fn reset_output(&mut self, factory: &mut impl OutputFactory) -> Result<(), PlaybackError> {
        self.video_base = self.stats.video_frames;
        self.audio_base = self.stats.audio_frames;
        self.position_base = self.stats.position_us;
        self.output.take();
        self.last_video = None;
        self.last_audio = None;
        self.output_started = Instant::now();
        self.processor = Some(MediaProcessor::new(self.initial_url.clone()));
        self.output = Some(factory.create().map_err(|_| PlaybackError::Codec)?);
        Ok(())
    }
    fn drain(&mut self) -> Result<(), PlaybackError> {
        let Some(output) = &mut self.output else {
            return Ok(());
        };
        let mut stats = output.drain().map_err(|_| PlaybackError::Codec)?;
        stats.video_frames += self.video_base;
        stats.audio_frames += self.audio_base;
        stats.position_us += self.position_base;
        let now = Instant::now();
        if stats.video_frames > self.stats.video_frames {
            self.last_video = Some(now);
            self.first_output.get_or_insert(now);
            self.last_progress = now;
        }
        if stats.audio_frames > self.stats.audio_frames {
            self.last_audio = Some(now);
            self.first_output.get_or_insert(now);
            self.last_progress = now;
        }
        self.stats = stats;
        Ok(())
    }
    fn advance(&mut self, factory: &mut impl OutputFactory) -> Result<(), PlaybackError> {
        if self.expires_unix_seconds.is_some_and(|end| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                >= end
        }) {
            return Err(PlaybackError::PreviewEnded);
        }
        self.drain()?;
        // Keep the two-event network jitter buffer, but only transform short
        // batches of the active fragment as the output accepts more input.
        if self.packets.is_empty() && self.fragment.is_none() {
            match self.events.try_recv() {
                Ok(Event::Qualities {
                    effective,
                    available,
                }) => {
                    self.effective_quality = Quality::parse(&effective).map_or("", Quality::id);
                    self.available_qualities = available
                        .iter()
                        .filter_map(|q| Quality::parse(q).map(Quality::id))
                        .collect();
                }
                Ok(Event::Resolved {
                    initial_url,
                    preview_seconds,
                    expires_unix_seconds,
                }) => {
                    self.initial_url = initial_url;
                    self.preview_seconds = preview_seconds;
                    self.expires_unix_seconds = expires_unix_seconds;
                    self.reset_output(factory)?;
                }
                Ok(Event::Fragment {
                    bytes,
                    discontinuity,
                }) => {
                    if discontinuity {
                        self.reset_output(factory)?;
                    }
                    self.fragment = Some(MediaFragment::new(bytes)?);
                }
                Ok(Event::Failed(error)) => return Err(error),
                Err(TryRecvError::Disconnected) => return Err(PlaybackError::Stalled),
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.packets.is_empty()
            && let Some(fragment) = &mut self.fragment
        {
            let processor = self.processor.as_mut().ok_or(PlaybackError::Media)?;
            self.packets = fragment.next(processor)?.into();
            if fragment.finished() {
                self.fragment.take();
            }
        }
        self.measure_buffers();
        for _ in 0..12 {
            let Some(packet) = self.packets.front_mut() else {
                break;
            };
            let output = self.output.as_mut().ok_or(PlaybackError::Codec)?;
            let result = match packet.codec {
                Codec::Avc => output.push_video_owned(&mut packet.data, packet.pts_us),
                Codec::Aac => output.push_audio(&packet.data, packet.pts_us),
                _ => return Err(PlaybackError::Media),
            };
            match result {
                Ok(()) => {
                    self.packets.pop_front();
                }
                Err(CodecError::Backpressure) => break,
                Err(_) => return Err(PlaybackError::Codec),
            }
        }
        self.drain()?;
        self.measure_buffers();
        if self.preview_seconds > 0
            && self
                .first_output
                .is_some_and(|start| start.elapsed().as_secs() >= self.preview_seconds)
        {
            return Err(PlaybackError::PreviewEnded);
        }
        if self.last_progress.elapsed() > Duration::from_secs(45) {
            return Err(PlaybackError::Stalled);
        }
        if self.output.is_some()
            && self.output_started.elapsed() > Duration::from_secs(12)
            && (self
                .last_video
                .is_none_or(|last| last.elapsed() > Duration::from_secs(12))
                || self
                    .last_audio
                    .is_none_or(|last| last.elapsed() > Duration::from_secs(12)))
        {
            return Err(PlaybackError::Stalled);
        }
        Ok(())
    }
    fn measure_buffers(&mut self) {
        let fragment = self.fragment.as_ref().map_or(0, MediaFragment::capacity);
        let demux = self
            .processor
            .as_ref()
            .map_or(0, MediaProcessor::buffered_capacity);
        let packets = self
            .packets
            .iter()
            .map(|packet| packet.data.capacity())
            .sum::<usize>();
        let output = self
            .output
            .as_ref()
            .map_or(0, |output| output.buffered_capacity());
        let total = fragment + demux + packets + output;
        self.buffers = BufferStats {
            fragment_capacity_bytes: fragment,
            demux_capacity_bytes: demux,
            pending_packet_capacity_bytes: packets,
            output_capacity_bytes: output,
            render_capacity_bytes: total,
            render_peak_capacity_bytes: self.buffers.render_peak_capacity_bytes.max(total),
        };
    }
    fn publish(&self, status: &mut Status) {
        status.effective_quality = self.effective_quality;
        status
            .available_qualities
            .clone_from(&self.available_qualities);
        status.buffers = self.buffers.clone();
        status.video_frames = self.stats.video_frames;
        status.audio_frames = self.stats.audio_frames;
        status.audio_sample_rate = self.stats.audio_sample_rate;
        status.audio_channels = self.stats.audio_channels;
        status.audio_nonzero_samples = self.stats.audio_nonzero_samples;
        status.audio_peak = self.stats.audio_peak;
        status.audio_rms = if self.stats.audio_samples > 0 {
            (self.stats.audio_energy / self.stats.audio_samples as f64).sqrt()
        } else {
            0.0
        };
        status.video_width = self.stats.width;
        status.video_height = self.stats.height;
        status.position_ms = (self.stats.position_us.max(0) / 1000) as u64;
        let recent =
            |last: Option<Instant>| last.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        status.state = if recent(self.last_video) && recent(self.last_audio) {
            "playing"
        } else {
            "loading"
        };
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.cancel();
        // Codec/audio teardown always occurs on the renderer; networking can
        // finish its bounded in-flight HTTP request without touching the Surface.
        self.output.take();
    }
}

#[cfg(test)]
mod tests {
    // Scripted providers/outputs test policy and ownership only. These results
    // are never used as evidence of real network/video/audio playback.
    use super::*;
    use atv_media::OutputError;
    use std::{cell::RefCell, rc::Rc};
    static SERIAL: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct Source {
        active: AtomicUsize,
        maximum: AtomicUsize,
        calls: AtomicUsize,
        qualities: Mutex<Vec<Quality>>,
    }
    impl LiveSource for Source {
        fn acquire_quality(
            &self,
            pid: &str,
            stream: &str,
            quality: Quality,
            cancel: &Cancellation,
            events: &mpsc::SyncSender<Event>,
        ) {
            self.qualities.lock().unwrap().push(quality);
            self.acquire(pid, stream, cancel, events);
        }
        fn acquire(
            &self,
            _: &str,
            _: &str,
            cancel: &Cancellation,
            events: &mpsc::SyncSender<Event>,
        ) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            let _ = events.send(Event::Resolved {
                initial_url: "https://fixture.invalid/".into(),
                preview_seconds: 0,
                expires_unix_seconds: None,
            });
            while !cancel.cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Factory {
        // Rc deliberately makes the main-thread factory !Send.
        outputs: Rc<RefCell<usize>>,
        audio: bool,
        fail: bool,
    }
    impl OutputFactory for Factory {
        fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError> {
            *self.outputs.borrow_mut() += 1;
            Ok(Box::new(Output {
                outputs: self.outputs.clone(),
                stats: OutputStats::default(),
                audio: self.audio,
                fail: self.fail,
            }))
        }
    }
    struct Output {
        outputs: Rc<RefCell<usize>>,
        stats: OutputStats,
        audio: bool,
        fail: bool,
    }
    impl MediaOutput for Output {
        fn push_video(&mut self, _: &[u8], _: i64) -> Result<(), OutputError> {
            Ok(())
        }
        fn push_audio(&mut self, _: &[u8], _: i64) -> Result<(), OutputError> {
            Ok(())
        }
        fn drain(&mut self) -> Result<OutputStats, OutputError> {
            if self.fail {
                return Err(OutputError::Unsupported("scripted failure"));
            }
            self.stats.video_frames += 1;
            if self.audio {
                self.stats.audio_frames += 1024;
            }
            self.stats.position_us += 20_000;
            Ok(self.stats)
        }
    }
    impl Drop for Output {
        fn drop(&mut self) {
            *self.outputs.borrow_mut() -= 1;
        }
    }
    fn wait(mut predicate: impl FnMut() -> bool) {
        let start = Instant::now();
        while !predicate() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "test worker did not complete"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture(audio: bool, fail: bool) -> (Player<Factory>, Arc<Source>, Rc<RefCell<usize>>) {
        let source = Arc::new(Source::default());
        let outputs = Rc::new(RefCell::new(0));
        (
            Player::new(
                Factory {
                    outputs: outputs.clone(),
                    audio,
                    fail,
                },
                source.clone(),
            ),
            source,
            outputs,
        )
    }
    #[test]
    fn preference_reaches_source_and_survives_pause_resume() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        player.set_quality(Quality::Shd);
        player.select("1".into(), "2".into()).unwrap();
        wait(|| {
            player.tick();
            player.status().state == "playing"
        });
        assert_eq!(*source.qualities.lock().unwrap(), [Quality::Shd]);
        player.pause();
        player.set_quality(Quality::Fhd);
        wait(|| source.active.load(Ordering::SeqCst) == 0);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        player.resume();
        wait(|| {
            player.tick();
            player.status().state == "playing"
        });
        assert_eq!(
            *source.qualities.lock().unwrap(),
            [Quality::Shd, Quality::Fhd]
        );
        assert_eq!(player.status().requested_quality, "fhd");
        player.stop();
        wait(|| source.active.load(Ordering::SeqCst) == 0);
    }
    #[test]
    fn main_thread_output_lifetime_pause_resume_and_validation() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, outputs) = fixture(true, false);
        assert!(matches!(
            player.select("invalid".into(), "1".into()),
            Err(ControlError::InvalidChannel)
        ));
        player.tick();
        assert_eq!(source.calls.load(Ordering::SeqCst), 0);
        player.select("1".into(), "2".into()).unwrap();
        wait(|| {
            player.tick();
            player.status().state == "playing"
        });
        assert_eq!(*outputs.borrow(), 1);
        player.pause();
        assert_eq!(player.status().state, "paused");
        assert_eq!(*outputs.borrow(), 0);
        wait(|| source.active.load(Ordering::SeqCst) == 0);
        player.resume();
        wait(|| {
            player.tick();
            player.status().state == "playing"
        });
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        player.stop();
        assert_eq!(player.status().state, "idle");
        assert_eq!(*outputs.borrow(), 0);
        wait(|| source.active.load(Ordering::SeqCst) == 0);
    }
    #[test]
    fn one_track_is_not_playing_and_output_errors_release_devices() {
        let _serial = SERIAL.lock().unwrap();
        for (audio, fail) in [(false, false), (true, true)] {
            let (mut player, source, outputs) = fixture(audio, fail);
            player.select("1".into(), "2".into()).unwrap();
            wait(|| {
                player.tick();
                if fail {
                    player.status().state == "error"
                } else {
                    player.status().video_frames > 0
                }
            });
            if fail {
                assert_eq!(player.status().error_code, "codec");
                assert_eq!(*outputs.borrow(), 0);
            } else {
                assert_eq!(player.status().state, "loading");
                assert_eq!(player.status().audio_frames, 0);
            }
            drop(player);
            assert_eq!(*outputs.borrow(), 0);
            wait(|| source.active.load(Ordering::SeqCst) == 0);
        }
    }
    #[test]
    fn rapid_switching_and_multiple_players_share_bounded_downloads() {
        let _serial = SERIAL.lock().unwrap();
        let source = Arc::new(Source::default());
        let mut players: Vec<_> = (0..6)
            .map(|_| {
                Player::new(
                    Factory {
                        outputs: Rc::new(RefCell::new(0)),
                        audio: true,
                        fail: false,
                    },
                    source.clone(),
                )
            })
            .collect();
        for step in 1..40 {
            for player in &mut players {
                player.select(step.to_string(), "2".into()).unwrap();
                player.tick();
            }
            thread::sleep(Duration::from_millis(1));
        }
        drop(players);
        wait(|| {
            source.active.load(Ordering::SeqCst) == 0
                && NETWORK_COUNT.get().unwrap().load(Ordering::SeqCst) == 0
        });
        assert!(source.calls.load(Ordering::SeqCst) > 2);
        assert!(source.maximum.load(Ordering::SeqCst) <= 2);
    }
    struct ThreadFactory(Arc<Mutex<Vec<(&'static str, thread::ThreadId)>>>);
    struct ThreadOutput(Arc<Mutex<Vec<(&'static str, thread::ThreadId)>>>);
    impl OutputFactory for ThreadFactory {
        fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError> {
            self.0
                .lock()
                .unwrap()
                .push(("create", thread::current().id()));
            Ok(Box::new(ThreadOutput(self.0.clone())))
        }
    }
    impl MediaOutput for ThreadOutput {
        fn push_video(&mut self, _: &[u8], _: i64) -> Result<(), OutputError> {
            Ok(())
        }
        fn push_audio(&mut self, _: &[u8], _: i64) -> Result<(), OutputError> {
            Ok(())
        }
        fn drain(&mut self) -> Result<OutputStats, OutputError> {
            Ok(OutputStats {
                video_frames: 1,
                audio_frames: 1024,
                ..Default::default()
            })
        }
    }
    impl Drop for ThreadOutput {
        fn drop(&mut self) {
            self.0
                .lock()
                .unwrap()
                .push(("output-drop", thread::current().id()));
        }
    }
    impl Drop for ThreadFactory {
        fn drop(&mut self) {
            self.0
                .lock()
                .unwrap()
                .push(("factory-drop", thread::current().id()));
        }
    }
    #[test]
    fn session_creates_and_destroys_output_before_factory_on_same_worker() {
        let _serial = SERIAL.lock().unwrap();
        let source = Arc::new(Source::default());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let session = Session::new(ThreadFactory(calls.clone()), source.clone()).unwrap();
        session.select("1".into(), "2".into()).unwrap();
        wait(|| session.status().state == "playing");
        drop(session);
        wait(|| calls.lock().unwrap().len() == 3 && source.active.load(Ordering::SeqCst) == 0);
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls.iter().map(|v| v.0).collect::<Vec<_>>(),
            ["create", "output-drop", "factory-drop"]
        );
        assert!(
            calls
                .iter()
                .all(|v| v.1 == calls[0].1 && v.1 != thread::current().id())
        );
    }
    struct RetryOutput {
        backpressure: bool,
        attempts: Arc<Mutex<Vec<Vec<u8>>>>,
        received: Arc<Mutex<Option<Vec<u8>>>>,
    }
    impl MediaOutput for RetryOutput {
        fn push_video(&mut self, bytes: &[u8], _: i64) -> Result<(), OutputError> {
            self.attempts.lock().unwrap().push(bytes.to_vec());
            if std::mem::take(&mut self.backpressure) {
                Err(OutputError::Backpressure)
            } else {
                Ok(())
            }
        }
        fn push_audio(&mut self, _: &[u8], _: i64) -> Result<(), OutputError> {
            Ok(())
        }
        fn push_video_owned(&mut self, bytes: &mut Vec<u8>, pts: i64) -> Result<(), OutputError> {
            self.push_video(bytes, pts)?;
            *self.received.lock().unwrap() = Some(std::mem::take(bytes));
            Ok(())
        }
        fn drain(&mut self) -> Result<OutputStats, OutputError> {
            Ok(OutputStats::default())
        }
    }
    #[test]
    fn backpressure_retries_exact_packet_without_consuming_it() {
        let _serial = SERIAL.lock().unwrap();
        let source = Arc::new(Source::default());
        let count = Arc::new(AtomicUsize::new(0));
        let guard = WorkerCount::try_acquire(&count).unwrap();
        let mut run =
            Run::new(source.clone(), "1".into(), "2".into(), Quality::Auto, guard).unwrap();
        let mut factory = Factory {
            outputs: Rc::new(RefCell::new(0)),
            audio: true,
            fail: false,
        };
        wait(|| {
            run.advance(&mut factory).unwrap();
            run.output.is_some()
        });
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::new(Mutex::new(None));
        run.output = Some(Box::new(RetryOutput {
            backpressure: true,
            attempts: attempts.clone(),
            received: received.clone(),
        }));
        run.packets.push_back(AccessUnit {
            codec: Codec::Avc,
            pts_us: 0,
            data: vec![1, 2, 3],
        });
        let pointer = run.packets.front().unwrap().data.as_ptr();
        run.advance(&mut factory).unwrap();
        assert_eq!(run.packets.len(), 1);
        assert_eq!(run.packets.front().unwrap().data.as_ptr(), pointer);
        assert!(received.lock().unwrap().is_none());
        run.advance(&mut factory).unwrap();
        assert!(run.packets.is_empty());
        assert_eq!(received.lock().unwrap().as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(*attempts.lock().unwrap(), [vec![1, 2, 3], vec![1, 2, 3]]);
        drop(run);
        wait(|| count.load(Ordering::SeqCst) == 0);
    }
}
