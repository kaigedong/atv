//! Portable compressed-media parsers and platform output contracts. No OS FFI.
use std::fmt;

#[derive(Debug)]
pub enum OutputError {
    InvalidData(&'static str),
    Unsupported(&'static str),
    Backend {
        operation: &'static str,
        status: i32,
    },
    /// No input was consumed. Drain/sleep briefly, then retry the same packet.
    Backpressure,
}
impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(reason) => write!(f, "invalid elementary stream: {reason}"),
            Self::Unsupported(reason) => write!(f, "unsupported media format: {reason}"),
            Self::Backend { operation, status } => write!(f, "{operation} failed ({status})"),
            Self::Backpressure => write!(f, "decoder input queue is full"),
        }
    }
}
impl std::error::Error for OutputError {}

#[derive(Debug, Default, Clone, Copy)]
pub struct OutputStats {
    pub video_frames: u64,
    /// Number of interleaved PCM sample frames accepted by the platform audio output.
    pub audio_frames: u64,
    pub width: u32,
    pub height: u32,
    pub position_us: i64,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    /// Internal evidence for PCM accepted by the platform audio output, not an audible-sink proof.
    pub audio_samples: u64,
    pub audio_nonzero_samples: u64,
    pub audio_peak: f64,
    pub audio_energy: f64,
}

// Some platform codec APIs require an unsigned timestamp. Preserve small negative normalized
// A/V offsets and B-frame PTS instead of clamping different timestamps to zero.
pub const TIMESTAMP_BIAS_US: i64 = 30_000_000;
pub fn codec_timestamp(pts: i64) -> Result<u64, OutputError> {
    let biased = pts
        .checked_add(TIMESTAMP_BIAS_US)
        .filter(|&value| value >= 0)
        .ok_or(OutputError::InvalidData("PTS outside normalized timeline"))?;
    Ok(biased as u64)
}

/// Bound queued compressed media by timeline span, including reordered B-frame
/// PTS. An empty queue always admits one packet regardless of its timestamp.
pub fn within_timestamp_window(
    timestamps: impl IntoIterator<Item = i64>,
    incoming: i64,
    max_span_us: u64,
) -> bool {
    let (mut first, mut last) = (incoming, incoming);
    for pts in timestamps {
        first = first.min(pts);
        last = last.max(pts);
    }
    last.abs_diff(first) <= max_span_us
}

#[derive(Default)]
pub struct PcmMeasurement {
    pub samples: u64,
    pub nonzero: u64,
    pub peak: f64,
    pub energy: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    Signed16Le,
    Float32Le,
}

