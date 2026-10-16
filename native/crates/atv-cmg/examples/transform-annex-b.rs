//! Offline validation tool. The input is a legitimately obtained AVC Annex B
//! elementary stream. No URL, token, network or upstream component is involved.
use atv_cmg::CmgSession;
use std::{env, error::Error, fs};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if !(3..=4).contains(&args.len()) || (args.len() == 4 && args[3] != "--official-raw") {
        return Err("usage: transform-annex-b INPUT.h264 OUTPUT.h264 [--official-raw]".into());
    }
    let input = fs::read(&args[1])?;
    let mut units = Vec::new();
    let mut cursor = 0;
    let mut start = None;
    while cursor + 3 <= input.len() {
        let prefix = if input.get(cursor..cursor + 4) == Some(&[0, 0, 0, 1]) {
            4
        } else if input.get(cursor..cursor + 3) == Some(&[0, 0, 1]) {
            3
        } else {
            0
        };
        if prefix == 0 {
            cursor += 1;
            continue;
        }
        if let Some(begin) = start {
            units.push(&input[begin..cursor]);
        }
        cursor += prefix;
        start = Some(cursor);
    }
    if let Some(begin) = start {
        units.push(&input[begin..]);
    }
    if units.is_empty() {
        return Err("input has no Annex B start code".into());
    }
    let mut session = CmgSession::new("");
    let mut output = Vec::with_capacity(input.len());
    let mut count = 0;
    for nal in units {
        if nal.is_empty() {
            continue;
        }
        let transformed = if args.len() == 4 {
            session.transform_nal(nal)?
        } else {
            session.transform_annex_b_nal(nal)?
        };
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(&transformed);
        count += 1;
    }
    fs::write(&args[2], &output)?;
    println!("transformed {count} NAL units ({} bytes)", output.len());
    Ok(())
}
