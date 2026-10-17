//! Rust-owned authorization, bounded HLS acquisition and elementary media processing.
//! Network work runs separately from the short-latency native output worker.
use crate::Quality;
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use atv_auth::{LiveResolver, ResolveError};
use atv_cmg::CmgSession;
use atv_core::{
    hls::{self, Playlist},
    transport::{Cancellation, HttpClient, HttpError},
    ts::{Codec, Demux, ElementaryPacket, annex_b_nals, fragment_alignment},
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::mpsc::{SyncSender, TrySendError},
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;
use url::Url;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub requested_quality: &'static str,
    pub effective_quality: &'static str,
    pub available_qualities: Vec<&'static str>,
    pub state: &'static str,
    pub pid: String,
    pub position_ms: u64,
    pub video_width: u32,
    pub video_height: u32,
    pub video_frames: u64,
    pub audio_frames: u64,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    pub audio_nonzero_samples: u64,
    pub audio_peak: f64,
    pub audio_rms: f64,
    /// Automatic reacquisitions since explicit channel selection, excluding initial startup.
    pub reconnect_attempts: u64,
    pub error_code: &'static str,
    pub message: &'static str,
    pub buffers: BufferStats,
}
/// Selected renderer-owned payload capacities, not RSS/PSS or total heap use.
/// Download channel, in-flight HTTP/TLS, allocator metadata and system codec
/// memory are deliberately outside this measurement. The high-water mark is
/// sampled before/after batch submission, not inside transforms or OS calls.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferStats {
    pub fragment_capacity_bytes: usize,
    pub demux_capacity_bytes: usize,
    pub pending_packet_capacity_bytes: usize,
    pub output_capacity_bytes: usize,
    pub render_capacity_bytes: usize,
    pub render_peak_capacity_bytes: usize,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            requested_quality: "auto",
            effective_quality: "",
            available_qualities: Vec::new(),
            state: "idle",
            pid: String::new(),
            position_ms: 0,
            video_width: 0,
            video_height: 0,
            video_frames: 0,
            audio_frames: 0,
            audio_sample_rate: 0,
            audio_channels: 0,
            audio_nonzero_samples: 0,
            audio_peak: 0.0,
            audio_rms: 0.0,
            reconnect_attempts: 0,
            error_code: "",
            message: "",
            buffers: BufferStats::default(),
        }
    }
}
impl Status {
    pub fn loading(pid: String) -> Self {
        Self {
            state: "loading",
            pid,
            ..Self::default()
        }
    }
    pub fn fail(&mut self, error: &PlaybackError) {
        self.state = "error";
        // The player tears down the run on errors; retain only its sampled peak.
        self.buffers = BufferStats {
            render_peak_capacity_bytes: self.buffers.render_peak_capacity_bytes,
            ..BufferStats::default()
        };
        (self.error_code, self.message) = match error {
            PlaybackError::Authorization => ("authorization", "当前频道暂不可观看，请检查观看权限"),
            PlaybackError::Challenge => ("challenge", "服务端返回了尚未支持的播放验证"),
            PlaybackError::Network => ("network", "网络连接失败，请重试"),
            PlaybackError::Playlist => ("playlist", "直播列表暂不可用，请重试"),
            PlaybackError::Encryption => ("encryption", "当前直播加密格式尚未支持"),
            PlaybackError::Media => ("media", "直播媒体格式异常，请重试"),
            PlaybackError::Cmg => ("cmg", "当前直播的媒体变换尚未实现"),
            PlaybackError::Codec => ("codec", "设备音视频输出失败，请重试"),
            PlaybackError::PreviewEnded => ("preview", "本次试看已结束"),
            PlaybackError::Cancelled => ("cancelled", "播放已停止"),
            PlaybackError::Stalled => ("stalled", "直播连接中断，请重试"),
        };
    }
}

