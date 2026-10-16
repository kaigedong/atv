//! Android-only NDK codec and audio output adapter.
use atv_media::{
    AudioConfig, MediaOutput, OutputError as CodecError, OutputStats, PcmFormat, TIMESTAMP_BIAS_US,
    adts_header, annexb_nals, codec_timestamp, measure_pcm, sps_dimensions,
    within_timestamp_window,
};
use std::collections::VecDeque;
use std::ffi::{CStr, c_void};
use std::ptr;

const VIDEO_LIMIT: usize = 8 * 1024 * 1024;
const AUDIO_LIMIT: usize = 1024 * 1024;
const PCM_LIMIT: usize = 1024 * 1024;
const PACKET_LIMIT: usize = 256;
const VIDEO_QUEUE_SPAN_US: u64 = 2_000_000;
const CONFIG_FLAG: u32 = 2;
const EOS_FLAG: u32 = 4;
#[cfg(feature = "legacy-android")]
use crate::legacy_api::*;
#[cfg(not(feature = "legacy-android"))]
use crate::ndk_api::*;
#[repr(C)]
pub(crate) struct Timespec {
    pub seconds: std::ffi::c_long,
    pub nanos: std::ffi::c_long,
}
unsafe extern "C" {
    fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
}

fn check(operation: &'static str, status: i32) -> Result<(), CodecError> {
    if status == 0 {
        Ok(())
    } else {
        Err(CodecError::Backend { operation, status })
    }
}
// c_long is 32 bits on KitKat and 64 bits on the modern ABIs.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn now_ns() -> Result<i64, CodecError> {
    let mut time = Timespec {
        seconds: 0,
        nanos: 0,
    };
    check("clock_gettime", unsafe { clock_gettime(1, &mut time) })?;
    Ok((time.seconds as i64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.nanos as i64))
}
struct Format(*mut AMediaFormat);
impl Format {
    fn new() -> Result<Self, CodecError> {
        let format = unsafe { AMediaFormat_new() };
        if format.is_null() {
            return Err(CodecError::Backend {
                operation: "AMediaFormat_new",
                status: -1,
            });
        }
        Ok(Self(format))
    }
    fn integer(&self, key: &CStr, value: i32) {
        unsafe { AMediaFormat_setInt32(self.0, key.as_ptr(), value) }
    }
    fn string(&self, key: &CStr, value: &CStr) {
        unsafe { AMediaFormat_setString(self.0, key.as_ptr(), value.as_ptr()) }
    }
    fn buffer(&self, key: &CStr, value: &[u8]) {
        unsafe { AMediaFormat_setBuffer(self.0, key.as_ptr(), value.as_ptr().cast(), value.len()) }
    }
    fn get(&self, key: &CStr) -> Option<i32> {
        let mut value = 0;
        unsafe { AMediaFormat_getInt32(self.0, key.as_ptr(), &mut value) }.then_some(value)
    }
}
impl Drop for Format {
    fn drop(&mut self) {
        unsafe { AMediaFormat_delete(self.0) };
    }
}
struct Decoder {
    pointer: *mut AMediaCodec,
    started: bool,
}
impl Decoder {
    fn new(mime: &CStr, format: &Format, window: *mut c_void) -> Result<Self, CodecError> {
        let pointer = unsafe { AMediaCodec_createDecoderByType(mime.as_ptr()) };
        if pointer.is_null() {
            return Err(CodecError::Unsupported("Android decoder unavailable"));
        }
        let mut decoder = Self {
            pointer,
            started: false,
        };
        check("AMediaCodec_configure", unsafe {
            AMediaCodec_configure(pointer, format.0, window, ptr::null_mut(), 0)
        })?;
        check("AMediaCodec_start", unsafe { AMediaCodec_start(pointer) })?;
        decoder.started = true;
        Ok(decoder)
    }
    fn queue(&self, packet: &Packet) -> Result<bool, CodecError> {
        let index = unsafe { AMediaCodec_dequeueInputBuffer(self.pointer, 0) };
        if index == -1 {
            return Ok(false);
        }
        if index < 0 {
            return Err(CodecError::Backend {
                operation: "AMediaCodec_dequeueInputBuffer",
                status: index as i32,
            });
        }
        let mut capacity = 0;
        let destination =
            unsafe { AMediaCodec_getInputBuffer(self.pointer, index as usize, &mut capacity) };
        if destination.is_null() || packet.bytes.len() > capacity {
            return Err(CodecError::Unsupported("decoder input buffer capacity"));
        }
        unsafe { ptr::copy_nonoverlapping(packet.bytes.as_ptr(), destination, packet.bytes.len()) };
        check("AMediaCodec_queueInputBuffer", unsafe {
            AMediaCodec_queueInputBuffer(
                self.pointer,
                index as usize,
                0,
                packet.bytes.len(),
                codec_timestamp(packet.pts)?,
                0,
            )
        })?;
        Ok(true)
    }
    fn output(&self) -> Result<Option<Output>, CodecError> {
        let mut info = BufferInfo::default();
        let index = unsafe { AMediaCodec_dequeueOutputBuffer(self.pointer, &mut info, 0) };
        if index >= 0 {
            info.pts = info.pts.saturating_sub(TIMESTAMP_BIAS_US);
        }
        match index {
            -1 => Ok(None),
            -2 => Ok(Some(Output::FormatChanged)),
            -3 => Ok(Some(Output::BuffersChanged)),
            index if index >= 0 => Ok(Some(Output::Buffer {
                index: index as usize,
                info,
            })),
            _ => Err(CodecError::Backend {
                operation: "AMediaCodec_dequeueOutputBuffer",
                status: index as i32,
            }),
        }
    }
    fn format(&self) -> Result<Format, CodecError> {
        let format = unsafe { AMediaCodec_getOutputFormat(self.pointer) };
        if format.is_null() {
            Err(CodecError::InvalidData("missing decoder output format"))
        } else {
            Ok(Format(format))
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            if self.started {
                AMediaCodec_stop(self.pointer);
            }
            AMediaCodec_delete(self.pointer);
        }
    }
}
struct Stream {
    pointer: *mut AAudioStream,
    bytes_per_frame: usize,
    rate: u32,
    encoding: i32,
}
impl Stream {
    fn open(rate: i32, channels: i32, encoding: i32) -> Result<Self, CodecError> {
        let (format, sample_bytes) = match encoding {
            2 => (1, 2),
            4 => (2, 4),
            _ => return Err(CodecError::Unsupported("PCM encoding")),
        };
        struct Builder(*mut AAudioStreamBuilder);
        impl Drop for Builder {
            fn drop(&mut self) {
                unsafe { AAudioStreamBuilder_delete(self.0) };
            }
        }
        let mut pointer = ptr::null_mut();
        check("AAudio_createStreamBuilder", unsafe {
            AAudio_createStreamBuilder(&mut pointer)
        })?;
        if pointer.is_null() {
            return Err(CodecError::InvalidData("null AAudio builder"));
        }
        let builder = Builder(pointer);
        unsafe {
            AAudioStreamBuilder_setDirection(builder.0, 0);
            AAudioStreamBuilder_setFormat(builder.0, format);
            AAudioStreamBuilder_setSampleRate(builder.0, rate);
            AAudioStreamBuilder_setChannelCount(builder.0, channels);
            AAudioStreamBuilder_setSharingMode(builder.0, 1);
            AAudioStreamBuilder_setPerformanceMode(builder.0, 12);
        }
        let mut stream = ptr::null_mut();
        check("AAudioStreamBuilder_openStream", unsafe {
            AAudioStreamBuilder_openStream(builder.0, &mut stream)
        })?;
        if stream.is_null() {
            return Err(CodecError::InvalidData("null AAudio stream"));
        }
        let output = Self {
            pointer: stream,
            bytes_per_frame: channels as usize * sample_bytes,
            rate: rate as u32,
            encoding,
        };
        if unsafe { AAudioStream_getSampleRate(stream) } != rate
            || unsafe { AAudioStream_getChannelCount(stream) } != channels
            || unsafe { AAudioStream_getFormat(stream) } != format
        {
            return Err(CodecError::Unsupported("AAudio format negotiation"));
        }
        check("AAudioStream_requestStart", unsafe {
            AAudioStream_requestStart(stream)
        })?;
        Ok(output)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            AAudioStream_requestStop(self.pointer);
            AAudioStream_close(self.pointer);
        }
    }
}
struct Packet {
    bytes: Vec<u8>,
    pts: i64,
}
#[derive(Clone, Copy)]
enum Output {
    FormatChanged,
    BuffersChanged,
    Buffer { index: usize, info: BufferInfo },
}
struct Pcm {
    bytes: Vec<u8>,
    offset: usize,
    pts: i64,
}

