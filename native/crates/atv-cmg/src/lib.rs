//! CMG signalling at the AVC NAL boundary.
//!
//! CMG operates on compressed NAL units before hardware video decoding. This
//! crate implements the shared CMG Live modes 1/2 sparse TEA media transform.
//! Mode 1 was compared byte-for-byte against real authorised stream NAL units;
//! mode 2 uses the same verified component path and synthetic oracle vectors.
//! VOD remains outside the live scope and returns an explicit error.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protection {
    #[default]
    Clear,
    Live(u8),
    Vod,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyNal,
    ForbiddenBit,
    TruncatedSps,
    TruncatedVodMarker,
    TransformRequired {
        protection: Protection,
        nal_type: u8,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyNal => f.write_str("empty AVC NAL unit"),
            Self::ForbiddenBit => f.write_str("AVC forbidden_zero_bit is set"),
            Self::TruncatedSps => f.write_str("SPS lacks profile/constraint bytes"),
            Self::TruncatedVodMarker => f.write_str("CMG VOD marker lacks its flag byte"),
            Self::TransformRequired {
                protection,
                nal_type,
            } => {
                write!(
                    f,
                    "CMG {protection:?} transform required for AVC NAL type {nal_type}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// Inspect signalling without modifying the compressed media.
///
/// `nal` excludes its Annex B start code or length prefix. The two reserved
/// low bits of the SPS constraint byte signal CMG Live mode 1 or 2. Value 3
/// is deliberately not interpreted as an observed CMG Live mode.
pub fn signalling(nal: &[u8]) -> Result<Option<Protection>, Error> {
    let header = *nal.first().ok_or(Error::EmptyNal)?;
    if header & 0x80 != 0 {
        return Err(Error::ForbiddenBit);
    }
    match header & 0x1f {
        7 => {
            let constraints = *nal.get(2).ok_or(Error::TruncatedSps)?;
            Ok(match constraints & 3 {
                mode @ (1 | 2) => Some(Protection::Live(mode)),
                _ => None,
            })
        }
        25 => {
            let flag = *nal.get(1).ok_or(Error::TruncatedVodMarker)?;
            Ok(Some(if flag == 1 {
                Protection::Vod
            } else {
                Protection::Clear
            }))
        }
        _ => Ok(None),
    }
}

/// One stream's observed signalling; reset it when changing channel or codec.
#[derive(Debug, Default, Clone)]
pub struct AvcState {
    protection: Protection,
}

impl AvcState {
    pub fn protection(&self) -> Protection {
        self.protection
    }

    pub fn reset(&mut self) {
        self.protection = Protection::Clear;
    }

    pub fn observe(&mut self, nal: &[u8]) -> Result<Protection, Error> {
        if let Some(protection) = signalling(nal)? {
            self.protection = protection;
        }
        Ok(self.protection)
    }

    /// Return a clear compressed NAL unchanged, or an explicit transform error.
    /// This is intentionally incapable of presenting protected bytes as decoded.
    pub fn clear_nal<'a>(&mut self, nal: &'a [u8]) -> Result<&'a [u8], Error> {
        let protection = self.observe(nal)?;
        let nal_type = nal[0] & 0x1f;
        if protection != Protection::Clear && matches!(nal_type, 1 | 5 | 7 | 25) {
            return Err(Error::TransformRequired {
                protection,
                nal_type,
            });
        }
        Ok(nal)
    }
}

/// Media session boundary used by the native HLS/TS pipeline.
///
/// The URL is the actual service context supplied by the caller, not a fabricated
/// browser header. Callers must surface an unsupported mode rather than submit
/// it to MediaCodec. Debug output intentionally excludes the URL.
#[derive(Clone)]
pub struct CmgSession {
    initial_url: String,
    avc: AvcState,
}

impl fmt::Debug for CmgSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CmgSession")
            .field("protection", &self.avc.protection())
            .finish_non_exhaustive()
    }
}

impl CmgSession {
    pub fn new(initial_url: impl Into<String>) -> Self {
        Self {
            initial_url: initial_url.into(),
            avc: AvcState::default(),
        }
    }

    pub fn initial_url(&self) -> &str {
        &self.initial_url
    }

    pub fn protection(&self) -> Protection {
        self.avc.protection()
    }

