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
    reconnect_at: Option<Instant>,
    failure_streak: u32,
    playing_since: Option<Instant>,
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
            reconnect_at: None,
            failure_streak: 0,
            playing_since: None,
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
        self.reconnect_at = None;
        self.failure_streak = 0;
        self.playing_since = None;
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
                let attempts = self.status.reconnect_attempts;
                let streak = self.failure_streak;
                let _ = self.select(pid, stream);
                self.status.reconnect_attempts = attempts;
                self.failure_streak = streak;
            }
            self.status.requested_quality = quality.id();
        }
    }
    pub fn pause(&mut self) {
        self.paused = true;
        self.pending_start = false;
        self.reconnect_at = None;
        self.playing_since = None;
        self.run.take();
        self.status.state = "paused";
        self.status.buffers = BufferStats::default();
    }
    pub fn resume(&mut self) {
        if self.paused {
            self.paused = false;
            self.pending_start = self.selected.is_some();
            if let Some((pid, _)) = &self.selected {
                let attempts = self.status.reconnect_attempts;
                self.status = Status::loading(pid.clone());
                self.status.reconnect_attempts = attempts;
                self.status.requested_quality = self.quality.id();
            }
        }
    }
    pub fn stop(&mut self) {
        self.run.take();
        self.selected = None;
        self.reconnect_at = None;
        self.failure_streak = 0;
        self.playing_since = None;
        self.pending_start = false;
        self.paused = false;
        self.status = Status::default();
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }
    fn tick_at(&mut self, now: Instant) {
        // A deadline never blocks the renderer or holds a network worker slot.
        if !self.paused
            && (self.pending_start || self.reconnect_at.is_some_and(|due| now >= due))
            && let Some(guard) = WorkerCount::try_acquire(&self.count)
        {
            self.pending_start = false;
            let reconnect = self.reconnect_at.take().is_some();
            if let Some((pid, stream_id)) = self.selected.clone() {
                let attempts = self
                    .status
                    .reconnect_attempts
                    .saturating_add(u64::from(reconnect));
                self.status = Status::loading(pid.clone());
                self.status.requested_quality = self.quality.id();
                self.status.reconnect_attempts = attempts;
                match Run::new(
                    self.source.clone(),
                    pid,
                    stream_id,
                    self.quality,
                    guard,
                    now,
                ) {
                    Ok(run) => self.run = Some(run),
                    Err(error) => self.failed(error, now),
                }
            }
        }
        if let Some(active) = &mut self.run {
            let result = active.advance(&mut self.factory, now);
            active.publish(&mut self.status, now);
            match result {
                Ok(()) => {
                    if self.status.state == "playing" {
                        self.status.error_code = "";
                        self.status.message = "";
                        let since = self.playing_since.get_or_insert(now);
                        if now.saturating_duration_since(*since) >= Duration::from_secs(20) {
                            self.failure_streak = 0;
                        }
                    } else {
                        self.playing_since = None;
                    }
                }
                Err(error) => self.failed(error, now),
            }
        }
    }
    fn failed(&mut self, error: PlaybackError, now: Instant) {
        self.run.take();
        self.playing_since = None;
        self.status.fail(&error);
        if error.retryable() && self.selected.is_some() && !self.paused {
            self.failure_streak = self.failure_streak.saturating_add(1);
            self.reconnect_at = Some(now + reconnect_delay(self.failure_streak));
            // Keep categorized diagnostics while recovery is waiting, without exposing
            // the terminal-error UI. Each attempt reacquires fresh permission/URLs.
            self.status.state = "loading";
        } else {
            self.reconnect_at = None;
        }
    }
}
impl<F: OutputFactory> Drop for Player<F> {
    fn drop(&mut self) {
        // Outputs must be destroyed before factory-owned windows/devices.
        self.run.take();
    }
}