/// Errors contain categories only. Never include signed resources or API payloads.
#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("server denied playback")]
    Authorization,
    #[error("unsupported server challenge")]
    Challenge,
    #[error("network failed")]
    Network,
    #[error("invalid HLS playlist")]
    Playlist,
    #[error("unsupported HLS encryption")]
    Encryption,
    #[error("invalid media")]
    Media,
    #[error("CMG transform is not implemented")]
    Cmg,
    #[error("native codec/output failed")]
    Codec,
    #[error("preview ended")]
    PreviewEnded,
    #[error("cancelled")]
    Cancelled,
    #[error("live stream stalled")]
    Stalled,
}
impl PlaybackError {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Network | Self::Playlist | Self::Media | Self::Codec | Self::Stalled
        )
    }
}
impl From<HttpError> for PlaybackError {
    fn from(error: HttpError) -> Self {
        match error {
            HttpError::Cancelled => Self::Cancelled,
            _ => Self::Network,
        }
    }
}
impl From<ResolveError> for PlaybackError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::Cancelled => Self::Cancelled,
            ResolveError::Server { .. } => Self::Authorization,
            ResolveError::UnsupportedChallenge => Self::Challenge,
            _ => Self::Network,
        }
    }
}

// No Debug implementation: even diagnostic formatting must not expose the URL.
pub enum Event {
    Qualities {
        effective: String,
        available: Vec<String>,
    },
    Resolved {
        initial_url: String,
        preview_seconds: u64,
        expires_unix_seconds: Option<u64>,
    },
    Fragment {
        bytes: Vec<u8>,
        discontinuity: bool,
    },
    Failed(PlaybackError),
}
fn send(
    tx: &SyncSender<Event>,
    cancel: &Cancellation,
    mut event: Event,
) -> Result<(), PlaybackError> {
    loop {
        cancel.check()?;
        match tx.try_send(event) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => return Err(PlaybackError::Cancelled),
            Err(TrySendError::Full(value)) => {
                event = value;
                sleep(cancel, Duration::from_millis(10))?;
            }
        }
    }
}
fn sleep(cancel: &Cancellation, duration: Duration) -> Result<(), PlaybackError> {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        cancel.check()?;
        thread::sleep(
            end.saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20)),
        );
    }
    cancel.check()?;
    Ok(())
}

