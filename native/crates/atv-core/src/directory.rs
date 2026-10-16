use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub pid: String,
    pub stream_id: String,
    pub channel_name: String,
    pub channel_type: String,
    #[serde(default)]
    pub is_vip: bool,
    #[serde(default)]
    pub is_limited_free: bool,
    #[serde(default)]
    pub is4_k: bool,
    #[serde(default)]
    pub pay_type: Option<String>,
}
impl Channel {
    pub fn public(&self) -> bool {
        !self.is_vip || self.is_limited_free
    }
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DirectoryError {
    #[error("channel directory exceeds its size limit")]
    TooLarge,
    #[error("truncated protobuf message")]
    Truncated,
    #[error("invalid protobuf integer")]
    Integer,
    #[error("unsupported protobuf wire type")]
    WireType,
    #[error("invalid UTF-8 in directory")]
    Utf8,
    #[error("upstream directory returned failure code {0}")]
    Upstream(u64),
    #[error("channel directory contains no valid channels")]
    Empty,
}
#[derive(Clone, Copy)]
enum Value<'a> {
    Integer(u64),
    Bytes(&'a [u8]),
}
struct Field<'a> {
    number: u32,
    value: Value<'a>,
}
fn varint(bytes: &[u8], pos: &mut usize) -> Result<u64, DirectoryError> {
    let mut result = 0u64;
    for i in 0..10 {
        let b = *bytes.get(*pos).ok_or(DirectoryError::Truncated)?;
        *pos += 1;
        if i == 9 && b > 1 {
            return Err(DirectoryError::Integer);
        }
        result |= u64::from(b & 127) << (i * 7);
        if b & 128 == 0 {
            return Ok(result);
        }
    }
    Err(DirectoryError::Integer)
}
fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>, DirectoryError> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < bytes.len() {
        let tag = varint(bytes, &mut pos)?;
        if tag < 8 || tag > u64::from(u32::MAX) {
            return Err(DirectoryError::Integer);
        }
        let value = match tag & 7 {
            0 => Value::Integer(varint(bytes, &mut pos)?),
            2 => {
                let n = usize::try_from(varint(bytes, &mut pos)?)
                    .map_err(|_| DirectoryError::Integer)?;
                let end = pos.checked_add(n).ok_or(DirectoryError::Integer)?;
                let data = bytes.get(pos..end).ok_or(DirectoryError::Truncated)?;
                pos = end;
                Value::Bytes(data)
            }
            1 | 5 => {
                let n = if tag & 7 == 1 { 8 } else { 4 };
                pos = pos.checked_add(n).ok_or(DirectoryError::Integer)?;
                if pos > bytes.len() {
                    return Err(DirectoryError::Truncated);
                }
                continue;
            }
            _ => return Err(DirectoryError::WireType),
        };
        result.push(Field {
            number: (tag >> 3) as u32,
            value,
        });
        if result.len() > 100_000 {
            return Err(DirectoryError::TooLarge);
        }
    }
    Ok(result)
}
fn text(fields: &[Field<'_>], number: u32) -> Result<String, DirectoryError> {
    match fields.iter().find(|f| f.number == number).map(|f| f.value) {
        Some(Value::Bytes(b)) => Ok(std::str::from_utf8(b)
            .map_err(|_| DirectoryError::Utf8)?
            .to_owned()),
        _ => Ok(String::new()),
    }
}
fn int(fields: &[Field<'_>], number: u32) -> u64 {
    fields
        .iter()
        .find_map(|f| {
            if f.number == number {
                if let Value::Integer(n) = f.value {
                    Some(n)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or(0)
}
fn messages<'a>(fields: &[Field<'a>], number: u32) -> Vec<&'a [u8]> {
    fields
        .iter()
        .filter_map(|f| {
            if f.number == number {
                if let Value::Bytes(b) = f.value {
                    Some(b)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect()
}
pub fn decode(bytes: &[u8]) -> Result<Vec<Channel>, DirectoryError> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(DirectoryError::TooLarge);
    }
    let root = fields(bytes)?;
    let code = int(&root, 1);
    if ![0, 200].contains(&code) {
        return Err(DirectoryError::Upstream(code));
    }
    let data = messages(&root, 2);
    let data = data.first().ok_or(DirectoryError::Empty)?;
    let mut channels = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for module in messages(&fields(data)?, 2) {
        for raw in messages(&fields(module)?, 15) {
            let f = fields(raw)?;
            let channel = Channel {
                pid: text(&f, 4)?,
                stream_id: text(&f, 6)?,
                channel_name: text(&f, 2)?,
                channel_type: text(&f, 11)?,
                is_vip: int(&f, 7) == 1,
                is4_k: int(&f, 8) == 1,
                is_limited_free: int(&f, 14) == 1,
                pay_type: Some(text(&f, 18)?),
            };
            let id_ok =
                |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
            if id_ok(&channel.pid)
                && id_ok(&channel.stream_id)
                && !channel.channel_name.is_empty()
                && ["yangshi", "weishi"].contains(&channel.channel_type.as_str())
                && seen.insert(channel.pid.clone())
            {
                channels.push(channel)
            }
        }
    }
    if channels.is_empty() {
        Err(DirectoryError::Empty)
    } else {
        Ok(channels)
    }
}
pub fn bundled() -> Result<Vec<Channel>, serde_json::Error> {
    serde_json::from_str(include_str!("../assets/channels.json"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_directory_has_real_ids_and_rights() {
        let c = bundled().unwrap();
        assert_eq!(c.len(), 60);
        assert_eq!(
            c.iter().find(|x| x.channel_name == "CCTV1").unwrap().pid,
            "600001859"
        );
        assert!(c.iter().all(|x| !x.is_vip));
    }
    #[test]
    fn rejects_truncation_and_integer_overflow() {
        for b in [
            &[18, 10, 1][..],
            &[0][..],
            &[8, 128][..],
            &[8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2][..],
        ] {
            assert!(decode(b).is_err());
        }
    }
}
