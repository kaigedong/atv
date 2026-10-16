//! Normal anonymous public playback flow. Server authorization remains authoritative.
use crate::*;
use rand::Rng;
use reqwest::{
    blocking::Client,
    header::{COOKIE, HeaderMap, HeaderName, HeaderValue},
};
use serde_json::json;
use std::{
    io::Read,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct ResolverConfig {
    pub page_url: String,
    pub user_agent: String,
    pub referrer: String,
    pub app_code_name: String,
    pub app_name: String,
    pub device_platform: String,
    /// Stable anonymous device identity. Android should load this from private
    /// application storage and preserve it across channels and app restarts.
    pub device_guid: Option<String>,
    /// Existing normally obtained account cookies, when supplied by the user.
    /// Anonymous playback otherwise remains subject to the ordinary preview limits.
    pub account_cookies: String,
}
impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            page_url: "https://www.yangshipin.cn/tv/home".into(),
            user_agent: "ATV-Native/0.1 (Android TV; Rust)".into(),
            referrer: "https://www.yangshipin.cn/".into(),
            app_code_name: "ATV-Native".into(),
            app_name: "ATV-Native".into(),
            device_platform: "Android TV".into(),
            device_guid: None,
            account_cookies: String::new(),
        }
    }
}
impl fmt::Debug for ResolverConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolverConfig")
            .field("page_url", &self.page_url)
            .field("user_agent", &self.user_agent)
            .field("device_guid", &"[redacted]")
            .field("account_cookies", &"[redacted]")
            .finish()
    }
}

#[derive(Clone)]
pub struct LiveInfo {
    url: String,
    pub definition: String,
    pub cdn: String,
    pub preview_seconds: u64,
    pub preview_total_seconds: u64,
    pub attempts_remaining: u64,
    pub attempts_total: u64,
    pub playback_time: u64,
    pub formats: Value,
    pub extra_info: Value,
}
impl LiveInfo {
    pub fn url(&self) -> &str {
        &self.url
    }
}
impl fmt::Debug for LiveInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveInfo")
            .field("url", &"[redacted]")
            .field("definition", &self.definition)
            .field("preview_seconds", &self.preview_seconds)
            .field("attempts_remaining", &self.attempts_remaining)
            .finish()
    }
}

