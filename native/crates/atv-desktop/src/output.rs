//! GStreamer decodes only; SDL owns the real window and queued PCM output.
//! No network access is delegated to either library. All media comes from the
//! shared Rust authorization/HLS/CMG pipeline, through bounded appsrc queues.
use atv_media::{
    MediaOutput, OutputError, OutputFactory, OutputStats, PcmFormat, TIMESTAMP_BIAS_US,
    codec_timestamp, measure_pcm,
};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSrc};
use gstreamer_video::{VideoFrameRef, VideoInfo, prelude::*};
use sdl2::{audio::AudioQueue, pixels::PixelFormatEnum, rect::Rect, render::WindowCanvas};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

pub struct Host {
    pub canvas: WindowCanvas,
    pub audio: AudioQueue<i16>,
    pub snapshot: Option<PathBuf>,
}
pub struct DesktopFactory(pub Rc<RefCell<Host>>);
impl OutputFactory for DesktopFactory {
    fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError> {
        Ok(Box::new(DesktopOutput::new(self.0.clone())?))
    }
}
fn device_error(_: impl std::fmt::Debug) -> OutputError {
    OutputError::Backend {
        operation: "desktop decode/output",
        status: -1,
    }
}
struct Decoder {
    pipeline: gst::Pipeline,
    input: AppSrc,
    output: AppSink,
    input_limit: u64,
}
impl Decoder {
    fn new(description: &str, input_limit: u64) -> Result<Self, OutputError> {
        let pipeline = gst::parse::launch(description)
            .map_err(device_error)?
            .downcast::<gst::Pipeline>()
            .map_err(device_error)?;
        let input = pipeline
            .by_name("input")
            .ok_or_else(|| device_error(()))?
            .downcast::<AppSrc>()
            .map_err(device_error)?;
        let output = pipeline
            .by_name("output")
            .ok_or_else(|| device_error(()))?
            .downcast::<AppSink>()
            .map_err(device_error)?;
        // Construct the RAII owner before entering Playing, including on error.
        let decoder = Self {
            pipeline,
            input,
            output,
            input_limit,
        };
        decoder.input.set_max_bytes(input_limit);
        decoder
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(device_error)?;
        Ok(decoder)
    }
    fn push(&self, bytes: &[u8], pts_us: i64) -> Result<(), OutputError> {
        if bytes.len() as u64 > self.input_limit {
            return Err(OutputError::InvalidData("packet exceeds decoder limit"));
        }
        if self.input.current_level_bytes() + bytes.len() as u64 > self.input_limit {
            return Err(OutputError::Backpressure);
        }
        let pts = codec_timestamp(pts_us)?
            .checked_mul(1000)
            .ok_or(OutputError::InvalidData("timestamp overflow"))?;
        let mut buffer = gst::Buffer::from_mut_slice(bytes.to_vec());
        buffer
            .get_mut()
            .ok_or_else(|| device_error(()))?
            .set_pts(gst::ClockTime::from_nseconds(pts));
        self.input.push_buffer(buffer).map_err(device_error)?;
        Ok(())
    }
    fn pull(&self) -> Option<gst::Sample> {
        self.output.try_pull_sample(gst::ClockTime::ZERO)
    }
    fn check(&self) -> Result<(), OutputError> {
        if self
            .pipeline
            .bus()
            .is_some_and(|bus| bus.pop_filtered(&[gst::MessageType::Error]).is_some())
        {
            return Err(device_error(()));
        }
        Ok(())
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

struct DesktopOutput {
    host: Rc<RefCell<Host>>,
    video: Decoder,
    audio: Decoder,
    pending_video: Option<gst::Sample>,
    pending_audio: Option<gst::Sample>,
    clock: Option<Instant>,
    stats: OutputStats,
}
impl DesktopOutput {
    fn new(host: Rc<RefCell<Host>>) -> Result<Self, OutputError> {
        let video = Decoder::new(
            "appsrc name=input is-live=true format=time block=false caps=video/x-h264,stream-format=byte-stream,alignment=au ! h264parse ! decodebin ! videoconvert ! video/x-raw,format=RGBA ! appsink name=output sync=false async=false max-buffers=4 drop=false",
            8 * 1024 * 1024,
        )?;
        let audio = Decoder::new(
            "appsrc name=input is-live=true format=time block=false caps=audio/mpeg,mpegversion=4,stream-format=adts ! aacparse ! decodebin ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=48000,channels=2 ! appsink name=output sync=false async=false max-buffers=8 drop=false",
            1024 * 1024,
        )?;
        host.borrow().audio.clear();
        host.borrow().audio.resume();
        Ok(Self {
            host,
            video,
            audio,
            pending_video: None,
            pending_audio: None,
            clock: None,
            stats: OutputStats::default(),
        })
    }
    fn pts(sample: &gst::Sample) -> Result<i64, OutputError> {
        let pts = sample
            .buffer()
            .and_then(|b| b.pts())
            .ok_or(OutputError::InvalidData("decoded sample has no timestamp"))?;
        Ok((pts.nseconds() / 1000) as i64 - TIMESTAMP_BIAS_US)
    }
    fn render(&mut self, sample: &gst::Sample, pts: i64) -> Result<(), OutputError> {
        let caps = sample.caps().ok_or_else(|| device_error(()))?;
        let info = VideoInfo::from_caps(caps).map_err(device_error)?;
        let buffer = sample.buffer().ok_or_else(|| device_error(()))?;
        let frame = VideoFrameRef::from_buffer_ref_readable(buffer, &info).map_err(device_error)?;
        let (width, height) = (info.width(), info.height());
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err(OutputError::InvalidData("video dimensions"));
        }
        let stride = usize::try_from(frame.plane_stride()[0]).map_err(device_error)?;
        let pixels = frame.plane_data(0).map_err(device_error)?;
        let mut host = self.host.borrow_mut();
        let creator = host.canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormatEnum::RGBA32, width, height)
            .map_err(device_error)?;
        texture.update(None, pixels, stride).map_err(device_error)?;
        let (window_width, window_height) = host.canvas.output_size().map_err(device_error)?;
        let scale = (window_width as f64 / width as f64).min(window_height as f64 / height as f64);
        let (w, h) = (
            (width as f64 * scale) as u32,
            (height as f64 * scale) as u32,
        );
        let destination = Rect::new(
            ((window_width - w) / 2) as i32,
            ((window_height - h) / 2) as i32,
            w,
            h,
        );
        host.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        host.canvas.clear();
        host.canvas
            .copy(&texture, None, destination)
            .map_err(device_error)?;
        if self.stats.video_frames >= 50
            && let Some(path) = host.snapshot.take()
        {
            let mut pixels = host
                .canvas
                .read_pixels(None, PixelFormatEnum::RGBA32)
                .map_err(device_error)?;
            let surface = sdl2::surface::Surface::from_data(
                &mut pixels,
                window_width,
                window_height,
                window_width * 4,
                PixelFormatEnum::RGBA32,
            )
            .map_err(device_error)?;
            surface.save_bmp(path).map_err(device_error)?;
        }
        host.canvas.present();
        self.stats.video_frames += 1;
        self.stats.width = width;
        self.stats.height = height;
        self.stats.position_us = self.stats.position_us.max(pts);
        Ok(())
    }
    fn write_audio(&mut self, sample: &gst::Sample, pts: i64) -> Result<(), OutputError> {
        let buffer = sample.buffer().ok_or_else(|| device_error(()))?;
        let map = buffer.map_readable().map_err(device_error)?;
        let bytes = map.as_slice();
        if !bytes.len().is_multiple_of(4) {
            return Err(OutputError::InvalidData("stereo PCM alignment"));
        }
        let pcm: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        self.host
            .borrow()
            .audio
            .queue_audio(&pcm)
            .map_err(device_error)?;
        let measure = measure_pcm(bytes, PcmFormat::Signed16Le);
        self.stats.audio_frames += (pcm.len() / 2) as u64;
        self.stats.audio_samples += measure.samples;
        self.stats.audio_nonzero_samples += measure.nonzero;
        self.stats.audio_peak = self.stats.audio_peak.max(measure.peak);
        self.stats.audio_energy += measure.energy;
        self.stats.audio_sample_rate = 48000;
        self.stats.audio_channels = 2;
        self.stats.position_us = self
            .stats
            .position_us
            .max(pts + (pcm.len() / 2) as i64 * 1_000_000 / 48000);
        Ok(())
    }
}
impl MediaOutput for DesktopOutput {
    fn push_video(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), OutputError> {
        self.video.push(bytes, pts_us)
    }
    fn push_audio(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), OutputError> {
        self.audio.push(bytes, pts_us)
    }
    fn drain(&mut self) -> Result<OutputStats, OutputError> {
        self.video.check()?;
        self.audio.check()?;
        if self.pending_video.is_none() {
            self.pending_video = self.video.pull();
        }
        if self.pending_audio.is_none() {
            self.pending_audio = self.audio.pull();
        }
        if self.clock.is_none() && self.pending_video.is_some() && self.pending_audio.is_some() {
            self.clock = Some(Instant::now());
        }
        let Some(clock) = self.clock else {
            return Ok(self.stats);
        };
        let elapsed = clock.elapsed().as_micros() as i64;
        if let Some(sample) = &self.pending_video {
            let pts = Self::pts(sample)?;
            if pts <= elapsed + 5000 {
                let sample = self.pending_video.take().unwrap();
                self.render(&sample, pts)?;
            }
        }
        for _ in 0..8 {
            if self.pending_audio.is_none() {
                self.pending_audio = self.audio.pull();
            }
            let Some(sample) = &self.pending_audio else {
                break;
            };
            let pts = Self::pts(sample)?;
            let queued_bytes = self.host.borrow().audio.size();
            if queued_bytes >= 48_000
                || pts > elapsed + i64::from(queued_bytes) * 1_000_000 / 192000 + 20_000
            {
                break;
            }
            let sample = self.pending_audio.take().unwrap();
            self.write_audio(&sample, pts)?;
        }
        Ok(self.stats)
    }
}
impl Drop for DesktopOutput {
    fn drop(&mut self) {
        self.host.borrow().audio.pause();
        self.host.borrow().audio.clear();
    }
}
