//! Native implementations of the observed public player request algorithms.
//! This crate does not grant viewing permissions or turn a rejected response into a stream.
use aes::cipher::{BlockEncryptMut, KeyIvInit, StreamCipher, block_padding::Pkcs7};
use md5::{Digest, Md5};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt};

mod resolver;
pub use resolver::{
    LiveInfo, LiveResolver, ResolveError, ResolverConfig, generate_device_guid, parse_live_info,
};

pub const PLAYER_APP_ID: &str = "519748109";
pub const PLAYER_VERSION: &str = "V1.0.0";
pub const PLAYER_PLATFORM: &str = "5910204";
pub const OPEN_TOKEN_URL: &str = "https://h5access.yangshipin.cn/web/open/token";
pub const PLAYER_AUTH_URL: &str = "https://player-api.yangshipin.cn/v1/player/auth";
pub const LIVE_INFO_URL: &str = "https://player-api.yangshipin.cn/v1/player/get_live_info";
const LIVE_SALT: &str = "0f$IVHi9Qno?G";
const AUTH_SALT: &str = "n@7QKk%YeSjfw%22";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError(pub String);
impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ProtocolError {}

pub fn md5_hex(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Md5::digest(bytes.as_ref()))
}

/// ECMAScript decodeURI preserves escaped URI syntax, unlike decodeURIComponent.
pub fn decode_uri(value: &str) -> Result<String, ProtocolError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 2 >= bytes.len() {
            return Err(ProtocolError("Malformed URI escape".into()));
        }
        let h = |b: u8| (b as char).to_digit(16).map(|x| x as u8);
        let decoded = h(bytes[i + 1])
            .zip(h(bytes[i + 2]))
            .map(|(a, b)| a * 16 + b)
            .ok_or_else(|| ProtocolError("Malformed URI escape".into()))?;
        if b";/?:@&=+$,#".contains(&decoded) {
            out.extend_from_slice(&bytes[i..i + 3]);
        } else {
            out.push(decoded);
        }
        i += 3;
    }
    String::from_utf8(out).map_err(|_| ProtocolError("Malformed URI UTF-8".into()))
}
fn js_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_value(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        _ => v.to_string(),
    }
}
/// Sort keys exactly as Array.sort() for the ASCII parameter names in this protocol.
pub fn canonical_parameters(params: &Map<String, Value>) -> Result<String, ProtocolError> {
    let sorted: BTreeMap<_, _> = params.iter().collect();
    sorted
        .into_iter()
        .map(|(k, v)| Ok(format!("{k}={}", decode_uri(&js_value(v))?)))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| v.join("&"))
}
pub fn live_signature(params: &Map<String, Value>) -> Result<String, ProtocolError> {
    Ok(md5_hex(format!(
        "{}{LIVE_SALT}",
        canonical_parameters(params)?
    )))
}
pub fn player_auth_signature(params: &Map<String, Value>) -> Result<String, ProtocolError> {
    Ok(md5_hex(format!(
        "{}{AUTH_SALT}",
        canonical_parameters(params)?
    )))
}
pub fn sdk_input(params: &Map<String, Value>) -> Result<String, ProtocolError> {
    // The SDK uses localeCompare rather than Array.sort. On the protocol's
    // fixed ASCII names, punctuation then case-folded order reproduces it.
    let mut names: Vec<_> = params.keys().collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    let joined = names
        .into_iter()
        .map(|name| Ok(format!("{name}={}", decode_uri(&js_value(&params[name]))?)))
        .collect::<Result<Vec<_>, ProtocolError>>()?
        .join("&");
    Ok(md5_hex(joined))
}
#[derive(Clone, Debug)]
pub struct PageMetadata<'a> {
    pub url: &'a str,
    pub user_agent: &'a str,
    pub referrer: &'a str,
    pub app_code_name: &'a str,
    pub app_name: &'a str,
    pub platform: &'a str,
}
fn truncate_js(s: &str, n: usize) -> String {
    String::from_utf16_lossy(&s.encode_utf16().take(n).collect::<Vec<_>>())
}
/// cKey hashes UTF-16 code units with signed, wrapping hash31, then encrypts UTF-8.
pub fn ckey(
    cnlid: &str,
    timestamp_seconds: u64,
    version: &str,
    guid: &str,
    platform: &str,
    page: &PageMetadata<'_>,
) -> String {
    let trim = |s: &str| truncate_js(s, 24);
    let base = format!(
        "|{cnlid}|{timestamp_seconds}|mg3c3b04ba|{version}|{guid}|{platform}|{}|{}|{}|{}|{}|{}|",
        trim(page.url),
        trim(&page.user_agent.to_lowercase()),
        trim(page.referrer),
        page.app_code_name,
        page.app_name,
        page.platform
    );
    let hash = base
        .encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32));
    let plaintext = format!("|{hash}{base}");
    let key = hex::decode("48e5918a74ae21c972b90cce8af6c8be").unwrap();
    let iv = hex::decode("9a7e7d23610266b1d9fbf98581384d92").unwrap();
    let encrypted = cbc::Encryptor::<aes::Aes128>::new_from_slices(&key, &iv)
        .unwrap()
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
    format!("--01{}", hex::encode_upper(encrypted))
}
/// OpenAPI keygen_bg.wasm get_token_rnd, v1 path observed in production.
pub fn token_rnd(appid: &str, version: &str, guid: &str, ts_millis: u64) -> String {
    md5_hex(format!("token_rnd;{appid};{version};{guid};{ts_millis};"))
}
/// Returns the complete yspsdksign value, including its input suffix.
pub fn openapi_signature(
    guid: &str,
    host: &str,
    token: &str,
    protocol: &str,
    appid: &str,
    input: &str,
) -> String {
    format!(
        "{}-{input}",
        md5_hex(format!("{guid};{host};{token};{protocol};{appid};{input};"))
    )
}
#[derive(Default, Debug)]
pub struct RequestSequence {
    value: u32,
}
impl RequestSequence {
    pub fn new(value: u32) -> Self {
        Self { value }
    }
    pub fn next(&mut self, random: &str, ts_millis: u64) -> Result<(u32, String), ProtocolError> {
        if random.len() != 10 || !random.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ProtocolError(
                "request random must contain 10 ASCII alphanumeric characters".into(),
            ));
        }
        self.value = if self.value < 2147483646 {
            self.value + 1
        } else {
            1
        };
        Ok((self.value, format!("999999{random}{ts_millis}")))
    }
}
/// SDK signs the original request before rand_str/signature are added.
pub fn sdk_signature_input(
    params: &Map<String, Value>,
    guid: &str,
    sequence: u32,
    request_id: &str,
) -> Result<String, ProtocolError> {
    Ok(format!(
        "{}-{guid}-{sequence}-{request_id}",
        sdk_input(params)?
    ))
}