pub fn acquire(
    resolver: &LiveResolver,
    pid: &str,
    stream_id: &str,
    cancel: &Cancellation,
    tx: &SyncSender<Event>,
) {
    acquire_quality(resolver, pid, stream_id, Quality::Auto, cancel, tx);
}
pub fn acquire_quality(
    resolver: &LiveResolver,
    pid: &str,
    stream_id: &str,
    quality: Quality,
    cancel: &Cancellation,
    tx: &SyncSender<Event>,
) {
    if let Err(error) = acquire_inner(resolver, pid, stream_id, quality, cancel, tx)
        && !cancel.cancelled()
    {
        let _ = send(tx, cancel, Event::Failed(error));
    }
}
fn acquire_inner(
    resolver: &LiveResolver,
    pid: &str,
    stream_id: &str,
    quality: Quality,
    cancel: &Cancellation,
    tx: &SyncSender<Event>,
) -> Result<(), PlaybackError> {
    let info = resolver.resolve_quality_cancellable(
        pid,
        stream_id,
        quality.request(),
        1,
        cancel.flag().as_ref(),
    )?;
    cancel.check()?;
    let mut url = Url::parse(info.url()).map_err(|_| PlaybackError::Playlist)?;
    send(
        tx,
        cancel,
        Event::Resolved {
            initial_url: info.url().to_owned(),
            preview_seconds: info.preview_seconds,
            expires_unix_seconds: paid_deadline(&info.extra_info)?,
        },
    )?;
    send(
        tx,
        cancel,
        Event::Qualities {
            effective: info.definition.clone(),
            available: info.definitions(),
        },
    )?;
    let client = HttpClient::new("ATV-Native/0.1 (Android TV; Rust)")?;
    let mut last_sequence = None;
    let mut last_progress = Instant::now();
    let mut failures = 0u32;
    let mut master_depth = 0;
    let mut keys = HashMap::<Url, [u8; 16]>::new();
    loop {
        cancel.check()?;
        // A key endpoint may rotate its value without changing URI. Cache only
        // within this playlist response, never across live playlist reloads.
        keys.clear();
        let resource = match client.get_resource(&url, 1024 * 1024, cancel) {
            Ok(raw) => {
                failures = 0;
                raw
            }
            Err(error) => {
                failures += 1;
                if failures >= 4 {
                    return Err(error.into());
                }
                sleep(cancel, Duration::from_millis(500 * u64::from(failures)))?;
                continue;
            }
        };
        url = resource.final_url;
        let playlist = hls::parse(
            std::str::from_utf8(&resource.bytes).map_err(|_| PlaybackError::Playlist)?,
            &url,
        )
        .map_err(|_| PlaybackError::Playlist)?;
        let media = match playlist {
            Playlist::Master(variants) => {
                master_depth += 1;
                if master_depth > 4 {
                    return Err(PlaybackError::Playlist);
                }
                let variant = quality
                    .choose_with_height(&variants, info.definition_height())
                    .ok_or(PlaybackError::Playlist)?;
                // External audio playlists and fragmented MP4 require a separate demux path.
                // Explicit failure prevents a silent video-only success claim.
                if variant.audio.is_some() {
                    return Err(PlaybackError::Media);
                }
                url = variant.url.clone();
                continue;
            }
            Playlist::Media(media) => media,
        };
        if media.map.is_some() {
            return Err(PlaybackError::Media);
        }
        let start = if last_sequence.is_none() {
            media.segments.len().saturating_sub(3)
        } else {
            0
        };
        for segment in &media.segments[start..] {
            if last_sequence.is_some_and(|n| segment.sequence <= n) {
                continue;
            }
            let missed = last_sequence.is_some_and(|n| segment.sequence > n.saturating_add(1));
            let mut bytes = retry_segment(&client, segment, cancel)?;
            if let Some(key) = &segment.key {
                if key.method != "AES-128" || key.format.as_deref().is_some_and(|f| f != "identity")
                {
                    return Err(PlaybackError::Encryption);
                }
                let key_url = key.uri.as_ref().ok_or(PlaybackError::Encryption)?;
                let secret = if let Some(secret) = keys.get(key_url) {
                    *secret
                } else {
                    let raw = client.get(key_url, 16, cancel)?;
                    let secret: [u8; 16] = raw.try_into().map_err(|_| PlaybackError::Encryption)?;
                    if keys.len() >= 8 {
                        keys.clear();
                    }
                    keys.insert(key_url.clone(), secret);
                    secret
                };
                let iv = key.iv.unwrap_or((segment.sequence as u128).to_be_bytes());
                bytes = decrypt_segment(bytes, &secret, &iv)?;
            }
            send(
                tx,
                cancel,
                Event::Fragment {
                    bytes,
                    discontinuity: segment.discontinuity || missed,
                },
            )?;
            last_sequence = Some(segment.sequence);
            last_progress = Instant::now();
        }
        if media.end_list {
            return Err(PlaybackError::Stalled);
        }
        if last_progress.elapsed() > Duration::from_secs(30) {
            return Err(PlaybackError::Stalled);
        }
        sleep(
            cancel,
            Duration::from_secs_f64((media.target_duration / 2.0).clamp(0.25, 2.0)),
        )?;
    }
}

