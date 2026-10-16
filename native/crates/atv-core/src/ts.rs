//! Bounded MPEG-TS PSI/PES reassembly. Codec transforms operate on the resulting elementary bytes.
use std::collections::HashMap;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Avc,
    Hevc,
    Aac,
    MpegAudio,
}
#[derive(Debug, Clone)]
pub struct ElementaryPacket {
    pub codec: Codec,
    pub pts_90khz: u64,
    pub data: Vec<u8>,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TsError {
    #[error("invalid MPEG-TS packet alignment")]
    Alignment,
    #[error("malformed MPEG-TS packet")]
    Packet,
    #[error("scrambled transport packets require an authorized transform")]
    Scrambled,
    #[error("malformed transport stream section")]
    Section,
    #[error("malformed elementary packet")]
    Pes,
    #[error("elementary packet exceeded its size limit")]
    TooLarge,
}
#[derive(Default)]
struct Sections {
    data: Vec<u8>,
}
impl Sections {
    fn push(&mut self, payload: &[u8], start: bool) -> Result<Vec<Vec<u8>>, TsError> {
        let mut output = Vec::new();
        let mut bytes = payload;
        if start {
            let (&pointer, rest) = payload.split_first().ok_or(TsError::Section)?;
            let n = usize::from(pointer);
            if n > rest.len() {
                return Err(TsError::Section);
            }
            if !self.data.is_empty() {
                self.data.extend_from_slice(&rest[..n]);
                self.extract(&mut output)?;
            }
            self.data.clear();
            bytes = &rest[n..];
        }
        if !start && self.data.is_empty() {
            return Ok(output);
        }
        self.data.extend_from_slice(bytes);
        self.extract(&mut output)?;
        Ok(output)
    }
    fn extract(&mut self, out: &mut Vec<Vec<u8>>) -> Result<(), TsError> {
        while self.data.len() >= 3 {
            if self.data[0] == 0xff {
                self.data.clear();
                break;
            }
            let n = 3 + ((usize::from(self.data[1] & 15) << 8) | usize::from(self.data[2]));
            if !(8..=4096).contains(&n) {
                self.data.clear();
                return Err(TsError::Section);
            }
            if self.data.len() < n {
                break;
            }
            let section: Vec<u8> = self.data.drain(..n).collect();
            if crc32(&section) != 0 {
                return Err(TsError::Section);
            }
            out.push(section);
        }
        if self.data.len() > 4096 {
            self.data.clear();
            return Err(TsError::TooLarge);
        }
        Ok(())
    }
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff;
    for &b in bytes {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            }
        }
    }
    crc
}
#[derive(Default)]
struct Pes {
    data: Vec<u8>,
    continuity: Option<u8>,
}
impl Pes {
    fn finish(&mut self, codec: Codec) -> Result<Option<ElementaryPacket>, TsError> {
        if self.data.is_empty() {
            return Ok(None);
        }
        let mut data = std::mem::take(&mut self.data);
        if data.len() < 9 || data[..3] != [0, 0, 1] {
            return Err(TsError::Pes);
        }
        let offset = 9 + usize::from(data[8]);
        if offset > data.len() {
            return Err(TsError::Pes);
        }
        let length = usize::from(u16::from_be_bytes([data[4], data[5]]));
        let end = if length == 0 { data.len() } else { 6 + length };
        if end > data.len() || offset > end {
            return Err(TsError::Pes);
        }
        let pts = if data[7] & 0x80 != 0 {
            if data[8] < 5 {
                return Err(TsError::Pes);
            }
            decode_pts(&data[9..14])?
        } else {
            return Ok(None);
        };
        if offset == end {
            return Ok(None);
        }
        // Keep the reassembly allocation instead of allocating a second PES
        // payload. Headers/stuffing are removed only after all validation.
        data.copy_within(offset..end, 0);
        data.truncate(end - offset);
        Ok(Some(ElementaryPacket {
            codec,
            pts_90khz: pts,
            data,
        }))
    }
}
fn decode_pts(p: &[u8]) -> Result<u64, TsError> {
    if p.len() < 5 || p[0] & 1 == 0 || p[2] & 1 == 0 || p[4] & 1 == 0 {
        return Err(TsError::Pes);
    }
    Ok((u64::from((p[0] >> 1) & 7) << 30)
        | (u64::from(p[1]) << 22)
        | (u64::from(p[2] >> 1) << 15)
        | (u64::from(p[3]) << 7)
        | u64::from(p[4] >> 1))
}
#[derive(Default)]
pub struct Demux {
    pmt_pid: Option<u16>,
    pat: Sections,
    pmt: Sections,
    tracks: HashMap<u16, Codec>,
    pes: HashMap<u16, Pes>,
    pub dropped_packets: u64,
}
impl Demux {
    /// Heap capacity of retained PSI/PES payload buffers, excluding map metadata.
    pub fn buffered_capacity(&self) -> usize {
        self.pat.data.capacity()
            + self.pmt.data.capacity()
            + self
                .pes
                .values()
                .map(|pes| pes.data.capacity())
                .sum::<usize>()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn push_fragment(&mut self, bytes: &[u8]) -> Result<Vec<ElementaryPacket>, TsError> {
        let alignment = fragment_alignment(bytes)?;
        self.push_aligned(&bytes[alignment..])
    }
    fn push_aligned(&mut self, bytes: &[u8]) -> Result<Vec<ElementaryPacket>, TsError> {
        let mut output = Vec::new();
        for packet in bytes.as_chunks::<188>().0 {
            if packet[0] != 0x47 {
                return Err(TsError::Alignment);
            }
            let pid = (u16::from(packet[1] & 31) << 8) | u16::from(packet[2]);
            let start = packet[1] & 0x40 != 0;
            let flags = packet[3];
            if packet[1] & 0x80 != 0 {
                self.pes.remove(&pid);
                self.dropped_packets += 1;
                continue;
            }
            if flags & 0xc0 != 0 {
                return Err(TsError::Scrambled);
            }
            let control = (flags >> 4) & 3;
            if control == 0 {
                return Err(TsError::Packet);
            }
            let mut offset = 4;
            let mut discontinuity = false;
            if control & 2 != 0 {
                let n = usize::from(packet[4]);
                if n > 183 {
                    return Err(TsError::Packet);
                }
                if n > 0 {
                    discontinuity = packet[5] & 0x80 != 0
                }
                offset = 5 + n;
            }
            if discontinuity {
                if let Some(pes) = self.pes.get_mut(&pid) {
                    pes.data.clear();
                    pes.continuity = None;
                }
                if pid == 0 {
                    self.pat = Sections::default();
                }
                if Some(pid) == self.pmt_pid {
                    self.pmt = Sections::default();
                }
            }
            if control & 1 == 0 || offset == 188 {
                continue;
            }
            if offset > 188 {
                return Err(TsError::Packet);
            }
            let payload = &packet[offset..];
            if pid == 0 {
                for section in self.pat.push(payload, start)? {
                    self.read_pat(&section)?;
                }
                continue;
            }
            if Some(pid) == self.pmt_pid {
                for section in self.pmt.push(payload, start)? {
                    self.read_pmt(&section)?;
                }
                continue;
            }
            let Some(&codec) = self.tracks.get(&pid) else {
                continue;
            };
            let continuity = flags & 15;
            let pes = self.pes.entry(pid).or_default();
            if discontinuity {
                pes.data.clear();
                pes.continuity = None;
            }
            if let Some(previous) = pes.continuity {
                if continuity == previous {
                    continue;
                }
                if continuity != (previous + 1) & 15 {
                    pes.data.clear();
                    self.dropped_packets += 1;
                }
            }
            pes.continuity = Some(continuity);
            if start {
                if let Some(value) = pes.finish(codec)? {
                    output.push(value)
                }
            } else if pes.data.is_empty() {
                continue;
            }
            if pes.data.len().saturating_add(payload.len()) > 8 * 1024 * 1024 {
                return Err(TsError::TooLarge);
            }
            pes.data.extend_from_slice(payload);
            if pes.data.len() >= 6 {
                let n = usize::from(u16::from_be_bytes([pes.data[4], pes.data[5]]));
                if n != 0
                    && pes.data.len() >= n + 6
                    && let Some(value) = pes.finish(codec)?
                {
                    output.push(value)
                }
            }
        }
        // A zero-length video PES ends at the next payload_unit_start, not at an
        // HTTP/HLS boundary. Retain it across fragments to avoid partial NALs.
        Ok(output)
    }
    fn read_pat(&mut self, s: &[u8]) -> Result<(), TsError> {
        if s.len() < 12 || s[0] != 0 {
            return Err(TsError::Section);
        }
        for entry in s[8..s.len() - 4].as_chunks::<4>().0 {
            if entry[0] != 0 || entry[1] != 0 {
                let pid = (u16::from(entry[2] & 31) << 8) | u16::from(entry[3]);
                if self.pmt_pid != Some(pid) {
                    self.pmt = Sections::default();
                    self.tracks.clear();
                    self.pes.clear();
                }
                self.pmt_pid = Some(pid);
                break;
            }
        }
        Ok(())
    }
    fn read_pmt(&mut self, s: &[u8]) -> Result<(), TsError> {
        if s.len() < 16 || s[0] != 2 {
            return Err(TsError::Section);
        }
        let mut i = 12 + ((usize::from(s[10] & 15) << 8) | usize::from(s[11]));
        let end = s.len() - 4;
        if i > end {
            return Err(TsError::Section);
        }
        let mut tracks = HashMap::new();
        while i < end {
            if i + 5 > end {
                return Err(TsError::Section);
            }
            let kind = s[i];
            let pid = (u16::from(s[i + 1] & 31) << 8) | u16::from(s[i + 2]);
            let length = (usize::from(s[i + 3] & 15) << 8) | usize::from(s[i + 4]);
            let codec = match kind {
                0x1b => Some(Codec::Avc),
                0x24 => Some(Codec::Hevc),
                0x0f => Some(Codec::Aac),
                0x03 | 0x04 => Some(Codec::MpegAudio),
                _ => None,
            };
            if let Some(c) = codec {
                tracks.insert(pid, c);
            }
            i += 5 + length;
            if i > end {
                return Err(TsError::Section);
            }
        }
        self.pes
            .retain(|pid, _| self.tracks.get(pid) == tracks.get(pid) && tracks.contains_key(pid));
        self.tracks = tracks;
        Ok(())
    }
}
/// Validate a complete HTTP fragment once before consuming transport-aligned
/// batches. A leading prefix shorter than one transport packet is permitted.
pub fn fragment_alignment(bytes: &[u8]) -> Result<usize, TsError> {
    let alignment = (0..188.min(bytes.len()))
        .find(|&i| {
            bytes.len() >= i + 188
                && bytes.get(i) == Some(&0x47)
                && (i + 188 >= bytes.len() || bytes.get(i + 188) == Some(&0x47))
                && (i + 376 >= bytes.len() || bytes.get(i + 376) == Some(&0x47))
        })
        .ok_or(TsError::Alignment)?;
    if !(bytes.len() - alignment).is_multiple_of(188) {
        return Err(TsError::Alignment);
    }
    Ok(alignment)
}
pub fn annex_b_nals(bytes: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(&[0, 0, 0, 1]) {
            starts.push((i, i + 4));
            i += 4;
        } else if bytes[i..].starts_with(&[0, 0, 1]) {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(index, &(_, start))| {
            let end = starts
                .get(index + 1)
                .map_or(bytes.len(), |(offset, _)| *offset);
            (start < end).then_some(&bytes[start..end])
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finishing_pes_reuses_allocation_and_removes_headers_and_stuffing() {
        let mut data = zero_length_pes(&[0x65, 0xaa, 0xbb]);
        let length = (data.len() - 6) as u16;
        data[4..6].copy_from_slice(&length.to_be_bytes());
        data.extend_from_slice(&[0xff; 20]);
        let pointer = data.as_ptr();
        let mut pes = Pes {
            data,
            continuity: Some(0),
        };
        let output = pes.finish(Codec::Avc).unwrap().unwrap();
        assert_eq!(output.data, [0x65, 0xaa, 0xbb]);
        assert_eq!(output.data.as_ptr(), pointer);
        assert!(pes.data.is_empty());
    }
    fn packet(payload: &[u8], start: bool, counter: u8) -> [u8; 188] {
        assert!(payload.len() <= 182);
        let mut ts = [0xff; 188];
        ts[..4].copy_from_slice(&[0x47, if start { 0x41 } else { 1 }, 0, 0x30 | counter]);
        let adaptation = 183 - payload.len();
        ts[4] = adaptation as u8;
        ts[5] = 0;
        ts[5 + adaptation..].copy_from_slice(payload);
        ts
    }
    fn zero_length_pes(data: &[u8]) -> Vec<u8> {
        let mut p = vec![0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
        p.extend_from_slice(data);
        p
    }
    #[test]
    fn zero_length_pes_waits_for_next_start_across_http_fragments() {
        let mut demux = Demux::default();
        demux.tracks.insert(256, Codec::Avc);
        let first = [0x11; 100];
        let tail = [0x22; 50];
        assert!(
            demux
                .push_fragment(&packet(&zero_length_pes(&first), true, 0))
                .unwrap()
                .is_empty()
        );
        assert!(
            demux
                .push_fragment(&packet(&tail, false, 1))
                .unwrap()
                .is_empty()
        );
        let output = demux
            .push_fragment(&packet(&zero_length_pes(&[0x33; 10]), true, 2))
            .unwrap();
        assert_eq!(output.len(), 1);
        let expected: Vec<u8> = first.into_iter().chain(tail).collect();
        assert_eq!(output[0].data, expected);
    }
    #[test]
    fn nal_boundaries_with_both_prefix_widths() {
        let bytes = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4];
        assert_eq!(
            annex_b_nals(&bytes),
            vec![&bytes[4..7], &bytes[10..12], &bytes[15..17]]
        );
    }
    #[test]
    fn pts_all_33_bits() {
        let n = (1u64 << 33) - 1;
        let p = [
            0x20 | (((n >> 30) as u8 & 7) << 1) | 1,
            (n >> 22) as u8,
            (((n >> 15) as u8 & 127) << 1) | 1,
            (n >> 7) as u8,
            ((n as u8 & 127) << 1) | 1,
        ];
        assert_eq!(decode_pts(&p).unwrap(), n);
    }
    #[test]
    fn reject_non_transport_input() {
        assert_eq!(
            Demux::default().push_fragment(&[0; 564]).unwrap_err(),
            TsError::Alignment
        );
    }
    #[test]
    fn pat_crc_and_section_split() {
        let mut pat = vec![0, 0xb0, 13, 0, 1, 0xc1, 0, 0, 0, 1, 0xe1, 0];
        let crc = crc32(&pat);
        pat.extend_from_slice(&crc.to_be_bytes());
        assert_eq!(crc32(&pat), 0);
        let mut section = Sections::default();
        let mut first = vec![0];
        first.extend_from_slice(&pat[..7]);
        assert!(section.push(&first, true).unwrap().is_empty());
        assert_eq!(section.push(&pat[7..], false).unwrap(), vec![pat]);
    }
}