pub struct NativeOutput {
    window: *mut c_void,
    video: Option<Decoder>,
    audio: Option<Decoder>,
    stream: Option<Stream>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    audio_config: Option<AudioConfig>,
    video_queue: VecDeque<Packet>,
    audio_queue: VecDeque<Packet>,
    pcm_queue: VecDeque<Pcm>,
    video_bytes: usize,
    audio_bytes: usize,
    pcm_bytes: usize,
    adts: Vec<u8>,
    adts_pts: Option<i64>,
    pending_video: Option<Output>,
    pending_audio: Option<Output>,
    epoch: Option<(i64, i64)>,
    stats: OutputStats,
}
impl NativeOutput {
    /// # Safety
    /// `window` must be an acquired ANativeWindow, valid until this output is
    /// dropped. Construct, use and drop the output on the same worker thread.
    pub unsafe fn new(window: *mut c_void) -> Result<Self, CodecError> {
        if window.is_null() {
            return Err(CodecError::InvalidData("null video Surface"));
        }
        Ok(Self {
            window,
            video: None,
            audio: None,
            stream: None,
            sps: None,
            pps: None,
            audio_config: None,
            video_queue: VecDeque::new(),
            audio_queue: VecDeque::new(),
            pcm_queue: VecDeque::new(),
            video_bytes: 0,
            audio_bytes: 0,
            pcm_bytes: 0,
            adts: Vec::new(),
            adts_pts: None,
            pending_video: None,
            pending_audio: None,
            epoch: None,
            stats: OutputStats::default(),
        })
    }