/// The official player treats `left_duration` as a truthy purchase marker and
/// formats `end_time` as Unix seconds. Its duration unit is not inferred here.
pub fn paid_deadline(extra: &serde_json::Value) -> Result<Option<u64>, PlaybackError> {
    fn object(value: &serde_json::Value) -> Option<serde_json::Value> {
        if value.is_object() {
            Some(value.clone())
        } else {
            value.as_str().and_then(|s| serde_json::from_str(s).ok())
        }
    }
    let Some(extra) = object(extra) else {
        return Ok(None);
    };
    let Some(vip) = object(&extra["vip_info"]) else {
        return Ok(None);
    };
    let left = &vip["left_duration"];
    let purchased = match left {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        serde_json::Value::String(value) => !value.is_empty(),
        _ => true,
    };
    if !purchased {
        return Ok(None);
    }
    let end = vip["end_time"]
        .as_u64()
        .or_else(|| vip["end_time"].as_str().and_then(|s| s.parse().ok()))
        .filter(|&n| n > 0)
        .ok_or(PlaybackError::Authorization)?;
    Ok(Some(end))
}
fn retry_segment(
    client: &HttpClient,
    segment: &hls::Segment,
    cancel: &Cancellation,
) -> Result<Vec<u8>, PlaybackError> {
    for attempt in 0..3 {
        match client.get_range(&segment.url, 16 * 1024 * 1024, segment.range, cancel) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if attempt == 2 || cancel.cancelled() => return Err(error.into()),
            Err(_) => sleep(cancel, Duration::from_millis(250 * (attempt + 1)))?,
        }
    }
    unreachable!()
}
fn decrypt_segment(
    mut bytes: Vec<u8>,
    key: &[u8; 16],
    iv: &[u8; 16],
) -> Result<Vec<u8>, PlaybackError> {
    let decrypted = cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut bytes)
        .map_err(|_| PlaybackError::Encryption)?;
    let length = decrypted.len();
    bytes.truncate(length);
    Ok(bytes)
}