#[derive(Debug)]
pub enum ResolveError {
    Transport {
        stage: &'static str,
    },
    Http {
        stage: &'static str,
        status: u16,
    },
    Server {
        stage: &'static str,
        code: i64,
        iretcode: Option<i64>,
    },
    Malformed {
        stage: &'static str,
    },
    UnsupportedChallenge,
    Cancelled,
    ResponseTooLarge {
        stage: &'static str,
    },
}
impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ResolveError {}
struct CachedToken {
    value: String,
    expires_millis: u64,
}
struct State {
    sequence: RequestSequence,
    token: Option<CachedToken>,
}
pub struct LiveResolver {
    client: Client,
    config: ResolverConfig,
    guid: String,
    state: Mutex<State>,
}
impl fmt::Debug for LiveResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveResolver")
            .field("config", &self.config)
            .field("guid", &"[redacted]")
            .finish()
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn random(n: usize) -> String {
    let a = b"ABCDEFGHIJKlMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..n)
        .map(|_| a[rng.random_range(0..a.len())] as char)
        .collect()
}
fn numeric(v: &Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}
fn string(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
fn base36(mut n: u64) -> String {
    let mut s = Vec::new();
    loop {
        let r = (n % 36) as u8;
        s.push(if r < 10 { b'0' + r } else { b'a' + r - 10 });
        n /= 36;
        if n == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}
/// Generate a normal anonymous GUID once, then persist it in private application
/// storage. Do not regenerate it on channel switches or to reset preview limits.
pub fn generate_device_guid() -> String {
    format!("{}_{}", base36(now()), random(12).to_lowercase())
}
fn valid_device_guid(guid: &str) -> bool {
    !guid.is_empty()
        && guid.len() <= 128
        && guid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
impl LiveResolver {
    pub fn new(config: ResolverConfig) -> Result<Self, ResolveError> {
        let guid = config
            .device_guid
            .clone()
            .unwrap_or_else(generate_device_guid);
        if !valid_device_guid(&guid) {
            return Err(ResolveError::Malformed {
                stage: "device_guid",
            });
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(12))
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(&config.user_agent)
            .build()
            .map_err(|_| ResolveError::Transport {
                stage: "initialize",
            })?;
        Ok(Self {
            client,
            config,
            guid,
            state: Mutex::new(State {
                sequence: RequestSequence::default(),
                token: None,
            }),
        })
    }
    fn cookies(&self, seq: Option<(u32, &str)>) -> String {
        // These are the normal initCookie protocol defaults, not account identity.
        let mut s = format!(
            "guid={}; versionName=99.99.99; versionCode=999999; vplatform=109; platformVersion=ATV-Native; deviceModel=0.1; newLogin=1",
            self.guid
        );
        if let Some((n, r)) = seq {
            s.push_str(&format!("; nseqId={n}; nrequest-id={r}"));
        }
        if !self.config.account_cookies.is_empty() {
            s.push_str("; ");
            s.push_str(&self.config.account_cookies)
        }
        s
    }
    fn headers(&self, sequence: Option<(u32, &str)>) -> Result<HeaderMap, ResolveError> {
        let mut h = HeaderMap::new();
        h.insert(
            COOKIE,
            HeaderValue::from_str(&self.cookies(sequence))
                .map_err(|_| ResolveError::Malformed { stage: "cookies" })?,
        );
        for (k, v) in [
            ("yspappid", PLAYER_APP_ID),
            ("referer", self.config.page_url.as_str()),
        ] {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v)
                    .map_err(|_| ResolveError::Malformed { stage: "headers" })?,
            );
        }
        Ok(h)
    }
    fn json(
        &self,
        response: Result<reqwest::blocking::Response, reqwest::Error>,
        stage: &'static str,
    ) -> Result<Value, ResolveError> {
        let response = response.map_err(|_| ResolveError::Transport { stage })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ResolveError::Http { stage, status });
        }
        if response.content_length().is_some_and(|n| n > 1048576) {
            return Err(ResolveError::ResponseTooLarge { stage });
        }
        let mut bytes = Vec::new();
        response
            .take(1048577)
            .read_to_end(&mut bytes)
            .map_err(|_| ResolveError::Transport { stage })?;
        if bytes.len() > 1048576 {
            return Err(ResolveError::ResponseTooLarge { stage });
        }
        serde_json::from_slice(&bytes).map_err(|_| ResolveError::Malformed { stage })
    }
    fn sdk_token(&self, state: &mut State) -> Result<String, ResolveError> {
        let time = now();
        if let Some(t) = &state.token
            && time < t.expires_millis
        {
            return Ok(t.value.clone());
        }
        let params = json!({"yspappid":PLAYER_APP_ID,"guid":self.guid,"vappid":"59306155","vsecret":"b42702bf7309a179d102f3d51b1add2fda0bc7ada64cb801","raw":1,"version":"v1","ts":time,"rnd":token_rnd(PLAYER_APP_ID,"v1",&self.guid,time)});
        let body = self.json(
            self.client
                .get(OPEN_TOKEN_URL)
                .headers(self.headers(None)?)
                .query(params.as_object().unwrap())
                .send(),
            "open_token",
        )?;
        let ret = body["ret"].as_i64().ok_or(ResolveError::Malformed {
            stage: "open_token",
        })?;
        if ret != 0 {
            return Err(ResolveError::Server {
                stage: "open_token",
                code: ret,
                iretcode: None,
            });
        }
        let value = body["data"]["token"]
            .as_str()
            .filter(|x| !x.is_empty())
            .ok_or(ResolveError::Malformed {
                stage: "open_token",
            })?
            .to_owned();
        state.token = Some(CachedToken {
            value: value.clone(),
            expires_millis: now() + numeric(&body["data"]["expire"]).max(60000),
        });
        Ok(value)
    }
    pub fn resolve_live(&self, pid: &str, stream_id: &str) -> Result<LiveInfo, ResolveError> {
        self.resolve_quality(pid, stream_id, "hd", 1)
    }
    pub fn resolve_quality(
        &self,
        pid: &str,
        stream_id: &str,
        quality: &str,
        adjust: u8,
    ) -> Result<LiveInfo, ResolveError> {
        self.resolve_quality_cancellable(pid, stream_id, quality, adjust, &AtomicBool::new(false))
    }
    /// Call on a worker thread. Cancellation is checked between bounded HTTP
    /// operations; an already active request may take up to 12 seconds to return.
    /// The caller must discard stale playback generations after this returns.
    pub fn resolve_quality_cancellable(
        &self,
        pid: &str,
        stream_id: &str,
        quality: &str,
        adjust: u8,
        cancel: &AtomicBool,
    ) -> Result<LiveInfo, ResolveError> {
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(ResolveError::Cancelled)
            } else {
                Ok(())
            }
        };
        check()?;
        if ![pid, stream_id]
            .into_iter()
            .all(|s| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(ResolveError::Malformed {
                stage: "channel_identifier",
            });
        }
        if !matches!(quality, "sd" | "hd" | "fhd" | "shd") || adjust > 1 {
            return Err(ResolveError::Malformed { stage: "quality" });
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResolveError::Malformed { stage: "state" })?;
        check()?;
        let page = PageMetadata {
            url: &self.config.page_url,
            user_agent: &self.config.user_agent,
            referrer: &self.config.referrer,
            app_code_name: &self.config.app_code_name,
            app_name: &self.config.app_name,
            platform: &self.config.device_platform,
        };
        let time = now();
        let mut body = json!({"cnlid":stream_id,"livepid":pid,"stream":"2","guid":self.guid,"cKey":ckey(stream_id,(time+500)/1000,PLAYER_VERSION,&self.guid,PLAYER_PLATFORM,&page),"adjust":adjust,"sphttps":"1","platform":PLAYER_PLATFORM,"cmd":"2","encryptVer":"8.1","dtype":"1","devid":"devid","otype":"ojson","appVer":PLAYER_VERSION,"app_version":PLAYER_VERSION,"channel":"ysp_tx","defn":quality});
        let mut auth = json!({"pid":pid,"guid":self.guid,"appid":"ysp_pc","rand_str":random(10)});
        auth["signature"] = player_auth_signature(auth.as_object().unwrap())
            .map_err(|_| ResolveError::Malformed {
                stage: "auth_signature",
            })?
            .into();
        let response = self.json(
            self.client
                .post(PLAYER_AUTH_URL)
                .headers(self.headers(None)?)
                .form(auth.as_object().unwrap())
                .send(),
            "player_auth",
        )?;
        check()?;
        let data = &response["data"];
        if response["code"].as_i64().unwrap_or(0) != 0 {
            return Err(ResolveError::Server {
                stage: "player_auth",
                code: response["code"].as_i64().unwrap_or(-1),
                iretcode: None,
            });
        }
        let player_token =
            data["token"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(ResolveError::Malformed {
                    stage: "player_auth",
                })?;
        let server_ts = string(&data["ts"]);
        let token = self.sdk_token(&mut state)?;
        check()?;
        let (sequence, request_id) = state
            .sequence
            .next(&random(10), now())
            .map_err(|_| ResolveError::Malformed { stage: "sequence" })?;
        let input =
            sdk_signature_input(body.as_object().unwrap(), &self.guid, sequence, &request_id)
                .map_err(|_| ResolveError::Malformed { stage: "sdk_input" })?;
        let page_url = url::Url::parse(&self.config.page_url)
            .map_err(|_| ResolveError::Malformed { stage: "page_url" })?;
        let host = page_url
            .host_str()
            .ok_or(ResolveError::Malformed { stage: "page_host" })?;
        let sdk_sign = openapi_signature(
            &self.guid,
            host,
            &token,
            &format!("{}:", page_url.scheme()),
            PLAYER_APP_ID,
            &input,
        );
        let mut headers = self.headers(Some((sequence, &request_id)))?;
        for (k, v) in [
            (
                "yspsdkinput",
                sdk_input(body.as_object().unwrap())
                    .map_err(|_| ResolveError::Malformed { stage: "sdk_input" })?,
            ),
            ("yspsdksign", sdk_sign),
            ("seqid", sequence.to_string()),
            ("request-id", request_id),
            ("yspplayertoken", player_token.to_owned()),
            (
                "yspticket",
                player_ticket(pid, &server_ts, &self.guid, PLAYER_APP_ID, now() / 1000),
            ),
        ] {
            headers.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(&v).map_err(|_| ResolveError::Malformed {
                    stage: "live_headers",
                })?,
            );
        }
        body["rand_str"] = random(10).into();
        body["signature"] = live_signature(body.as_object().unwrap())
            .map_err(|_| ResolveError::Malformed {
                stage: "live_signature",
            })?
            .into();
        check()?;
        let response = self.json(
            self.client
                .post(LIVE_INFO_URL)
                .headers(headers)
                .json(&body)
                .send(),
            "get_live_info",
        )?;
        check()?;
        parse_live_info(&response)
    }
}
pub fn parse_live_info(response: &Value) -> Result<LiveInfo, ResolveError> {
    let code = response["code"]
        .as_i64()
        .or_else(|| response["ret"].as_i64())
        .ok_or(ResolveError::Malformed {
            stage: "get_live_info",
        })?;
    let data = &response["data"];
    let iretcode = data["iretcode"].as_i64();
    if code != 0 || iretcode != Some(0) {
        return Err(ResolveError::Server {
            stage: "get_live_info",
            code,
            iretcode,
        });
    }
    let playurl = data["playurl"]
        .as_str()
        .ok_or(ResolveError::Malformed { stage: "playurl" })?;
    let mut parsed =
        url::Url::parse(playurl).map_err(|_| ResolveError::Malformed { stage: "playurl" })?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ResolveError::Malformed { stage: "playurl" });
    }
    if data["chanll"].as_str().is_some_and(|s| !s.is_empty()) {
        return Err(ResolveError::UnsupportedChallenge);
    }
    // Preserve existing signed query octets: query_pairs_mut would decode and
    // re-encode them and could invalidate an upstream signature.
    let mut query = parsed.query().unwrap_or("").to_owned();
    if !query.is_empty() {
        query.push('&')
    }
    query.push_str("revoi=");
    if let Some(extra) = data["extended_param"].as_str().filter(|s| !s.is_empty()) {
        if !extra.starts_with('&')
            || extra.contains('#')
            || extra.bytes().any(|b| b.is_ascii_control())
        {
            return Err(ResolveError::Malformed {
                stage: "extended_param",
            });
        }
        query.push_str(extra);
    }
    parsed.set_query(Some(&query));
    let url = parsed.to_string();
    Ok(LiveInfo {
        url,
        definition: string(&data["defn"]),
        cdn: string(&data["cdn_name"]),
        preview_seconds: numeric(&data["playtime"]),
        preview_total_seconds: numeric(&data["totalplaytime"]),
        attempts_remaining: numeric(&data["restpreviewcnt"]),
        attempts_total: numeric(&data["previewcnt"]),
        playback_time: numeric(&data["playback"]["playbacktime"]),
        formats: data["formats"].clone(),
        extra_info: data["extraInfo"].clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_device_identity_is_reused_and_redacted() {
        let config = ResolverConfig {
            device_guid: Some("fixture_persistent_guid".into()),
            ..ResolverConfig::default()
        };
        let one = LiveResolver::new(config.clone()).unwrap();
        let two = LiveResolver::new(config).unwrap();
        assert_eq!(one.guid, two.guid);
        assert_eq!(one.guid, "fixture_persistent_guid");
        assert!(!format!("{one:?}").contains("fixture_persistent_guid"));
    }
    #[test]
    fn generated_guid_has_the_normal_safe_shape() {
        let guid = generate_device_guid();
        assert!(valid_device_guid(&guid));
        assert!(guid.contains('_'));
        for invalid in [
            "",
            "abc; guid=override",
            "a\r\nCookie: bad",
            "汉字",
            "abc%20def",
        ] {
            assert!(matches!(
                LiveResolver::new(ResolverConfig {
                    device_guid: Some(invalid.into()),
                    ..ResolverConfig::default()
                }),
                Err(ResolveError::Malformed {
                    stage: "device_guid"
                })
            ));
        }
    }
}