    pub fn push_video(&mut self, annexb: &[u8], pts_us: i64) -> Result<(), CodecError> {
        if annexb.is_empty() {
            return Ok(());
        }
        self.prepare_video(annexb, pts_us)?;
        self.enqueue_video(annexb.to_vec(), pts_us)
    }

    pub fn push_video_owned(
        &mut self,
        annexb: &mut Vec<u8>,
        pts_us: i64,
    ) -> Result<(), CodecError> {
        if annexb.is_empty() {
            return Ok(());
        }
        // All backpressure checks precede taking the allocation. A blocked
        // caller retains the exact access unit and PTS without a clone.
        self.prepare_video(annexb, pts_us)?;
        self.enqueue_video(std::mem::take(annexb), pts_us)
    }

    fn prepare_video(&mut self, annexb: &[u8], pts_us: i64) -> Result<(), CodecError> {
        if annexb.len() > VIDEO_LIMIT {
            return Err(CodecError::InvalidData("AVC access unit too large"));
        }
        if self.video_queue.len() >= PACKET_LIMIT
            || self.video_bytes + annexb.len() > VIDEO_LIMIT
            || !within_timestamp_window(
                self.video_queue.iter().map(|p| p.pts),
                pts_us,
                VIDEO_QUEUE_SPAN_US,
            )
        {
            return Err(CodecError::Backpressure);
        }
        for nal in annexb_nals(annexb) {
            let end = nal
                .iter()
                .rposition(|&byte| byte != 0)
                .map_or(0, |end| end + 1);
            let nal = &nal[..end];
            if nal.is_empty() {
                continue;
            }
            match nal[0] & 31 {
                7 => {
                    let dimensions = sps_dimensions(nal)?;
                    if self.sps.as_deref() != Some(nal) {
                        self.video.take();
                        self.pending_video = None;
                        self.video_queue.clear();
                        self.video_bytes = 0;
                        self.sps = Some(nal.to_vec());
                        self.pps = None;
                        self.stats.width = dimensions.0;
                        self.stats.height = dimensions.1;
                    }
                }
                8 if self.pps.as_deref() != Some(nal) => {
                    self.video.take();
                    self.pending_video = None;
                    if self.pps.is_some() {
                        self.video_queue.clear();
                        self.video_bytes = 0;
                    }
                    self.pps = Some(nal.to_vec());
                }
                _ => (),
            }
        }
        if self.video.is_none()
            && let (Some(sps), Some(pps)) = (&self.sps, &self.pps)
        {
            let format = Format::new()?;
            format.string(c"mime", c"video/avc");
            format.integer(c"width", self.stats.width as i32);
            format.integer(c"height", self.stats.height as i32);
            // Scale compressed input buffers to the stream rather than reserving
            // 4 MiB for every decoder slot on small-memory TV boxes.
            let input_limit = (u64::from(self.stats.width) * u64::from(self.stats.height) * 3 / 4)
                .clamp(256 * 1024, 4 * 1024 * 1024) as i32;
            format.integer(c"max-input-size", input_limit);
            let mut csd0 = vec![0, 0, 0, 1];
            csd0.extend_from_slice(sps);
            let mut csd1 = vec![0, 0, 0, 1];
            csd1.extend_from_slice(pps);
            format.buffer(c"csd-0", &csd0);
            format.buffer(c"csd-1", &csd1);
            self.video = Some(Decoder::new(c"video/avc", &format, self.window)?);
        }
        Ok(())
    }