pub fn measure_pcm(bytes: &[u8], encoding: PcmFormat) -> PcmMeasurement {
    let width = if encoding == PcmFormat::Float32Le {
        4
    } else {
        2
    };
    let mut result = PcmMeasurement::default();
    for sample in bytes.chunks_exact(width) {
        let value = if encoding == PcmFormat::Float32Le {
            f32::from_le_bytes(sample.try_into().unwrap()) as f64
        } else {
            i16::from_le_bytes(sample.try_into().unwrap()) as f64 / 32768.0
        };
        result.samples += 1;
        if value.is_finite() {
            result.nonzero += u64::from(value != 0.0);
            result.peak = result.peak.max(value.abs());
            result.energy += value * value;
        }
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioConfig {
    pub object_type: u8,
    pub rate_index: u8,
    pub channels: u8,
    pub channel_config: u8,
    pub sample_rate: u32,
}
impl AudioConfig {
    pub fn asc(self) -> [u8; 2] {
        [
            (self.object_type << 3) | (self.rate_index >> 1),
            ((self.rate_index & 1) << 7) | (self.channel_config << 3),
        ]
    }
}
#[derive(Debug)]
pub struct AdtsHeader {
    pub config: AudioConfig,
    pub frame_len: usize,
    pub header_len: usize,
}

pub fn adts_header(bytes: &[u8]) -> Result<Option<AdtsHeader>, OutputError> {
    if bytes.len() < 7 {
        return Ok(None);
    }
    if bytes[0] != 0xff || bytes[1] & 0xf6 != 0xf0 {
        return Err(OutputError::InvalidData("ADTS sync/layer"));
    }
    let rate_index = (bytes[2] >> 2) & 15;
    const RATES: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    let sample_rate = *RATES
        .get(rate_index as usize)
        .ok_or(OutputError::Unsupported(
            "ADTS explicit/reserved sample rate",
        ))?;
    let channel_config = ((bytes[2] & 1) << 2) | (bytes[3] >> 6);
    let channels = match channel_config {
        1..=6 => channel_config,
        7 => 8,
        _ => {
            return Err(OutputError::Unsupported(
                "AAC program-config-element channels",
            ));
        }
    };
    if bytes[6] & 3 != 0 {
        return Err(OutputError::Unsupported("ADTS multiple raw data blocks"));
    }
    let header_len = if bytes[1] & 1 == 0 { 9 } else { 7 };
    let frame_len =
        (((bytes[3] & 3) as usize) << 11) | ((bytes[4] as usize) << 3) | ((bytes[5] as usize) >> 5);
    if frame_len <= header_len {
        return Err(OutputError::InvalidData("ADTS frame length"));
    }
    Ok(Some(AdtsHeader {
        config: AudioConfig {
            object_type: (bytes[2] >> 6) + 1,
            rate_index,
            channels,
            channel_config,
            sample_rate,
        },
        frame_len,
        header_len,
    }))
}

pub fn annexb_nals(bytes: &[u8]) -> Vec<&[u8]> {
    let mut result = Vec::new();
    let mut start = None;
    let mut cursor = 0;
    while cursor + 3 <= bytes.len() {
        let prefix = if bytes[cursor..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if bytes[cursor..].starts_with(&[0, 0, 1]) {
            3
        } else {
            0
        };
        if prefix != 0 {
            if let Some(previous) = start
                && previous < cursor
            {
                result.push(&bytes[previous..cursor]);
            }
            start = Some(cursor + prefix);
            cursor += prefix;
        } else {
            cursor += 1;
        }
    }
    if let Some(previous) = start
        && previous < bytes.len()
    {
        result.push(&bytes[previous..]);
    }
    result
}

struct Bits {
    bytes: Vec<u8>,
    bit: usize,
}
impl Bits {
    fn from_sps(nal: &[u8]) -> Result<Self, OutputError> {
        if nal.len() < 4 || nal[0] & 31 != 7 {
            return Err(OutputError::InvalidData("AVC SPS"));
        }
        let mut bytes = Vec::new();
        let mut zeros = 0;
        for &byte in &nal[1..] {
            if zeros >= 2 && byte == 3 {
                zeros = 0;
                continue;
            }
            bytes.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        Ok(Self { bytes, bit: 0 })
    }
    fn read(&mut self, count: usize) -> Result<u32, OutputError> {
        if count > 32 || self.bit + count > self.bytes.len() * 8 {
            return Err(OutputError::InvalidData("truncated AVC SPS"));
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | (((self.bytes[self.bit / 8] >> (7 - self.bit % 8)) & 1) as u32);
            self.bit += 1;
        }
        Ok(value)
    }
    fn ue(&mut self) -> Result<u32, OutputError> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 30 {
                return Err(OutputError::InvalidData("AVC exponential-Golomb overflow"));
            }
        }
        Ok(((1u32 << zeros) - 1) + self.read(zeros)?)
    }
    fn se(&mut self) -> Result<i32, OutputError> {
        let code = self.ue()?;
        Ok(if code & 1 != 0 {
            code.div_ceil(2) as i32
        } else {
            -((code / 2) as i32)
        })
    }
}

pub fn sps_dimensions(nal: &[u8]) -> Result<(u32, u32), OutputError> {
    let mut bits = Bits::from_sps(nal)?;
    let profile = bits.read(8)?;
    bits.read(8)?;
    bits.read(8)?;
    bits.ue()?;
    let mut chroma = 1;
    let mut separate_colour_plane = false;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = bits.ue()?;
        if chroma > 3 {
            return Err(OutputError::InvalidData("AVC chroma format"));
        }
        if chroma == 3 {
            separate_colour_plane = bits.read(1)? != 0;
        }
        bits.ue()?;
        bits.ue()?;
        bits.read(1)?;
        if bits.read(1)? != 0 {
            for index in 0..if chroma == 3 { 12 } else { 8 } {
                if bits.read(1)? != 0 {
                    let mut last = 8i64;
                    let mut next = 8i64;
                    for _ in 0..if index < 6 { 16 } else { 64 } {
                        if next != 0 {
                            next = (last + bits.se()? as i64).rem_euclid(256);
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    }
    if separate_colour_plane {
        chroma = 0;
    }
    bits.ue()?;
    match bits.ue()? {
        0 => {
            bits.ue()?;
        }
        1 => {
            bits.read(1)?;
            bits.se()?;
            bits.se()?;
            let cycle = bits.ue()?;
            if cycle > 256 {
                return Err(OutputError::InvalidData("AVC POC cycle"));
            }
            for _ in 0..cycle {
                bits.se()?;
            }
        }
        2 => (),
        _ => return Err(OutputError::InvalidData("AVC picture order count")),
    }
    bits.ue()?;
    bits.read(1)?;
    let width_mbs = bits.ue()? as u64 + 1;
    let height_units = bits.ue()? as u64 + 1;
    let frame_only = bits.read(1)? as u64;
    if frame_only == 0 {
        bits.read(1)?;
    }
    bits.read(1)?;
    let (mut left, mut right, mut top, mut bottom) = (0u64, 0u64, 0u64, 0u64);
    if bits.read(1)? != 0 {
        left = bits.ue()? as u64;
        right = bits.ue()? as u64;
        top = bits.ue()? as u64;
        bottom = bits.ue()? as u64;
    }
    let crop_x = if chroma == 1 || chroma == 2 { 2 } else { 1 };
    let crop_y = (if chroma == 1 { 2 } else { 1 }) * (2 - frame_only);
    let width = (width_mbs * 16)
        .checked_sub((left + right) * crop_x)
        .ok_or(OutputError::InvalidData("AVC horizontal crop"))?;
    let height = (height_units * 16 * (2 - frame_only))
        .checked_sub((top + bottom) * crop_y)
        .ok_or(OutputError::InvalidData("AVC vertical crop"))?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(OutputError::Unsupported("AVC dimensions"));
    }
    Ok((width as u32, height as u32))
}

/// A single-thread-owned decoder and output device. Push success means accepted,
/// not displayed/audible. Drain reports actual rendering and audio acceptance.
/// Backpressure must leave the supplied input unconsumed so it can be retried.
pub trait MediaOutput {
    fn push_video(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), OutputError>;
    /// Optionally transfer the compressed allocation into the platform queue.
    /// Backpressure must preserve both bytes and ownership for an exact retry.
    /// Platforms without an owned-input API use the borrowed implementation.
    fn push_video_owned(&mut self, bytes: &mut Vec<u8>, pts_us: i64) -> Result<(), OutputError> {
        self.push_video(bytes, pts_us)
    }
    fn push_audio(&mut self, bytes: &[u8], pts_us: i64) -> Result<(), OutputError>;
    fn drain(&mut self) -> Result<OutputStats, OutputError>;
    /// Capacity of application-owned media payload buffers only. Excludes OS
    /// codec/Surface/audio allocations. Zero means this backend is uninstrumented.
    fn buffered_capacity(&self) -> usize {
        0
    }
}

/// Called on the player's owning thread, once per stream/discontinuity.
/// This trait and its outputs deliberately have no Send requirement: desktop
/// UI APIs may require creation, rendering and teardown on the main thread.
/// Session::new separately requires the factory to be Send when using a worker.
pub trait OutputFactory {
    fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError>;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_time_budget_preserves_reordered_pts_and_handles_extremes() {
        assert!(within_timestamp_window([], i64::MAX, 2_000_000));
        assert!(within_timestamp_window(
            [0, 120_000, 40_000],
            -40_000,
            2_000_000
        ));
        assert!(within_timestamp_window([0, 40_000], 2_000_000, 2_000_000));
        assert!(!within_timestamp_window([0, 40_000], 2_000_001, 2_000_000));
        assert!(!within_timestamp_window([i64::MIN], i64::MAX, 2_000_000));
    }
    #[test]
    fn codec_bias_keeps_negative_av_offsets_and_b_frames() {
        for pts in [-150_000, -40_000, 0, 40_000, 120_000, 6_000_000_000] {
            assert_eq!(
                codec_timestamp(pts).unwrap() as i64 - TIMESTAMP_BIAS_US,
                pts
            );
        }
        assert!(codec_timestamp(-TIMESTAMP_BIAS_US - 1).is_err());
        assert!(codec_timestamp(i64::MAX).is_err());
    }
    #[test]
    fn written_pcm_metrics_distinguish_silence_and_signal() {
        let silence = measure_pcm(&[0; 16], PcmFormat::Signed16Le);
        assert_eq!(silence.samples, 8);
        assert_eq!(silence.nonzero, 0);
        assert_eq!(silence.energy, 0.0);
        let samples = [0i16, 32767, -32768, 16384]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let signal = measure_pcm(&samples, PcmFormat::Signed16Le);
        assert_eq!(signal.samples, 4);
        assert_eq!(signal.nonzero, 3);
        assert_eq!(signal.peak, 1.0);
        assert!(signal.energy > 2.2 && signal.energy < 2.3);
        let float_samples = [0f32, 0.5, -1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let signal = measure_pcm(&float_samples, PcmFormat::Float32Le);
        assert_eq!(signal.nonzero, 2);
        assert_eq!(signal.energy, 1.25);
    }
    struct Writer(Vec<bool>);
    impl Writer {
        fn bit(&mut self, value: bool) {
            self.0.push(value);
        }
        fn bits(&mut self, value: u32, count: u32) {
            for shift in (0..count).rev() {
                self.bit(value & (1 << shift) != 0);
            }
        }
        fn ue(&mut self, value: u32) {
            let value = value + 1;
            let length = 32 - value.leading_zeros();
            for _ in 1..length {
                self.bit(false);
            }
            self.bits(value, length);
        }
        fn bytes(mut self) -> Vec<u8> {
            self.bit(true);
            while !self.0.len().is_multiple_of(8) {
                self.bit(false);
            }
            self.0
                .chunks(8)
                .map(|chunk| chunk.iter().fold(0, |v, &b| (v << 1) | u8::from(b)))
                .collect()
        }
    }
    fn baseline_sps(width: u32, height_units: u32, crop_bottom: u32) -> Vec<u8> {
        let mut b = Writer(Vec::new());
        b.bits(66, 8);
        b.bits(0, 8);
        b.bits(30, 8);
        b.ue(0);
        b.ue(0);
        b.ue(0);
        b.ue(0);
        b.ue(1);
        b.bit(false);
        b.ue(width / 16 - 1);
        b.ue(height_units - 1);
        b.bit(true);
        b.bit(true);
        b.bit(crop_bottom != 0);
        if crop_bottom != 0 {
            b.ue(0);
            b.ue(0);
            b.ue(0);
            b.ue(crop_bottom);
        }
        let mut result = vec![0x67];
        result.extend(b.bytes());
        result
    }
    #[test]
    fn baseline_and_cropped_sps() {
        assert_eq!(
            sps_dimensions(&baseline_sps(1280, 45, 0)).unwrap(),
            (1280, 720)
        );
        assert_eq!(
            sps_dimensions(&baseline_sps(1920, 68, 4)).unwrap(),
            (1920, 1080)
        );
    }
    #[test]
    fn truncated_sps_is_rejected() {
        let sps = baseline_sps(1920, 68, 4);
        for length in 0..sps.len() - 1 {
            assert!(sps_dimensions(&sps[..length]).is_err());
        }
        assert!(sps_dimensions(&[0x67, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
    #[test]
    fn annexb_handles_three_and_four_byte_prefixes() {
        assert_eq!(
            annexb_nals(&[0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 2]),
            vec![&[0x67, 1][..], &[0x68, 2][..]]
        );
        assert!(annexb_nals(&[1, 2, 3]).is_empty());
    }
    #[test]
    fn adts_aac_lc_config_and_crc() {
        let mut bytes = vec![0xff, 0xf1, 0x4c, 0x80, 3, 0x7f, 0xfc];
        bytes.resize(27, 0);
        let header = adts_header(&bytes).unwrap().unwrap();
        assert_eq!(header.config.sample_rate, 48000);
        assert_eq!(header.config.channels, 2);
        assert_eq!(header.config.asc(), [0x11, 0x90]);
        assert_eq!(header.frame_len, 27);
        assert_eq!(header.header_len, 7);
        bytes[1] = 0xf0;
        assert_eq!(adts_header(&bytes).unwrap().unwrap().header_len, 9);
        bytes[6] |= 1;
        assert!(adts_header(&bytes).is_err());
    }
    #[test]
    fn adts_incomplete_or_reserved_is_not_accepted() {
        assert!(adts_header(&[0xff, 0xf1, 0x4c]).unwrap().is_none());
        assert!(adts_header(&[0xff, 0xf1, 0x7c, 0x80, 3, 0x7f, 0xfc]).is_err());
        assert!(adts_header(&[0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
