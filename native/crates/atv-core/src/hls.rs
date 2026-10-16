use std::{collections::BTreeMap, fmt};
use thiserror::Error;
use url::Url;
#[derive(Debug, Error, PartialEq)]
pub enum PlaylistError {
    #[error("invalid or oversized HLS playlist")]
    Invalid,
    #[error("invalid HLS attribute list")]
    Attributes,
    #[error("invalid HLS number")]
    Number,
    #[error("invalid HLS resource URL")]
    Url,
    #[error("HLS key has an unsupported IV")]
    Iv,
}
#[derive(Clone, PartialEq)]
pub struct Key {
    pub method: String,
    pub uri: Option<Url>,
    pub iv: Option<[u8; 16]>,
    pub format: Option<String>,
}
#[derive(Clone, PartialEq)]
pub struct Segment {
    pub sequence: u64,
    pub url: Url,
    pub duration: f64,
    pub discontinuity: bool,
    pub key: Option<Key>,
    pub range: Option<(u64, u64)>,
}
#[derive(Clone, PartialEq)]
pub struct Variant {
    pub url: Url,
    pub bandwidth: u64,
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
    pub audio: Option<String>,
}
#[derive(Clone, PartialEq)]
pub struct Media {
    pub target_duration: f64,
    pub sequence: u64,
    pub segments: Vec<Segment>,
    pub end_list: bool,
    pub map: Option<Url>,
}
#[derive(Clone, PartialEq)]
pub enum Playlist {
    Master(Vec<Variant>),
    Media(Media),
}
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("method", &self.method)
            .field("uri_present", &self.uri.is_some())
            .field("iv_present", &self.iv.is_some())
            .field("format", &self.format)
            .finish()
    }
}
impl fmt::Debug for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Segment")
            .field("sequence", &self.sequence)
            .field("url", &"[redacted]")
            .field("duration", &self.duration)
            .field("discontinuity", &self.discontinuity)
            .field("key", &self.key)
            .field("range", &self.range)
            .finish()
    }
}
impl fmt::Debug for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Variant")
            .field("url", &"[redacted]")
            .field("bandwidth", &self.bandwidth)
            .field("resolution", &self.resolution)
            .field("codecs", &self.codecs)
            .field("audio", &self.audio)
            .finish()
    }
}
impl fmt::Debug for Media {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Media")
            .field("target_duration", &self.target_duration)
            .field("sequence", &self.sequence)
            .field("segment_count", &self.segments.len())
            .field("end_list", &self.end_list)
            .field("map_present", &self.map.is_some())
            .finish()
    }
}
impl fmt::Debug for Playlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Master(variants) => f
                .debug_struct("Master")
                .field("variant_count", &variants.len())
                .finish(),
            Self::Media(media) => media.fmt(f),
        }
    }
}
fn number<T: std::str::FromStr>(s: &str) -> Result<T, PlaylistError> {
    s.parse().map_err(|_| PlaylistError::Number)
}
fn address(base: &Url, value: &str) -> Result<Url, PlaylistError> {
    let u = base.join(value).map_err(|_| PlaylistError::Url)?;
    if !["http", "https"].contains(&u.scheme()) {
        return Err(PlaylistError::Url);
    }
    Ok(u)
}
fn attributes(input: &str) -> Result<BTreeMap<String, String>, PlaylistError> {
    let mut result = BTreeMap::new();
    let b = input.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] == b',' || b[i].is_ascii_whitespace()) {
            i += 1
        }
        if i == b.len() {
            break;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && b[i] != b',' {
            i += 1
        }
        if i == b.len() || b[i] != b'=' || i == start {
            return Err(PlaylistError::Attributes);
        }
        let name = input[start..i].trim().to_owned();
        i += 1;
        let value;
        if b.get(i) == Some(&b'"') {
            i += 1;
            let start = i;
            while i < b.len() && b[i] != b'"' {
                i += 1
            }
            if i == b.len() {
                return Err(PlaylistError::Attributes);
            }
            value = input[start..i].to_owned();
            i += 1;
        } else {
            let start = i;
            while i < b.len() && b[i] != b',' {
                i += 1
            }
            value = input[start..i].trim().to_owned();
        }
        if result.insert(name, value).is_some() {
            return Err(PlaylistError::Attributes);
        }
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1
        }
        if i < b.len() && b[i] != b',' {
            return Err(PlaylistError::Attributes);
        }
    }
    Ok(result)
}
fn iv(input: &str) -> Result<[u8; 16], PlaylistError> {
    let s = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
        .ok_or(PlaylistError::Iv)?;
    if s.is_empty() || s.len() > 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PlaylistError::Iv);
    }
    let n = u128::from_str_radix(s, 16).map_err(|_| PlaylistError::Iv)?;
    Ok(n.to_be_bytes())
}
pub fn parse(text: &str, base: &Url) -> Result<Playlist, PlaylistError> {
    if text.len() > 4 * 1024 * 1024 {
        return Err(PlaylistError::Invalid);
    }
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err(PlaylistError::Invalid);
    }
    let mut variants = Vec::new();
    let mut pending_variant = None;
    let mut target = 0.0;
    let mut sequence = 0u64;
    let mut segments = Vec::new();
    let mut duration = None;
    let mut discontinuity = false;
    let mut key = None;
    let mut end_list = false;
    let mut map = None;
    let mut range = None;
    let mut previous_end = 0u64;
    for line in lines {
        if let Some(s) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_variant = Some(attributes(s)?);
        } else if let Some(s) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            target = number::<f64>(s)?;
            if !target.is_finite() || target <= 0.0 {
                return Err(PlaylistError::Number);
            }
        } else if let Some(s) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if !segments.is_empty() {
                return Err(PlaylistError::Invalid);
            }
            sequence = number(s)?;
        } else if let Some(s) = line.strip_prefix("#EXTINF:") {
            let d = number::<f64>(s.split(',').next().ok_or(PlaylistError::Number)?)?;
            if !d.is_finite() || d < 0.0 {
                return Err(PlaylistError::Number);
            }
            duration = Some(d);
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" {
            end_list = true;
        } else if let Some(s) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attributes(s)?;
            map = Some(address(
                base,
                a.get("URI").ok_or(PlaylistError::Attributes)?,
            )?);
        } else if let Some(s) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attributes(s)?;
            let method = a.get("METHOD").ok_or(PlaylistError::Attributes)?.to_owned();
            key = if method == "NONE" {
                None
            } else {
                Some(Key {
                    method,
                    uri: a.get("URI").map(|v| address(base, v)).transpose()?,
                    iv: a.get("IV").map(|v| iv(v)).transpose()?,
                    format: a.get("KEYFORMAT").cloned(),
                })
            };
        } else if let Some(s) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let mut parts = s.split('@');
            let length = number::<u64>(parts.next().ok_or(PlaylistError::Number)?)?;
            let offset = parts
                .next()
                .map(number)
                .transpose()?
                .unwrap_or(previous_end);
            if parts.next().is_some() {
                return Err(PlaylistError::Number);
            }
            previous_end = offset.checked_add(length).ok_or(PlaylistError::Number)?;
            range = Some((offset, length));
        } else if !line.starts_with('#') {
            let url = address(base, line)?;
            if let Some(a) = pending_variant.take() {
                let resolution = a
                    .get("RESOLUTION")
                    .map(|r| {
                        let (w, h) = r.split_once('x').ok_or(PlaylistError::Number)?;
                        Ok((number(w)?, number(h)?))
                    })
                    .transpose()?;
                variants.push(Variant {
                    url,
                    bandwidth: number(a.get("BANDWIDTH").ok_or(PlaylistError::Attributes)?)?,
                    resolution,
                    codecs: a.get("CODECS").cloned(),
                    audio: a.get("AUDIO").cloned(),
                });
            } else {
                let seq = sequence
                    .checked_add(segments.len() as u64)
                    .ok_or(PlaylistError::Number)?;
                segments.push(Segment {
                    sequence: seq,
                    url,
                    duration: duration.take().ok_or(PlaylistError::Invalid)?,
                    discontinuity,
                    key: key.clone(),
                    range: range.take(),
                });
                discontinuity = false;
            }
        }
    }
    if pending_variant.is_some() || duration.is_some() {
        return Err(PlaylistError::Invalid);
    }
    if !variants.is_empty() {
        if !segments.is_empty() {
            return Err(PlaylistError::Invalid);
        }
        Ok(Playlist::Master(variants))
    } else if !segments.is_empty() && target > 0.0 {
        Ok(Playlist::Media(Media {
            target_duration: target,
            sequence,
            segments,
            end_list,
            map,
        }))
    } else {
        Err(PlaylistError::Invalid)
    }
}
pub fn choose_hd(variants: &[Variant]) -> Option<&Variant> {
    variants
        .iter()
        .filter(|v| v.resolution.is_none_or(|(_, h)| h <= 720))
        .max_by_key(|v| v.bandwidth)
        .or_else(|| variants.iter().min_by_key(|v| v.bandwidth))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Url {
        Url::parse("https://example.test/live/index.m3u8?opaque=secret").unwrap()
    }
    #[test]
    fn relative_live_keys_discontinuities_and_ranges() {
        let s = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:42\n#EXT-X-KEY:METHOD=AES-128,URI=\"../key\",IV=0x1\n#EXTINF:4.0,\n#EXT-X-BYTERANGE:100@20\na.ts\n#EXT-X-KEY:METHOD=NONE\n#EXT-X-DISCONTINUITY\n#EXTINF:3.5,\nb.ts\n";
        let Playlist::Media(m) = parse(s, &base()).unwrap() else {
            panic!()
        };
        assert_eq!(m.segments[0].sequence, 42);
        assert_eq!(m.segments[1].sequence, 43);
        assert_eq!(m.segments[0].range, Some((20, 100)));
        assert_eq!(m.segments[0].key.as_ref().unwrap().iv.unwrap()[15], 1);
        assert!(m.segments[1].discontinuity);
        assert!(m.segments[1].key.is_none());
        assert_eq!(m.segments[0].url.as_str(), "https://example.test/live/a.ts");
    }
    #[test]
    fn quoted_codec_comma_and_hd_selection() {
        let s = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=960x540,CODECS=\"avc1.4d401f,mp4a.40.2\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\nhigh.m3u8\n";
        let Playlist::Master(v) = parse(s, &base()).unwrap() else {
            panic!()
        };
        assert_eq!(v[0].codecs.as_deref(), Some("avc1.4d401f,mp4a.40.2"));
        assert_eq!(choose_hd(&v).unwrap().resolution, Some((960, 540)));
    }
    #[test]
    fn rejects_truncation_invalid_durations_and_uris() {
        for s in [
            "hello",
            "#EXTM3U\n#EXTINF:NaN,\nx.ts",
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1",
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nfile:///secret",
        ] {
            assert!(parse(s, &base()).is_err());
        }
    }
    #[test]
    fn every_playlist_debug_type_redacts_resource_urls() {
        let base =
            Url::parse("https://fixture.invalid/newdir/index.m3u8?token=fixture-sensitive-token")
                .unwrap();
        let Playlist::Media(media)=parse("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MAP:URI=\"init.mp4?token=fixture-sensitive-token\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key?token=fixture-sensitive-token\"\n#EXTINF:4,\na.ts?token=fixture-sensitive-token\n",&base).unwrap() else {panic!()};
        let segment = &media.segments[0];
        let key = segment.key.as_ref().unwrap();
        let Playlist::Master(variants)=parse("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000,RESOLUTION=1280x720\nchild/index.m3u8?token=fixture-sensitive-token\n",&base).unwrap() else {panic!()};
        for debug in [
            format!("{media:?}"),
            format!("{segment:?}"),
            format!("{key:?}"),
            format!("{:?}", variants[0]),
            format!("{:?}", Playlist::Media(media.clone())),
            format!("{:?}", Playlist::Master(variants)),
        ] {
            assert!(!debug.contains("fixture-sensitive-token"));
            assert!(!debug.contains("https://"));
        }
        assert_eq!(segment.url.path(), "/newdir/a.ts");
    }
}