    fn enqueue_video(&mut self, bytes: Vec<u8>, pts: i64) -> Result<(), CodecError> {
        self.video_bytes += bytes.len();
        self.video_queue.push_back(Packet { bytes, pts });
        self.pump_inputs()?;
        self.drain()?;
        Ok(())
    }

    pub fn push_audio(&mut self, adts: &[u8], pts_us: i64) -> Result<(), CodecError> {
        if adts.len() > 256 * 1024 {
            return Err(CodecError::InvalidData("ADTS packet limit"));
        }
        if self.adts.len() + adts.len() > 256 * 1024 {
            return Err(CodecError::Backpressure);
        }
        if self.audio_queue.len() >= PACKET_LIMIT || self.audio_bytes + adts.len() > AUDIO_LIMIT {
            return Err(CodecError::Backpressure);
        }
        if self.adts.is_empty() {
            self.adts_pts = Some(pts_us);
        }
        self.adts.extend_from_slice(adts);
        self.stage_audio()?;
        self.pump_inputs()?;
        self.drain()?;
        Ok(())
    }

    fn stage_audio(&mut self) -> Result<(), CodecError> {
        while let Some(header) = adts_header(&self.adts)? {
            if self.adts.len() < header.frame_len {
                break;
            }
            if self.audio_queue.len() >= PACKET_LIMIT
                || self.audio_bytes + header.frame_len > AUDIO_LIMIT
            {
                break;
            }
            if self.audio_config != Some(header.config) {
                self.configure_audio(header.config)?;
            }
            let pts = self.adts_pts.unwrap_or(0);
            let bytes = self.adts[header.header_len..header.frame_len].to_vec();
            self.audio_bytes += bytes.len();
            self.audio_queue.push_back(Packet { bytes, pts });
            self.adts.drain(..header.frame_len);
            self.adts_pts =
                Some(pts.saturating_add(1_024_000_000 / header.config.sample_rate as i64));
        }
        Ok(())
    }