/// One shared 33-bit PTS origin preserves audio/video offsets and B-frame reordering.
#[derive(Default)]
pub struct MediaClock {
    origin: Option<u64>,
}
impl MediaClock {
    pub fn timestamp_us(&mut self, pts: u64) -> i64 {
        let origin = *self.origin.get_or_insert(pts);
        let wrapped = pts.wrapping_sub(origin) & ((1u64 << 33) - 1);
        let signed = if wrapped >= 1u64 << 32 {
            wrapped as i64 - (1i64 << 33)
        } else {
            wrapped as i64
        };
        signed * 1_000_000 / 90_000
    }
}
pub struct MediaProcessor {
    demux: Demux,
    cmg: CmgSession,
    clock: MediaClock,
}
pub struct AccessUnit {
    pub codec: Codec,
    pub pts_us: i64,
    pub data: Vec<u8>,
}
/// Retain one downloaded fragment and process short TS batches under output
/// backpressure, rather than materialising a whole fragment of access units.
pub(crate) struct MediaFragment {
    bytes: Vec<u8>,
    cursor: usize,
}
impl MediaFragment {
    pub(crate) fn new(bytes: Vec<u8>) -> Result<Self, PlaybackError> {
        let cursor = fragment_alignment(&bytes).map_err(|_| PlaybackError::Media)?;
        Ok(Self { bytes, cursor })
    }
    pub(crate) fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
    pub(crate) fn next(
        &mut self,
        processor: &mut MediaProcessor,
    ) -> Result<Vec<AccessUnit>, PlaybackError> {
        let end = (self.cursor + 188 * 128).min(self.bytes.len());
        let result = processor.fragment(&self.bytes[self.cursor..end])?;
        self.cursor = end;
        Ok(result)
    }
    pub(crate) fn finished(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}
impl MediaProcessor {
    pub(crate) fn buffered_capacity(&self) -> usize {
        self.demux.buffered_capacity()
    }
    pub fn new(initial_url: String) -> Self {
        Self {
            demux: Demux::default(),
            cmg: CmgSession::new(initial_url),
            clock: MediaClock::default(),
        }
    }
    pub fn fragment(&mut self, bytes: &[u8]) -> Result<Vec<AccessUnit>, PlaybackError> {
        let packets = self
            .demux
            .push_fragment(bytes)
            .map_err(|_| PlaybackError::Media)?;
        packets
            .into_iter()
            .map(|packet| self.packet(packet))
            .collect()
    }
    fn packet(&mut self, packet: ElementaryPacket) -> Result<AccessUnit, PlaybackError> {
        let pts_us = self.clock.timestamp_us(packet.pts_90khz);
        let data = match packet.codec {
            Codec::Avc => {
                let nals = annex_b_nals(&packet.data);
                if nals.is_empty() {
                    return Err(PlaybackError::Media);
                }
                let mut clear = Vec::with_capacity(packet.data.len());
                for nal in nals {
                    let transformed = self
                        .cmg
                        .transform_annex_b_nal(nal)
                        .map_err(|_| PlaybackError::Cmg)?;
                    clear.extend_from_slice(&[0, 0, 0, 1]);
                    clear.extend_from_slice(&transformed);
                }
                clear
            }
            Codec::Aac => packet.data,
            _ => return Err(PlaybackError::Media),
        };
        Ok(AccessUnit {
            codec: packet.codec,
            pts_us,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;
    fn fixture() -> Vec<u8> {
        fn section_packet(pid: u16, mut section: Vec<u8>) -> [u8; 188] {
            let mut crc = 0xffff_ffffu32;
            for &byte in &section {
                crc ^= u32::from(byte) << 24;
                for _ in 0..8 {
                    crc = (crc << 1)
                        ^ if crc & 0x8000_0000 != 0 {
                            0x04c1_1db7
                        } else {
                            0
                        };
                }
            }
            section.extend_from_slice(&crc.to_be_bytes());
            let mut packet = [0xff; 188];
            packet[..5].copy_from_slice(&[0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10, 0]);
            packet[5..5 + section.len()].copy_from_slice(&section);
            packet
        }
        let mut ts = Vec::new();
        ts.extend_from_slice(&section_packet(
            0,
            vec![0, 0xb0, 13, 0, 1, 0xc1, 0, 0, 0, 1, 0xe0, 32],
        ));
        ts.extend_from_slice(&section_packet(
            32,
            vec![
                2, 0xb0, 23, 0, 1, 0xc1, 0, 0, 0xe1, 0, 0xf0, 0, 0x1b, 0xe1, 0, 0xf0, 0, 0x0f,
                0xe1, 1, 0xf0, 0,
            ],
        ));
        let mut video_counter = 0;
        let mut audio_counter = 0;
        for frame in 0..4 {
            let mut video = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
            video.extend_from_slice(&[0, 0, 0, 1, 0x65]);
            video.resize(40_000 + frame, 0x11);
            let mut audio = vec![
                0, 0, 1, 0xc0, 0, 11, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1, 0xff, 0xf1, 0xaa,
            ];
            for (pid, pes, counter) in [
                (256, &mut video, &mut video_counter),
                (257, &mut audio, &mut audio_counter),
            ] {
                for (i, payload) in pes.chunks(182).enumerate() {
                    let mut packet = [0xff; 188];
                    let adaptation = 183 - payload.len();
                    packet[..6].copy_from_slice(&[
                        0x47,
                        ((pid >> 8) as u8) | if i == 0 { 0x40 } else { 0 },
                        pid as u8,
                        0x30 | *counter,
                        adaptation as u8,
                        0,
                    ]);
                    packet[5 + adaptation..].copy_from_slice(payload);
                    ts.extend_from_slice(&packet);
                    *counter = (*counter + 1) & 15;
                }
            }
        }
        ts
    }
    #[test]
    fn incremental_batches_preserve_pes_across_batches_and_http_fragments() {
        let bytes = fixture();
        let summarize = |units: Vec<AccessUnit>| {
            units
                .into_iter()
                .map(|u| (u.codec, u.pts_us, u.data))
                .collect::<Vec<_>>()
        };
        let mut eager = MediaProcessor::new(String::new());
        let mut expected = eager.fragment(&bytes).unwrap();
        expected.extend(eager.fragment(&bytes).unwrap());
        assert!(expected.iter().any(|u| u.codec == Codec::Avc));
        assert!(expected.iter().any(|u| u.codec == Codec::Aac));
        let mut processor = MediaProcessor::new(String::new());
        let mut output = Vec::new();
        for data in [&bytes[..188 * 150], &bytes[188 * 150..], &bytes[..]] {
            let mut fragment = MediaFragment::new(data.to_vec()).unwrap();
            while !fragment.finished() {
                output.extend(fragment.next(&mut processor).unwrap());
            }
        }
        assert_eq!(summarize(output), summarize(expected));
        assert!(MediaFragment::new(bytes[..bytes.len() - 1].to_vec()).is_err());
    }
    #[test]
    fn clock_wrap_and_b_frames_keep_common_av_origin() {
        let mut clock = MediaClock::default();
        let start = (1u64 << 33) - 90_000;
        assert_eq!(clock.timestamp_us(start), 0);
        assert_eq!(clock.timestamp_us(0), 1_000_000);
        assert_eq!(clock.timestamp_us(9_000), 1_100_000);
        assert_eq!(clock.timestamp_us((1u64 << 33) - 9_000), 900_000);
        assert_eq!(clock.timestamp_us(start - 9_000), -100_000);
    }
    #[test]
    fn standard_hls_aes_cbc_requires_valid_padding() {
        let key = [7; 16];
        let iv = [3; 16];
        let clear = b"authorized sample plaintext";
        let encrypted = cbc::Encryptor::<aes::Aes128>::new((&key).into(), (&iv).into())
            .encrypt_padded_vec_mut::<Pkcs7>(clear);
        assert_eq!(decrypt_segment(encrypted, &key, &iv).unwrap(), clear);
        assert!(decrypt_segment(vec![1; 17], &key, &iv).is_err());
    }
    #[test]
    fn cancelled_bounded_queue_does_not_block_shutdown() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let cancel = Cancellation::default();
        send(&tx, &cancel, Event::Failed(PlaybackError::Network)).unwrap();
        cancel.cancel();
        assert!(matches!(
            send(&tx, &cancel, Event::Failed(PlaybackError::Network)),
            Err(PlaybackError::Cancelled)
        ));
    }
    #[test]
    fn reconnect_attempts_are_serialized_as_cumulative_camel_case_counter() {
        let mut status = Status::loading("1".into());
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["reconnectAttempts"], 0);
        status.reconnect_attempts = 7;
        status.fail(&PlaybackError::Network);
        assert_eq!(
            serde_json::to_value(&status).unwrap()["reconnectAttempts"],
            7
        );
    }
    #[test]
    fn error_status_contains_no_raw_protocol_details() {
        let mut status = Status::loading("600001859".to_owned());
        status.fail(&PlaybackError::Authorization);
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("authorization"));
        assert!(!json.contains("https://"));
        assert_eq!(status.state, "error");
        assert_eq!(status.video_frames, 0);
    }
    #[test]
    fn purchased_deadline_preserves_server_epoch_without_guessing_duration_unit() {
        let extra =
            serde_json::json!({"vip_info":"{\"left_duration\":123,\"end_time\":1700000000}"});
        assert_eq!(paid_deadline(&extra).unwrap(), Some(1700000000));
        assert_eq!(
            paid_deadline(&serde_json::Value::String(extra.to_string())).unwrap(),
            Some(1700000000)
        );
        assert_eq!(
            paid_deadline(&serde_json::json!({"vip_info":{"left_duration":0,"end_time":1}}))
                .unwrap(),
            None
        );
        assert!(
            paid_deadline(
                &serde_json::json!({"vip_info":{"left_duration":1,"end_time":"invalid"}})
            )
            .is_err()
        );
    }
}
