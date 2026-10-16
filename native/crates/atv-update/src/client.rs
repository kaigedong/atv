use crate::{Config, Error, check_cancel};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    sync::atomic::AtomicBool,
    time::Duration,
};
use url::Url;

pub const REPOSITORY: &str = "kaigedong/atv";
const APK_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Deserialize)]
pub struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}
#[derive(Deserialize)]
pub struct Release {
    pub tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Clone)]
pub struct Candidate {
    pub version: u32,
    pub name: String,
    pub url: Url,
    pub sha256: String,
    pub bytes: u64,
    pub release_url: String,
}
#[derive(Serialize, Deserialize)]
struct CachedMetadata {
    release: String,
    manifest: String,
}
fn sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn asset_url(release: &Release, asset: &Asset) -> Result<Url, Error> {
    let url = Url::parse(&asset.browser_download_url).map_err(|_| Error::Metadata)?;
    let path = format!(
        "/{REPOSITORY}/releases/download/{}/{}",
        release.tag_name, asset.name
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.path() != path
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Metadata);
    }
    Ok(url)
}
pub fn parse_release(bytes: &[u8]) -> Result<Release, Error> {
    let release: Release = serde_json::from_slice(bytes).map_err(|_| Error::Metadata)?;
    if release.draft
        || release.prerelease
        || release.tag_name.is_empty()
        || release.tag_name.len() > 80
        || !release
            .tag_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::Metadata);
    }
    Ok(release)
}
fn unique_asset<'a>(release: &'a Release, name: &str) -> Result<&'a Asset, Error> {
    let mut matches = release.assets.iter().filter(|a| a.name == name);
    let asset = matches.next().ok_or(Error::Metadata)?;
    if matches.next().is_some() {
        return Err(Error::Metadata);
    }
    Ok(asset)
}
fn asset_hash(asset: &Asset) -> Result<&str, Error> {
    asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| sha(s))
        .ok_or(Error::Metadata)
}
pub fn candidate(
    release: &Release,
    manifest_bytes: &[u8],
    config: &Config,
) -> Result<Option<Candidate>, Error> {
    let manifest_asset = unique_asset(release, "manifest.json")?;
    if hex::encode(Sha256::digest(manifest_bytes)) != asset_hash(manifest_asset)?
        || manifest_bytes.len() as u64 != manifest_asset.size
    {
        return Err(Error::Checksum);
    }
    let manifest: Value = serde_json::from_slice(manifest_bytes).map_err(|_| Error::Metadata)?;
    let version = manifest["versionCode"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= 2100000000)
        .ok_or(Error::Metadata)? as u32;
    let name = manifest["versionName"]
        .as_str()
        .filter(|s| s.len() <= 80)
        .ok_or(Error::Metadata)?;
    let commit = manifest["sourceCommit"].as_str().ok_or(Error::Metadata)?;
    if manifest["schemaVersion"] != 1
        || manifest["tag"] != release.tag_name
        || format!("v{name}") != release.tag_name
        || commit.len() != 40
        || !commit.bytes().all(|b| b.is_ascii_hexdigit())
        || manifest["android"]["sourceCommit"] != commit
        || manifest["android"]["versionCode"] != version
    {
        return Err(Error::Metadata);
    }
    if version <= config.current_version {
        return Ok(None);
    }
    let artifacts = manifest["android"]["artifacts"]
        .as_array()
        .ok_or(Error::Metadata)?;
    let entry = artifacts
        .iter()
        .find(|e| e["abi"] == config.abi)
        .or_else(|| artifacts.iter().find(|e| e["abi"] == "universal"))
        .ok_or(Error::Incompatible)?;
    let abi = entry["abi"].as_str().ok_or(Error::Metadata)?;
    let file = format!("atv-android-{abi}-release.apk");
    let size = entry["minSdk"]
        .as_u64()
        .filter(|sdk| *sdk <= u64::from(config.api))
        .ok_or(Error::Incompatible)?;
    if size < 19
        || entry["file"] != file
        || entry["signature"]["certificateSha256"] != config.certificate
    {
        return Err(Error::Metadata);
    }
    for scheme in ["v1", "v2", "v3"] {
        if entry["signature"]["schemes"][scheme] != true {
            return Err(Error::Metadata);
        }
    }
    let expected = entry["sha256"]
        .as_str()
        .filter(|s| sha(s))
        .ok_or(Error::Metadata)?;
    let asset = unique_asset(release, &file)?;
    if asset.size == 0 || asset.size > APK_LIMIT || asset_hash(asset)? != expected {
        return Err(Error::Metadata);
    }
    let products = manifest["assets"].as_array().ok_or(Error::Metadata)?;
    let product = products
        .iter()
        .find(|e| e["file"] == file)
        .ok_or(Error::Metadata)?;
    if product["sha256"] != expected || product["bytes"] != asset.size {
        return Err(Error::Metadata);
    }
    Ok(Some(Candidate {
        version,
        name: name.to_owned(),
        url: asset_url(release, asset)?,
        sha256: expected.into(),
        bytes: asset.size,
        release_url: format!(
            "https://github.com/{REPOSITORY}/releases/tag/{}",
            release.tag_name
        ),
    }))
}
fn trusted(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
pub fn client() -> Result<Client, Error> {
    Client::builder()
        .https_only(true)
        .user_agent("ATV-Native-Updater/0.1")
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if trusted(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| Error::Network)
}
fn response(
    client: &Client,
    url: &Url,
    cancel: &AtomicBool,
    metadata: bool,
) -> Result<Response, Error> {
    check_cancel(cancel)?;
    if !trusted(url) {
        return Err(Error::Metadata);
    }
    let response = client
        .get(url.clone())
        .timeout(Duration::from_secs(if metadata { 12 } else { 45 }))
        .send()
        .map_err(|_| Error::Network)?;
    check_cancel(cancel)?;
    if response.status().as_u16() == 403 || response.status().as_u16() == 429 {
        return Err(Error::RateLimited);
    }
    if !response.status().is_success() || !trusted(response.url()) {
        return Err(Error::Network);
    }
    Ok(response)
}
fn bytes(client: &Client, url: &Url, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>, Error> {
    let mut response = response(client, url, cancel, true)?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(Error::Metadata);
    }
    let mut result = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        check_cancel(cancel)?;
        let length = response.read(&mut buffer).map_err(|_| Error::Network)?;
        if length == 0 {
            break;
        }
        if result.len() + length > limit {
            return Err(Error::Metadata);
        }
        result.extend_from_slice(&buffer[..length]);
    }
    Ok(result)
}
pub fn discover(config: &Config, cancel: &AtomicBool) -> Result<Option<Candidate>, Error> {
    let client = client()?;
    let url = Url::parse(&format!(
        "https://api.github.com/repos/{REPOSITORY}/releases/latest"
    ))
    .map_err(|_| Error::Metadata)?;
    let release_bytes = bytes(&client, &url, 1024 * 1024, cancel)?;
    let release = parse_release(&release_bytes)?;
    let manifest = unique_asset(&release, "manifest.json")?;
    let raw = bytes(&client, &asset_url(&release, manifest)?, 512 * 1024, cancel)?;
    let choice = candidate(&release, &raw, config)?;
    check_cancel(cancel)?;
    // Cache only the already validated public metadata in private app storage.
    // Restarts retain the update badge without spending another API request.
    // Preserve exact manifest octets: its GitHub asset digest covers those bytes.
    let record = CachedMetadata {
        release: String::from_utf8(release_bytes).map_err(|_| Error::Metadata)?,
        manifest: STANDARD.encode(raw),
    };
    if let Ok(record) = serde_json::to_vec(&record) {
        let _ = fs::create_dir_all(&config.cache_dir);
        let temporary = config.cache_dir.join("metadata.tmp");
        if fs::write(&temporary, record).is_ok() {
            let _ = fs::rename(&temporary, config.cache_dir.join("metadata.json"));
        }
    }
    Ok(choice)
}