fn reconnect_delay(streak: u32) -> Duration {
    Duration::from_secs((1u64 << streak.saturating_sub(1).min(4)).min(15))
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
    started: Instant,
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
        now: Instant,
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
            output_started: now,
            started: now,
            last_video: None,
            last_audio: None,
            last_progress: now,
            stats: OutputStats::default(),
            video_base: 0,
            audio_base: 0,
            position_base: 0,
        })
    }
    fn reset_output(
        &mut self,
        factory: &mut impl OutputFactory,
        now: Instant,
    ) -> Result<(), PlaybackError> {
        self.video_base = self.stats.video_frames;
        self.audio_base = self.stats.audio_frames;
        self.position_base = self.stats.position_us;
        self.output.take();
        self.last_video = None;
        self.last_audio = None;
        self.output_started = now;
        self.processor = Some(MediaProcessor::new(self.initial_url.clone()));
        self.output = Some(factory.create().map_err(|_| PlaybackError::Codec)?);
        Ok(())
    }
    fn drain(&mut self, now: Instant) -> Result<(), PlaybackError> {
        let Some(output) = &mut self.output else {
            return Ok(());
        };
        let mut stats = output.drain().map_err(|_| PlaybackError::Codec)?;
        stats.video_frames += self.video_base;
        stats.audio_frames += self.audio_base;
        stats.position_us += self.position_base;
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
    fn advance(
        &mut self,
        factory: &mut impl OutputFactory,
        now: Instant,
    ) -> Result<(), PlaybackError> {
        if self.expires_unix_seconds.is_some_and(|end| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                >= end
        }) {
            return Err(PlaybackError::PreviewEnded);
        }
        self.drain(now)?;
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
                    self.reset_output(factory, now)?;
                }
                Ok(Event::Fragment {
                    bytes,
                    discontinuity,
                }) => {
                    if discontinuity {
                        self.reset_output(factory, now)?;
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
        self.drain(now)?;
        self.measure_buffers();
        if self.preview_seconds > 0
            && self.first_output.is_some_and(|start| {
                now.saturating_duration_since(start).as_secs() >= self.preview_seconds
            })
        {
            return Err(PlaybackError::PreviewEnded);
        }
        if (self.first_output.is_none()
            && now.saturating_duration_since(self.started) >= Duration::from_secs(20))
            || now.saturating_duration_since(self.last_progress) >= Duration::from_secs(45)
        {
            return Err(PlaybackError::Stalled);
        }
        if self.output.is_some()
            && now.saturating_duration_since(self.output_started) >= Duration::from_secs(12)
            && (self
                .last_video
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(12))
                || self.last_audio.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_secs(12)
                }))
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
    fn publish(&self, status: &mut Status, now: Instant) {
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
        let recent = |last: Option<Instant>| {
            last.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(3))
        };
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
    use std::{cell::RefCell, rc::Rc, sync::atomic::AtomicBool};
    static SERIAL: Mutex<()> = Mutex::new(());

    #[derive(Default)]
    struct Source {
        active: AtomicUsize,
        maximum: AtomicUsize,
        calls: AtomicUsize,
        qualities: Mutex<Vec<Quality>>,
        channels: Mutex<Vec<(String, String)>>,
        failures: Mutex<VecDeque<PlaybackError>>,
        events: Mutex<Option<mpsc::SyncSender<Event>>>,
        hold_resolution: AtomicBool,
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
            self.channels
                .lock()
                .unwrap()
                .push((pid.into(), stream.into()));
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
            *self.events.lock().unwrap() = Some(events.clone());
            if let Some(error) = self.failures.lock().unwrap().pop_front() {
                let _ = events.send(Event::Failed(error));
                self.active.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            while self.hold_resolution.load(Ordering::SeqCst) && !cancel.cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            if cancel.cancelled() {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return;
            }
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
            self.stats.width = 1920;
            self.stats.height = 1080;
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
    fn inject_failure(source: &Source, error: PlaybackError) {
        source
            .events
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(Event::Failed(error))
            .unwrap();
    }
    fn playing_at(player: &mut Player<Factory>, now: Instant) {
        wait(|| {
            player.tick_at(now);
            player.status().state == "playing"
        });
    }
    fn recovering_at(player: &mut Player<Factory>, now: Instant) {
        wait(|| {
            player.tick_at(now);
            player.reconnect_at.is_some()
        });
        assert_eq!(player.status().state, "loading");
    }
    fn stopped(player: &mut Player<Factory>, source: &Source) {
        player.stop();
        wait(|| source.active.load(Ordering::SeqCst) == 0);
    }

    #[test]
    fn failure_before_first_frame_recovers_same_channel_and_quality() {
        let _serial = SERIAL.lock().unwrap();
        for error in [
            PlaybackError::Network,
            PlaybackError::Playlist,
            PlaybackError::Media,
            PlaybackError::Codec,
            PlaybackError::Stalled,
        ] {
            let (mut player, source, outputs) = fixture(true, false);
            source.failures.lock().unwrap().push_back(error);
            player.set_quality(Quality::Shd);
            player.select("1".into(), "2".into()).unwrap();
            let now = Instant::now();
            recovering_at(&mut player, now);
            assert_eq!(player.status().video_frames, 0);
            assert_eq!(*outputs.borrow(), 0);
            assert_eq!(player.status().reconnect_attempts, 0);
            assert!(!player.status().error_code.is_empty());
            player.tick_at(now + Duration::from_millis(999));
            assert_eq!(source.calls.load(Ordering::SeqCst), 1);
            playing_at(&mut player, now + Duration::from_secs(1));
            assert_eq!(player.status().reconnect_attempts, 1);
            assert!(player.status().video_frames > 0);
            assert_eq!(player.status().error_code, "");
            assert_eq!(player.status().message, "");
            assert_eq!(player.status().requested_quality, "shd");
            assert_eq!(
                *source.channels.lock().unwrap(),
                [("1".into(), "2".into()), ("1".into(), "2".into())]
            );
            assert_eq!(
                *source.qualities.lock().unwrap(),
                [Quality::Shd, Quality::Shd]
            );
            stopped(&mut player, &source);
        }
    }
    #[test]
    fn reconnect_backoff_uses_deadlines_and_caps_at_fifteen_seconds() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        source
            .failures
            .lock()
            .unwrap()
            .extend((0..8).map(|_| PlaybackError::Network));
        player.select("1".into(), "2".into()).unwrap();
        let mut now = Instant::now();
        for (index, delay) in [1, 2, 4, 8, 15, 15, 15].into_iter().enumerate() {
            recovering_at(&mut player, now);
            let due = now + Duration::from_secs(delay);
            assert_eq!(player.reconnect_at, Some(due));
            assert_eq!(player.status().reconnect_attempts, index as u64);
            assert_eq!(source.calls.load(Ordering::SeqCst), index + 1);
            for _ in 0..10 {
                player.tick_at(due - Duration::from_millis(1));
            }
            assert_eq!(source.calls.load(Ordering::SeqCst), index + 1);
            now = due;
            wait(|| {
                player.tick_at(now);
                player.failure_streak == index as u32 + 2
            });
        }
        assert_eq!(reconnect_delay(u32::MAX), Duration::from_secs(15));
        stopped(&mut player, &source);
    }
    #[test]
    fn successful_first_frames_do_not_reset_backoff_until_twenty_seconds_playing() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        source
            .failures
            .lock()
            .unwrap()
            .push_back(PlaybackError::Network);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        recovering_at(&mut player, now);
        let first = now + Duration::from_secs(1);
        playing_at(&mut player, first);
        for second in 1..20 {
            player.tick_at(first + Duration::from_secs(second));
            assert_eq!(player.failure_streak, 1);
        }
        inject_failure(&source, PlaybackError::Network);
        player.tick_at(first + Duration::from_secs(19));
        assert_eq!(player.failure_streak, 2);
        let second = first + Duration::from_secs(21);
        assert_eq!(player.reconnect_at, Some(second));
        playing_at(&mut player, second);
        for elapsed in 1..=20 {
            player.tick_at(second + Duration::from_secs(elapsed));
        }
        assert_eq!(player.failure_streak, 0);
        assert_eq!(player.status().reconnect_attempts, 2);
        let failure = second + Duration::from_secs(21);
        inject_failure(&source, PlaybackError::Codec);
        player.tick_at(failure);
        assert_eq!(player.reconnect_at, Some(failure + Duration::from_secs(1)));
        stopped(&mut player, &source);
    }
    #[test]
    fn smooth_playback_never_restarts_or_increments_reconnect_count() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        playing_at(&mut player, now);
        for second in 1..=120 {
            player.tick_at(now + Duration::from_secs(second));
            assert_eq!(player.status().state, "playing");
            assert!(player.reconnect_at.is_none());
            assert_eq!(player.status().reconnect_attempts, 0);
        }
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        stopped(&mut player, &source);
    }
    #[test]
    fn startup_without_output_times_out_at_twenty_seconds_and_recovers() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        source.hold_resolution.store(true, Ordering::SeqCst);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        player.tick_at(now);
        wait(|| source.calls.load(Ordering::SeqCst) == 1);
        player.tick_at(now + Duration::from_millis(19_999));
        assert!(player.reconnect_at.is_none());
        player.tick_at(now + Duration::from_secs(20));
        assert_eq!(player.status().error_code, "stalled");
        assert_eq!(player.status().state, "loading");
        assert_eq!(player.reconnect_at, Some(now + Duration::from_secs(21)));
        assert_eq!(player.status().video_frames, 0);
        source.hold_resolution.store(false, Ordering::SeqCst);
        playing_at(&mut player, now + Duration::from_secs(21));
        assert_eq!(player.status().reconnect_attempts, 1);
        stopped(&mut player, &source);
    }
    #[test]
    fn uninterrupted_video_with_missing_audio_recovers_after_output_timeout() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(false, false);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        wait(|| {
            player.tick_at(now);
            player.status().video_frames > 0
        });
        player.tick_at(now + Duration::from_millis(11_999));
        assert!(player.reconnect_at.is_none());
        player.tick_at(now + Duration::from_secs(12));
        assert_eq!(player.status().state, "loading");
        assert_eq!(player.status().error_code, "stalled");
        assert_eq!(player.reconnect_at, Some(now + Duration::from_secs(13)));
        player.factory.audio = true;
        playing_at(&mut player, now + Duration::from_secs(13));
        assert_eq!(player.status().reconnect_attempts, 1);
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        stopped(&mut player, &source);
    }
    #[test]
    fn elapsed_preview_deadline_is_terminal_even_with_advancing_frames() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        playing_at(&mut player, now);
        player.run.as_mut().unwrap().preview_seconds = 5;
        player.tick_at(now + Duration::from_secs(5));
        assert_eq!(player.status().state, "error");
        assert_eq!(player.status().error_code, "preview");
        assert!(player.reconnect_at.is_none());
        player.tick_at(now + Duration::from_secs(60));
        assert_eq!(player.status().reconnect_attempts, 0);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        stopped(&mut player, &source);
    }
    #[test]
    fn output_failure_releases_devices_then_automatically_reopens_them() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, outputs) = fixture(true, true);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        recovering_at(&mut player, now);
        assert_eq!(player.status().error_code, "codec");
        assert_eq!(*outputs.borrow(), 0);
        player.factory.fail = false;
        playing_at(&mut player, now + Duration::from_secs(1));
        assert_eq!(*outputs.borrow(), 1);
        assert_eq!(player.status().reconnect_attempts, 1);
        stopped(&mut player, &source);
        assert_eq!(*outputs.borrow(), 0);
    }
    #[test]
    fn pause_stop_select_and_drop_cancel_pending_reconnect() {
        let _serial = SERIAL.lock().unwrap();
        for command in ["pause", "stop", "select", "drop"] {
            let (mut player, source, _) = fixture(true, false);
            source
                .failures
                .lock()
                .unwrap()
                .push_back(PlaybackError::Network);
            player.set_quality(Quality::Shd);
            player.select("1".into(), "2".into()).unwrap();
            let now = Instant::now();
            recovering_at(&mut player, now);
            let later = now + Duration::from_secs(60);
            match command {
                "pause" => {
                    player.pause();
                    player.tick_at(later);
                    assert_eq!(player.status().state, "paused");
                    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
                    assert!(player.reconnect_at.is_none());
                    player.resume();
                    playing_at(&mut player, later);
                    assert_eq!(player.status().reconnect_attempts, 0);
                }
                "stop" => {
                    player.stop();
                    player.tick_at(later);
                    assert_eq!(player.status().state, "idle");
                    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
                    assert!(player.reconnect_at.is_none());
                }
                "select" => {
                    player.select("3".into(), "4".into()).unwrap();
                    playing_at(&mut player, later);
                    player.tick_at(later + Duration::from_secs(1));
                    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
                    assert_eq!(
                        source.channels.lock().unwrap().last().unwrap(),
                        &("3".into(), "4".into())
                    );
                    assert_eq!(player.status().pid, "3");
                    assert_eq!(player.status().reconnect_attempts, 0);
                }
                "drop" => {
                    drop(player);
                    wait(|| source.active.load(Ordering::SeqCst) == 0);
                    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
                    continue;
                }
                _ => unreachable!(),
            }
            assert!(
                source
                    .qualities
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|q| *q == Quality::Shd)
            );
            stopped(&mut player, &source);
        }
    }
    #[test]
    fn reconnect_counter_survives_pause_and_quality_changes_but_resets_on_select() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        source
            .failures
            .lock()
            .unwrap()
            .push_back(PlaybackError::Network);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        recovering_at(&mut player, now);
        let now = now + Duration::from_secs(1);
        playing_at(&mut player, now);
        assert_eq!(player.status().reconnect_attempts, 1);
        inject_failure(&source, PlaybackError::Network);
        player.tick_at(now);
        player.pause();
        player.tick_at(now + Duration::from_secs(60));
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
        player.resume();
        playing_at(&mut player, now + Duration::from_secs(60));
        assert_eq!(player.status().reconnect_attempts, 1);
        player.set_quality(Quality::Fhd);
        playing_at(&mut player, now + Duration::from_secs(61));
        assert_eq!(player.failure_streak, 2);
        assert_eq!(player.status().reconnect_attempts, 1);
        assert_eq!(player.status().requested_quality, "fhd");
        player.select("3".into(), "4".into()).unwrap();
        assert_eq!(player.status().reconnect_attempts, 0);
        stopped(&mut player, &source);
    }
    #[test]
    fn permission_preview_and_unsupported_failures_are_terminal() {
        let _serial = SERIAL.lock().unwrap();
        for (error, code) in [
            (PlaybackError::Authorization, "authorization"),
            (PlaybackError::PreviewEnded, "preview"),
            (PlaybackError::Challenge, "challenge"),
            (PlaybackError::Encryption, "encryption"),
            (PlaybackError::Cmg, "cmg"),
            (PlaybackError::Cancelled, "cancelled"),
        ] {
            let (mut player, source, _) = fixture(true, false);
            source.failures.lock().unwrap().push_back(error);
            player.select("1".into(), "2".into()).unwrap();
            let now = Instant::now();
            wait(|| {
                player.tick_at(now);
                player.status().state == "error"
            });
            player.tick_at(now + Duration::from_secs(60));
            assert_eq!(player.status().state, "error");
            assert_eq!(player.status().error_code, code);
            assert_eq!(player.status().reconnect_attempts, 0);
            assert!(player.reconnect_at.is_none());
            assert_eq!(source.calls.load(Ordering::SeqCst), 1);
            stopped(&mut player, &source);
        }
    }
    #[test]
    fn scheduled_reconnect_waits_for_shared_worker_capacity() {
        let _serial = SERIAL.lock().unwrap();
        let (mut player, source, _) = fixture(true, false);
        source
            .failures
            .lock()
            .unwrap()
            .push_back(PlaybackError::Network);
        player.select("1".into(), "2".into()).unwrap();
        let now = Instant::now();
        recovering_at(&mut player, now);
        wait(|| player.count.load(Ordering::SeqCst) == 0);
        let first = WorkerCount::try_acquire(&player.count).unwrap();
        let second = WorkerCount::try_acquire(&player.count).unwrap();
        player.tick_at(now + Duration::from_secs(1));
        assert_eq!(player.status().state, "loading");
        assert_eq!(player.status().reconnect_attempts, 0);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        drop(first);
        playing_at(&mut player, now + Duration::from_secs(1));
        assert_eq!(player.status().reconnect_attempts, 1);
        assert_eq!(player.count.load(Ordering::SeqCst), 2);
        stopped(&mut player, &source);
        drop(second);
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
                    player.status().state == "loading" && player.status().error_code == "codec"
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
        let mut run = Run::new(
            source.clone(),
            "1".into(),
            "2".into(),
            Quality::Auto,
            guard,
            Instant::now(),
        )
        .unwrap();
        let mut factory = Factory {
            outputs: Rc::new(RefCell::new(0)),
            audio: true,
            fail: false,
        };
        wait(|| {
            run.advance(&mut factory, Instant::now()).unwrap();
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
        run.advance(&mut factory, Instant::now()).unwrap();
        assert_eq!(run.packets.len(), 1);
        assert_eq!(run.packets.front().unwrap().data.as_ptr(), pointer);
        assert!(received.lock().unwrap().is_none());
        run.advance(&mut factory, Instant::now()).unwrap();
        assert!(run.packets.is_empty());
        assert_eq!(received.lock().unwrap().as_ref().unwrap().as_ptr(), pointer);
        assert_eq!(*attempts.lock().unwrap(), [vec![1, 2, 3], vec![1, 2, 3]]);
        drop(run);
        wait(|| count.load(Ordering::SeqCst) == 0);
    }
}