    /// Accept exactly one complete AVC NAL, without start code/length prefix.
    /// A successful result is the official component's compressed AVC NAL,
    /// never decoded pixels. Protected slices longer than 128 bytes have their
    /// emulation prevention removed, matching the official component's output.
    /// Use `transform_annex_b_nal` before wrapping it in an Annex B start code.
    pub fn transform_nal(&mut self, nal: &[u8]) -> Result<Vec<u8>, Error> {
        let protection = self.avc.observe(nal)?;
        let nal_type = nal[0] & 0x1f;
        match (protection, nal_type) {
            (Protection::Live(1 | 2), 7) => {
                let mut output = nal.to_vec();
                // This operation is in hls.cmg.js after the WASM call, rather
                // than in the sparse cipher itself.
                output[2] = 0;
                Ok(output)
            }
            (Protection::Live(1 | 2), 1 | 5) => Ok(transform_live(nal)),
            (Protection::Clear, _) | (_, 6 | 8 | 9 | 12) => Ok(nal.to_vec()),
            (protection, _) => Err(Error::TransformRequired {
                protection,
                nal_type,
            }),
        }
    }

    /// Transform a NAL and return EBSP safe for an Annex B byte stream.
    /// The returned bytes exclude the start code. Re-escaping is applied only
    /// when CMG removed the original slice's emulation prevention bytes.
    pub fn transform_annex_b_nal(&mut self, nal: &[u8]) -> Result<Vec<u8>, Error> {
        let mut transformed = self.transform_nal(nal)?;
        if matches!(self.avc.protection(), Protection::Live(1 | 2))
            && matches!(nal[0] & 0x1f, 1 | 5)
            && nal.len() > 128
        {
            // Decrypting a block can restore an existing encoder prevention
            // sequence. Canonicalise that EBSP before inserting missing escape
            // bytes, so a valid 00 00 03 01 is not escaped a second time.
            remove_emulation_prevention_in_place(&mut transformed);
            add_emulation_prevention_in_place(&mut transformed);
        }
        Ok(transformed)
    }
}

/// The official media transform removes emulation prevention before selecting
/// its sparse block offsets. The output length therefore need not equal input.
fn remove_emulation_prevention(nal: &[u8]) -> Vec<u8> {
    let mut out = nal.to_vec();
    remove_emulation_prevention_in_place(&mut out);
    out
}
fn remove_emulation_prevention_in_place(nal: &mut Vec<u8>) {
    let mut index = 0;
    let mut written = 0;
    while index < nal.len() {
        if index + 3 < nal.len() && nal[index..index + 3] == [0, 0, 3] && nal[index + 3] <= 3 {
            nal[written..written + 2].fill(0);
            written += 2;
            index += 3;
        } else {
            nal[written] = nal[index];
            written += 1;
            index += 1;
        }
    }
    nal.truncate(written);
}