    fn configure_audio(&mut self, config: AudioConfig) -> Result<(), CodecError> {
        self.stream.take();
        self.audio.take();
        self.pending_audio = None;
        self.audio_queue.clear();
        self.audio_bytes = 0;
        self.pcm_queue.clear();
        self.pcm_bytes = 0;
        let format = Format::new()?;
        format.string(c"mime", c"audio/mp4a-latm");
        format.integer(c"sample-rate", config.sample_rate as i32);
        format.integer(c"channel-count", config.channels as i32);
        format.integer(c"aac-profile", config.object_type as i32);
        format.integer(c"pcm-encoding", 2);
        format.integer(c"max-input-size", 32768);
        format.buffer(c"csd-0", &config.asc());
        self.audio = Some(Decoder::new(c"audio/mp4a-latm", &format, ptr::null_mut())?);
        self.audio_config = Some(config);
        Ok(())
    }

    fn pump_inputs(&mut self) -> Result<(), CodecError> {
        if let Some(codec) = &self.video {
            for _ in 0..4 {
                let Some(packet) = self.video_queue.front() else {
                    break;
                };
                if !codec.queue(packet)? {
                    break;
                }
                self.video_bytes -= packet.bytes.len();
                self.video_queue.pop_front();
            }
        }
        if let Some(codec) = &self.audio {
            for _ in 0..4 {
                let Some(packet) = self.audio_queue.front() else {
                    break;
                };
                if !codec.queue(packet)? {
                    break;
                }
                self.audio_bytes -= packet.bytes.len();
                self.audio_queue.pop_front();
            }
        }
        Ok(())
    }

    fn target_ns(&mut self, pts: i64) -> Result<i64, CodecError> {
        if self.epoch.is_none() {
            self.epoch = Some((pts, now_ns()?.saturating_add(30_000_000)));
        }
        let (media, clock) = self.epoch.unwrap();
        Ok(clock.saturating_add(pts.saturating_sub(media).saturating_mul(1000)))
    }

