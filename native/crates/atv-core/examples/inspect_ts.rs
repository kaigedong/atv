//! Offline diagnostic: inspect a user-owned, normally authorized TS sample.
//! Deliberately reports only media counts and timestamps, never resource URLs.
use atv_core::ts::{Codec, Demux, annex_b_nals};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("supply a TS file")?;
    let data = std::fs::read(path)?;
    let mut demux = Demux::default();
    let packets = demux.push_fragment(&data)?;
    let mut nal_types = std::collections::BTreeMap::new();
    let mut video = 0;
    let mut audio = 0;
    for packet in &packets {
        match packet.codec {
            Codec::Avc => {
                video += 1;
                for nal in annex_b_nals(&packet.data) {
                    *nal_types.entry(nal[0] & 31).or_insert(0) += 1;
                }
            }
            Codec::Aac => audio += 1,
            _ => {}
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "bytes": data.len(), "packets": packets.len(), "videoPackets": video,
            "audioPackets": audio, "nalTypes": nal_types, "droppedPackets": demux.dropped_packets,
            "firstPts": packets.first().map(|p|p.pts_90khz),
            "lastPts": packets.last().map(|p|p.pts_90khz)
        })
    );
    Ok(())
}
