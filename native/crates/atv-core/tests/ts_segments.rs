//! Opt-in regressions against normally acquired local media. No sample media,
//! signed resources, credentials or media bytes are stored in these tests.
use atv_core::ts::{Codec, Demux, ElementaryPacket};
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
};
fn sample(index: u8) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    std::fs::read(root.join(format!("research/native-cmg/authorized-sample-{index}.ts")))
        .expect("obtain the authorized local sample first")
}
fn summary(packets: &[ElementaryPacket]) -> (usize, usize, usize, u64) {
    let mut video = 0;
    let mut audio = 0;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for packet in packets {
        let kind = match packet.codec {
            Codec::Avc => {
                video += 1;
                1
            }
            Codec::Aac => {
                audio += 1;
                2
            }
            Codec::Hevc => 3,
            Codec::MpegAudio => 4,
        };
        kind.hash(&mut hash);
        packet.pts_90khz.hash(&mut hash);
        packet.data.hash(&mut hash)
    }
    (packets.len(), video, audio, hash.finish())
}
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn actual_pes_survives_a_transport_aligned_fragment_boundary() {
    for index in [1, 2] {
        let bytes = sample(index);
        let expected = Demux::default().push_fragment(&bytes).unwrap();
        // Packet 2 begins the first large video PES; packet 10 is genuinely
        // inside that PES in both authorized samples, before its next PUSI.
        let boundary = 188 * 10;
        let mut demux = Demux::default();
        let mut output = demux.push_fragment(&bytes[..boundary]).unwrap();
        output.extend(demux.push_fragment(&bytes[boundary..]).unwrap());
        assert_eq!(
            summary(&output),
            summary(&expected),
            "sample {index} lost or truncated a cross-fragment PES"
        );
    }
}
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn actual_adjacent_live_segments_have_consistent_demux_results() {
    let one = sample(1);
    let two = sample(2);
    let mut whole = one.clone();
    whole.extend_from_slice(&two);
    let expected = Demux::default().push_fragment(&whole).unwrap();
    let mut demux = Demux::default();
    let mut actual = demux.push_fragment(&one).unwrap();
    actual.extend(demux.push_fragment(&two).unwrap());
    assert_eq!(summary(&actual), summary(&expected));
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
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
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn pmt_track_removal_does_not_panic_with_pending_pes() {
    let mut bytes = sample(1);
    let mut pmt = bytes[188..376].to_vec();
    assert_eq!(((u16::from(pmt[1] & 31)) << 8) | u16::from(pmt[2]), 4097);
    let payload = if pmt[3] & 0x20 != 0 {
        5 + usize::from(pmt[4])
    } else {
        4
    };
    let section_start = payload + 1 + usize::from(pmt[payload]);
    let section_len =
        3 + ((usize::from(pmt[section_start + 1] & 15) << 8) | usize::from(pmt[section_start + 2]));
    let section = &mut pmt[section_start..section_start + section_len];
    let end = section.len() - 4;
    let mut offset = 12 + ((usize::from(section[10] & 15) << 8) | usize::from(section[11]));
    let mut changed = false;
    while offset < end {
        let descriptors =
            (usize::from(section[offset + 3] & 15) << 8) | usize::from(section[offset + 4]);
        if section[offset] == 0x1b {
            section[offset] = 0x06;
            changed = true;
        }
        offset += 5 + descriptors;
    }
    assert!(changed);
    let crc = crc32(&section[..end]);
    section[end..].copy_from_slice(&crc.to_be_bytes());
    assert_eq!(crc32(section), 0);
    bytes.truncate(188 * 10);
    bytes.extend_from_slice(&pmt);
    let result = std::panic::catch_unwind(|| Demux::default().push_fragment(&bytes));
    assert!(
        result.is_ok(),
        "PMT track removal panicked while video PES was pending"
    );
}
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn actual_duplicate_transport_packet_is_ignored() {
    let bytes = sample(1);
    let expected = Demux::default().push_fragment(&bytes).unwrap();
    let boundary = 188 * 10;
    let mut duplicated = bytes[..boundary].to_vec();
    duplicated.extend_from_slice(&bytes[boundary - 188..boundary]);
    duplicated.extend_from_slice(&bytes[boundary..]);
    let mut demux = Demux::default();
    let actual = demux.push_fragment(&duplicated).unwrap();
    assert_eq!(summary(&actual), summary(&expected));
    assert_eq!(demux.dropped_packets, 0);
}
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn actual_packet_loss_discards_only_the_damaged_pes() {
    let bytes = sample(1);
    let mut expected = Demux::default().push_fragment(&bytes).unwrap();
    let first_video = expected.iter().position(|p| p.codec == Codec::Avc).unwrap();
    expected.remove(first_video);
    let boundary = 188 * 10;
    let mut lost = bytes[..boundary].to_vec();
    lost.extend_from_slice(&bytes[boundary + 188..]);
    let mut demux = Demux::default();
    let actual = demux.push_fragment(&lost).unwrap();
    assert_eq!(summary(&actual), summary(&expected));
    assert_eq!(demux.dropped_packets, 1);
}
#[test]
#[ignore = "requires ignored, normally authorized local TS samples"]
fn adaptation_only_discontinuity_discards_pending_pes() {
    let bytes = sample(1);
    let mut expected = Demux::default().push_fragment(&bytes).unwrap();
    let first_video = expected.iter().position(|p| p.codec == Codec::Avc).unwrap();
    expected.remove(first_video);
    let mut adaptation = [0xff; 188];
    adaptation[..6].copy_from_slice(&[0x47, 0x01, 0x00, 0x20, 183, 0x80]);
    let boundary = 188 * 10;
    let mut discontinuous = bytes[..boundary].to_vec();
    discontinuous.extend_from_slice(&adaptation);
    discontinuous.extend_from_slice(&bytes[boundary..]);
    let mut demux = Demux::default();
    let actual = demux.push_fragment(&discontinuous).unwrap();
    assert_eq!(summary(&actual), summary(&expected));
}