    fn drain_video(&mut self) -> Result<(), CodecError> {
        for _ in 0..8 {
            let Some(codec) = self.video.as_ref() else {
                return Ok(());
            };
            let output = match self.pending_video.take() {
                Some(output) => Some(output),
                None => codec.output()?,
            };
            let Some(output) = output else {
                break;
            };
            match output {
                Output::BuffersChanged => (),
                Output::FormatChanged => {
                    let format = self.video.as_ref().unwrap().format()?;
                    let width = format.get(c"width").unwrap_or(self.stats.width as i32);
                    let height = format.get(c"height").unwrap_or(self.stats.height as i32);
                    let crop_width = match (format.get(c"crop-left"), format.get(c"crop-right")) {
                        (Some(l), Some(r)) if r >= l => r - l + 1,
                        _ => width,
                    };
                    let crop_height = match (format.get(c"crop-top"), format.get(c"crop-bottom")) {
                        (Some(t), Some(b)) if b >= t => b - t + 1,
                        _ => height,
                    };
                    if !(1..=8192).contains(&crop_width) || !(1..=8192).contains(&crop_height) {
                        return Err(CodecError::Unsupported("decoded video dimensions"));
                    }
                    self.stats.width = crop_width as u32;
                    self.stats.height = crop_height as u32;
                }
                Output::Buffer { index, info } => {
                    let render = info.flags & CONFIG_FLAG == 0
                        && (info.size > 0 || info.flags & EOS_FLAG == 0);
                    let target = if render { self.target_ns(info.pts)? } else { 0 };
                    if render && target > now_ns()?.saturating_add(VIDEO_LEAD_NS) {
                        self.pending_video = Some(output);
                        break;
                    }
                    let pointer = self.video.as_ref().unwrap().pointer;
                    if render {
                        check("AMediaCodec_releaseOutputBufferAtTime", unsafe {
                            AMediaCodec_releaseOutputBufferAtTime(pointer, index, target)
                        })?;
                        self.stats.video_frames += 1;
                        self.stats.position_us = self.stats.position_us.max(info.pts);
                    } else {
                        check("AMediaCodec_releaseOutputBuffer", unsafe {
                            AMediaCodec_releaseOutputBuffer(pointer, index, false)
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    fn audio_format(&mut self) -> Result<(), CodecError> {
        let codec = self
            .audio
            .as_ref()
            .ok_or(CodecError::InvalidData("AAC decoder missing"))?;
        let format = codec.format()?;
        let fallback = self
            .audio_config
            .ok_or(CodecError::InvalidData("AAC configuration missing"))?;
        let rate = format
            .get(c"sample-rate")
            .unwrap_or(fallback.sample_rate as i32);
        let channels = format
            .get(c"channel-count")
            .unwrap_or(fallback.channels as i32);
        let encoding = format.get(c"pcm-encoding").unwrap_or(2);
        if !(7350..=192000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err(CodecError::Unsupported("decoded audio format"));
        }
        self.stream.take();
        self.pcm_queue.clear();
        self.pcm_bytes = 0;
        self.stream = Some(Stream::open(rate, channels, encoding)?);
        self.stats.audio_sample_rate = rate as u32;
        self.stats.audio_channels = channels as u32;
        Ok(())
    }

    fn drain_audio(&mut self) -> Result<(), CodecError> {
        for _ in 0..8 {
            let Some(codec) = self.audio.as_ref() else {
                return Ok(());
            };
            let output = match self.pending_audio.take() {
                Some(output) => Some(output),
                None => codec.output()?,
            };
            let Some(output) = output else {
                break;
            };
            match output {
                Output::BuffersChanged => (),
                Output::FormatChanged => self.audio_format()?,
                Output::Buffer { index, info } => {
                    if info.size < 0 || info.offset < 0 {
                        return Err(CodecError::InvalidData("negative PCM buffer range"));
                    }
                    if info.size as usize > PCM_LIMIT {
                        return Err(CodecError::InvalidData("PCM output limit"));
                    }
                    if self.pcm_bytes + info.size as usize > PCM_LIMIT {
                        self.pending_audio = Some(output);
                        break;
                    }
                    if self.stream.is_none() {
                        self.audio_format()?;
                    }
                    let pointer = self.audio.as_ref().unwrap().pointer;
                    if info.size > 0 && info.flags & CONFIG_FLAG == 0 {
                        let mut capacity = 0;
                        let source =
                            unsafe { AMediaCodec_getOutputBuffer(pointer, index, &mut capacity) };
                        let offset = info.offset as usize;
                        let size = info.size as usize;
                        if source.is_null() || offset > capacity || size > capacity - offset {
                            return Err(CodecError::InvalidData("PCM buffer range"));
                        }
                        let frame_bytes = self.stream.as_ref().unwrap().bytes_per_frame;
                        if !size.is_multiple_of(frame_bytes) {
                            return Err(CodecError::InvalidData("unaligned PCM frames"));
                        }
                        let bytes = unsafe { std::slice::from_raw_parts(source.add(offset), size) }
                            .to_vec();
                        self.pcm_bytes += bytes.len();
                        self.pcm_queue.push_back(Pcm {
                            bytes,
                            offset: 0,
                            pts: info.pts,
                        });
                    }
                    check("AMediaCodec_releaseOutputBuffer", unsafe {
                        AMediaCodec_releaseOutputBuffer(pointer, index, false)
                    })?;
                }
            }
        }
        Ok(())
    }

    fn drain_pcm(&mut self) -> Result<(), CodecError> {
        for _ in 0..8 {
            let Some(stream) = self.stream.as_ref() else {
                break;
            };
            let Some(chunk) = self.pcm_queue.front() else {
                break;
            };
            let frame_bytes = stream.bytes_per_frame;
            let rate = stream.rate;
            let frames_done = chunk.offset / frame_bytes;
            let pts = chunk
                .pts
                .saturating_add((frames_done as i64).saturating_mul(1_000_000) / rate as i64);
            if self.target_ns(pts)? > now_ns()?.saturating_add(20_000_000) {
                break;
            }
            let stream = self.stream.as_ref().unwrap();
            let chunk = self.pcm_queue.front_mut().unwrap();
            let frames = ((chunk.bytes.len() - chunk.offset) / frame_bytes).min(512) as i32;
            let written = unsafe {
                AAudioStream_write(
                    stream.pointer,
                    chunk.bytes.as_ptr().add(chunk.offset).cast(),
                    frames,
                    2_000_000,
                )
            };
            if written < 0 {
                return Err(CodecError::Backend {
                    operation: "AAudioStream_write",
                    status: written,
                });
            }
            if written == 0 {
                break;
            }
            if written > frames {
                return Err(CodecError::InvalidData("AAudio frame count"));
            }
            let written_bytes = written as usize * frame_bytes;
            let measurement = measure_pcm(
                &chunk.bytes[chunk.offset..chunk.offset + written_bytes],
                if stream.encoding == 4 {
                    PcmFormat::Float32Le
                } else {
                    PcmFormat::Signed16Le
                },
            );
            self.stats.audio_samples += measurement.samples;
            self.stats.audio_nonzero_samples += measurement.nonzero;
            self.stats.audio_peak = self.stats.audio_peak.max(measurement.peak);
            self.stats.audio_energy += measurement.energy;
            chunk.offset += written_bytes;
            self.pcm_bytes -= written_bytes;
            self.stats.audio_frames += written as u64;
            self.stats.position_us = self
                .stats
                .position_us
                .max(pts.saturating_add(written as i64 * 1_000_000 / rate as i64));
            if chunk.offset == chunk.bytes.len() {
                self.pcm_queue.pop_front();
            }
        }
        Ok(())
    }

    pub fn drain(&mut self) -> Result<OutputStats, CodecError> {
        self.drain_pcm()?;
        self.pump_inputs()?;
        self.stage_audio()?;
        self.pump_inputs()?;
        self.drain_video()?;
        self.drain_audio()?;
        self.drain_pcm()?;
        Ok(self.stats)
    }
}
impl Drop for NativeOutput {
    fn drop(&mut self) {
        self.stream.take();
        self.audio.take();
        self.video.take();
    }
}

impl MediaOutput for NativeOutput {
    fn push_video(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), CodecError> {
        NativeOutput::push_video(self, bytes, pts_us)
    }
    fn push_audio(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), CodecError> {
        NativeOutput::push_audio(self, bytes, pts_us)
    }
    fn push_video_owned(&mut self, bytes: &mut Vec<u8>, pts_us: i64) -> Result<(), CodecError> {
        NativeOutput::push_video_owned(self, bytes, pts_us)
    }
    fn buffered_capacity(&self) -> usize {
        self.video_queue
            .iter()
            .map(|p| p.bytes.capacity())
            .sum::<usize>()
            + self
                .audio_queue
                .iter()
                .map(|p| p.bytes.capacity())
                .sum::<usize>()
            + self
                .pcm_queue
                .iter()
                .map(|p| p.bytes.capacity())
                .sum::<usize>()
            + self.adts.capacity()
            + self.sps.as_ref().map_or(0, Vec::capacity)
            + self.pps.as_ref().map_or(0, Vec::capacity)
    }
    fn drain(&mut self) -> Result<OutputStats, CodecError> {
        NativeOutput::drain(self)
    }
}

// KitKat has no timed buffer release. Leave future video pending until its
// target rather than sleeping through several frames and starving AudioTrack.
#[cfg(feature = "legacy-android")]
const VIDEO_LEAD_NS: i64 = 0;
#[cfg(not(feature = "legacy-android"))]
const VIDEO_LEAD_NS: i64 = 40_000_000;