fn add_emulation_prevention_in_place(nal: &mut Vec<u8>) {
    let mut extra = 0;
    let mut zeros = 0;
    for &byte in nal.iter() {
        if zeros >= 2 && byte <= 3 {
            extra += 1;
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    if extra == 0 {
        return;
    }
    let length = nal.len();
    nal.resize(length + extra, 0);
    // Move the source into the available tail. Forward encoding closes this
    // gap without overwriting unread bytes, including arbitrarily long zero runs.
    nal.copy_within(..length, extra);
    let mut written = 0;
    zeros = 0;
    for index in extra..length + extra {
        let byte = nal[index];
        if zeros >= 2 && byte <= 3 {
            nal[written] = 3;
            written += 1;
            zeros = 0;
        }
        nal[written] = byte;
        written += 1;
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    debug_assert_eq!(written, nal.len());
}

fn transform_live(nal: &[u8]) -> Vec<u8> {
    // The threshold applies before removing emulation prevention, as verified
    // by 128/129-byte oracle inputs containing one prevention sequence.
    if nal.len() <= 128 {
        return nal.to_vec();
    }
    let mut out = remove_emulation_prevention(nal);
    let idr = out[0] & 0x1f == 5;
    let key_offset = if idr { 5 } else { 1 };
    let key = std::array::from_fn::<_, 4, _>(|index| {
        let start = key_offset + index * 4;
        u32::from_le_bytes(out[start..start + 4].try_into().unwrap())
    });
    // The stride derives from two low bits of the first key byte. IDR and
    // non-IDR units differ only in where those clear key bytes begin.
    let stride = 160 + usize::from(out[key_offset] & 3) * 32;
    // The official loop preserves a final 16-byte margin, including when the
    // next eight-byte block would otherwise fit.
    for offset in (32..out.len() - 16).step_by(stride) {
        tea_decrypt_16(&mut out[offset..offset + 8], key);
    }
    out
}

fn tea_decrypt_16(block: &mut [u8], key: [u32; 4]) {
    let mut left = u32::from_le_bytes(block[..4].try_into().unwrap());
    let mut right = u32::from_le_bytes(block[4..8].try_into().unwrap());
    let delta = 0x9e37_79b9u32;
    let mut sum = delta.wrapping_mul(16);
    for _ in 0..16 {
        right = right.wrapping_sub(
            ((left << 4).wrapping_add(key[2]))
                ^ left.wrapping_add(sum)
                ^ ((left >> 5).wrapping_add(key[3])),
        );
        left = left.wrapping_sub(
            ((right << 4).wrapping_add(key[0]))
                ^ right.wrapping_add(sum)
                ^ ((right >> 5).wrapping_add(key[1])),
        );
        sum = sum.wrapping_sub(delta);
    }
    block[..4].copy_from_slice(&left.to_le_bytes());
    block[4..8].copy_from_slice(&right.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_place_escaping_handles_expansion_and_restored_encoder_escapes() {
        for (input, expected) in [
            (vec![], vec![]),
            (vec![0, 0], vec![0, 0]),
            (vec![0, 0, 0], vec![0, 0, 3, 0]),
            (vec![0, 0, 0, 0, 0, 1], vec![0, 0, 3, 0, 0, 3, 0, 1]),
            (vec![0, 0, 3, 1], vec![0, 0, 3, 3, 1]),
            (vec![0, 0, 4, 0, 0, 2], vec![0, 0, 4, 0, 0, 3, 2]),
        ] {
            let mut output = input.clone();
            add_emulation_prevention_in_place(&mut output);
            assert_eq!(output, expected);
            remove_emulation_prevention_in_place(&mut output);
            assert_eq!(output, input);
        }
        let mut restored = vec![0, 0, 3, 1, 4, 5, 6, 7];
        remove_emulation_prevention_in_place(&mut restored);
        add_emulation_prevention_in_place(&mut restored);
        assert_eq!(restored, [0, 0, 3, 1, 4, 5, 6, 7]);
    }

    #[test]
    fn live_signalling_blocks_slices_until_channel_reset() {
        let mut state = AvcState::default();
        assert_eq!(
            state.clear_nal(&[0x67, 0x64, 1]),
            Err(Error::TransformRequired {
                protection: Protection::Live(1),
                nal_type: 7,
            })
        );
        assert!(matches!(
            state.clear_nal(&[0x65, 0xaa]),
            Err(Error::TransformRequired { .. })
        ));
        // Repeated clear-marked SPS does not undo established live protection.
        assert!(state.clear_nal(&[0x67, 0x64, 0]).is_err());
        assert_eq!(state.clear_nal(&[0x68, 0xee]).unwrap(), &[0x68, 0xee]);
        state.reset();
        assert_eq!(state.clear_nal(&[0x65, 0xaa]).unwrap(), &[0x65, 0xaa]);
    }

    #[test]
    fn modes_and_malformed_markers_are_distinct() {
        assert_eq!(signalling(&[0x67, 0x64, 2]), Ok(Some(Protection::Live(2))));
        assert_eq!(signalling(&[0x67, 0x64, 3]), Ok(None));
        assert_eq!(signalling(&[0x19, 1]), Ok(Some(Protection::Vod)));
        assert_eq!(signalling(&[0x19, 0]), Ok(Some(Protection::Clear)));
        assert_eq!(signalling(&[0x67]), Err(Error::TruncatedSps));
        assert_eq!(signalling(&[0x19]), Err(Error::TruncatedVodMarker));
        assert_eq!(signalling(&[0xe7, 0x64, 0]), Err(Error::ForbiddenBit));
    }
}