pub fn cached(config: &Config) -> Result<Option<Candidate>, Error> {
    let path = config.cache_dir.join("metadata.json");
    if fs::metadata(&path).map_err(|_| Error::Storage)?.len() > 3 * 1024 * 1024 {
        return Err(Error::Metadata);
    }
    let record: CachedMetadata =
        serde_json::from_slice(&fs::read(path).map_err(|_| Error::Storage)?)
            .map_err(|_| Error::Metadata)?;
    let release = parse_release(record.release.as_bytes())?;
    let raw = STANDARD
        .decode(record.manifest)
        .map_err(|_| Error::Metadata)?;
    if raw.len() > 512 * 1024 {
        return Err(Error::Metadata);
    }
    candidate(&release, &raw, config)
}
/// APK bytes never accumulate in RAM. The same bounded transfer verifies cached files before installation.
pub fn copy_verified(
    reader: &mut impl Read,
    writer: &mut impl Write,
    size: u64,
    expected: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), Error> {
    let mut buffer = [0; 65536];
    let mut digest = Sha256::new();
    let mut count = 0u64;
    loop {
        check_cancel(cancel)?;
        let length = reader.read(&mut buffer).map_err(|_| Error::Network)?;
        if length == 0 {
            break;
        }
        count += length as u64;
        if count > size {
            return Err(Error::Checksum);
        }
        writer
            .write_all(&buffer[..length])
            .map_err(|_| Error::Storage)?;
        digest.update(&buffer[..length]);
        progress(count);
    }
    check_cancel(cancel)?;
    if count != size || hex::encode(digest.finalize()) != expected {
        return Err(Error::Checksum);
    }
    Ok(())
}
pub fn download(
    candidate: &Candidate,
    writer: &mut impl Write,
    cancel: &AtomicBool,
    progress: impl FnMut(u64),
) -> Result<(), Error> {
    let client = client()?;
    let mut response = response(&client, &candidate.url, cancel, false)?;
    if response
        .content_length()
        .is_some_and(|n| n != candidate.bytes)
    {
        return Err(Error::Checksum);
    }
    copy_verified(
        &mut response,
        writer,
        candidate.bytes,
        &candidate.sha256,
        cancel,
        progress,
    )
}