/// Ticket nonce follows the shipped musl rand() implementation, seeded from
/// the client clock independently of the server's `ts` authorization field.
pub fn ticket_nonce(client_time_seconds: u64) -> String {
    let alphabet = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut state = (client_time_seconds as u32).wrapping_sub(1) as u64;
    (0..10)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            alphabet[((state >> 33) as usize) % 62] as char
        })
        .collect()
}
/// Replaces RJq7sO71JF.wasm `_aes_encrypt_ctr`. Channel ID and application
/// version are absent from its actual cipher input, despite being JS arguments.
pub fn player_ticket(
    pid: &str,
    server_ts: &str,
    guid: &str,
    appid: &str,
    client_time_seconds: u64,
) -> String {
    let text = format!(
        "{}&{}&{}&{}&{}",
        pid.trim(),
        server_ts.trim(),
        guid.trim(),
        appid.trim(),
        ticket_nonce(client_time_seconds)
    );
    let mut bytes = text.into_bytes();
    let mut cipher =
        ctr::Ctr128BE::<aes::Aes128>::new(b"L@cTjfZHDDk3QVZp".into(), b"jbbmj3zgTN2ys@p2".into());
    cipher.apply_keystream(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticket_wasm_fixture() {
        assert_eq!(ticket_nonce(1700000000), "Oi7EIAC1oh");
        assert_eq!(
            player_ticket(
                "fixture-pid",
                "1700000000",
                "fixture-guid",
                "519748109",
                1700000000
            ),
            "0c1991d8750ea9153c41ccabcc1521eefb56797bfab39b0711802347c08fe1f2e18d818d473ffa930abf82f712a05646c79aa3ceebc518fc"
        );
    }
    #[test]
    fn keygen_wasm_fixtures() {
        assert_eq!(
            token_rnd("FIXTURE-APP", "v1", "FIXTURE-GUID", 1700000000000),
            "ba97037f92736ada2d36a4e505efa302"
        );
        assert_eq!(
            openapi_signature(
                "FIXTURE-GUID",
                "www.yangshipin.cn",
                "FIXTURE-TOKEN",
                "https:",
                "FIXTURE-APP",
                "FIXTURE-INPUT"
            ),
            "08b4447e9b10729fdf3c06e9cc8012da-FIXTURE-INPUT"
        );
    }
    #[test]
    fn decode_uri_reserved() {
        assert_eq!(
            decode_uri("%E7%9B%B4%E6%92%AD%2F%2f%3F%23%20").unwrap(),
            "直播%2F%2f%3F%23 "
        );
        assert!(decode_uri("%C0%AF").is_err());
    }
    #[test]
    fn rollover() {
        let mut s = RequestSequence::new(2147483646);
        assert_eq!(
            s.next("ABCDEFGHIJ", 1700000000000).unwrap(),
            (1, "999999ABCDEFGHIJ1700000000000".into())
        );
    }
    #[test]
    fn ckey_vendor_fixture() {
        let page = PageMetadata {
            url: "https://www.yangshipin.cn/tv/home",
            user_agent: "Fixture UA",
            referrer: "https://www.yangshipin.cn/",
            app_code_name: "Mozilla",
            app_name: "Netscape",
            platform: "Linux x86_64",
        };
        assert_eq!(
            ckey(
                "fixture-cnlid",
                1700000000,
                "V1.0.0",
                "fixture-guid",
                "5910204",
                &page
            ),
            "--01F510FDD6FE52A189627BDD98D4F9E5D2294BC7C34C5C08568EE1E5A00150710CEA9445CB6D7305512DF16479384B87C68CEEEE8810141DC736F95A79ADB92BA6C9AF18E7EE258F1EC951576053B84496FCB045669A46CB2F6D2CCB611E958F3AFF4567CFC63B67E7E7CECC0D167C7EE6A000728C0B0E16938A30CD3ED5C8AB4E575882801D0B61AB7A3E7523A7A73C3D244B6AD8B9263E216AD100975266019B5F74A84FD8B74C7D453ECB150DFEA8EC"
        );
    }
}
