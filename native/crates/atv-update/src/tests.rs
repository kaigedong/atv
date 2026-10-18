use super::*;
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

fn fixture() -> (Config, Value, Value) {
    let config = Config {
        current_version: 1,
        api: 19,
        abi: "armeabi-v7a".into(),
        certificate: "b".repeat(64),
        cache_dir: std::env::temp_dir(),
        download_dir: std::env::temp_dir(),
    };
    let filename = "atv-android-armeabi-v7a-release.apk";
    let digest = "a".repeat(64);
    let manifest = json!({"schemaVersion":1,"sourceCommit":"c".repeat(40),"versionCode":1007,"versionName":"0.1.0-r1007","tag":"v0.1.0-r1007",
        "assets":[{"file":filename,"sha256":digest,"bytes":123}],
        "android":{"sourceCommit":"c".repeat(40),"versionCode":1007,
            "artifacts":[{"abi":"armeabi-v7a","file":filename,"sha256":digest,"minSdk":19,
                "signature":{"certificateSha256":config.certificate,"schemes":{"v1":true,"v2":true,"v3":true}}}]}});
    let release = json!({"tag_name":"v0.1.0-r1007","draft":false,"prerelease":false,"assets":[
        {"name":filename,"size":123,"digest":format!("sha256:{digest}"),
            "browser_download_url":format!("https://github.com/kaigedong/atv/releases/download/v0.1.0-r1007/{filename}")},
        {"name":"manifest.json","size":0,"digest":"","browser_download_url":"https://github.com/kaigedong/atv/releases/download/v0.1.0-r1007/manifest.json"}]});
    (config, manifest, release)
}
fn select(
    config: &Config,
    manifest: &Value,
    mut release: Value,
) -> Result<Option<client::Candidate>, Error> {
    let bytes = serde_json::to_vec(manifest).unwrap();
    release["assets"][1]["size"] = json!(bytes.len());
    release["assets"][1]["digest"] =
        json!(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))));
    let release = client::parse_release(&serde_json::to_vec(&release).unwrap())?;
    client::candidate(&release, &bytes, config)
}
#[test]
fn exact_device_asset_and_newer_version_are_selected() {
    let (mut config, manifest, release) = fixture();
    let choice = select(&config, &manifest, release.clone())
        .unwrap()
        .unwrap();
    assert_eq!(choice.version, 1007);
    assert_eq!(choice.bytes, 123);
    assert!(choice.url.path().ends_with("armeabi-v7a-release.apk"));
    config.current_version = 1007;
    assert!(
        select(&config, &manifest, release.clone())
            .unwrap()
            .is_none()
    );
    config.current_version = 1008;
    assert!(select(&config, &manifest, release).unwrap().is_none());
}
#[test]
fn wrong_signer_incompatible_sdk_and_foreign_download_are_rejected() {
    let (config, mut manifest, release) = fixture();
    manifest["android"]["artifacts"][0]["signature"]["certificateSha256"] = json!("d".repeat(64));
    assert!(matches!(
        select(&config, &manifest, release.clone()),
        Err(Error::Metadata)
    ));
    let (_, mut manifest, _) = fixture();
    manifest["android"]["artifacts"][0]["minSdk"] = json!(21);
    assert!(matches!(
        select(&config, &manifest, release.clone()),
        Err(Error::Incompatible)
    ));
    let (_, manifest, mut release) = fixture();
    release["assets"][0]["browser_download_url"] = json!("https://fixture.invalid/untrusted.apk");
    assert!(matches!(
        select(&config, &manifest, release),
        Err(Error::Metadata)
    ));
}
#[test]
fn missing_legacy_signature_and_mismatched_product_digest_are_rejected() {
    let (config, mut manifest, release) = fixture();
    manifest["android"]["artifacts"][0]["signature"]["schemes"]["v1"] = json!(false);
    assert!(select(&config, &manifest, release.clone()).is_err());
    let (_, mut manifest, _) = fixture();
    manifest["assets"][0]["sha256"] = json!("e".repeat(64));
    assert!(select(&config, &manifest, release).is_err());
}
#[test]
fn prereleases_and_tampered_manifest_are_rejected() {
    let (config, manifest, mut release) = fixture();
    release["prerelease"] = json!(true);
    assert!(select(&config, &manifest, release).is_err());
    let (_, _, mut release) = fixture();
    release["assets"][1]["digest"] = json!(format!("sha256:{}", "a".repeat(64)));
    let release = client::parse_release(&serde_json::to_vec(&release).unwrap()).unwrap();
    assert!(matches!(
        client::candidate(&release, b"tampered", &config),
        Err(Error::Checksum)
    ));
}
struct Metered {
    input: Cursor<Vec<u8>>,
    largest_request: usize,
}
impl Read for Metered {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.largest_request = self.largest_request.max(buffer.len());
        self.input.read(buffer)
    }
}
#[test]
fn large_download_uses_bounded_chunks_and_exact_digest() {
    let bytes = vec![17; 1024 * 1024 + 23];
    let digest = hex::encode(Sha256::digest(&bytes));
    let mut reader = Metered {
        input: Cursor::new(bytes.clone()),
        largest_request: 0,
    };
    let mut output = Vec::new();
    client::copy_verified(
        &mut reader,
        &mut output,
        bytes.len() as u64,
        &digest,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(output, bytes);
    assert!(reader.largest_request <= 65536);
}
#[test]
fn cancellation_truncation_overflow_and_bad_hash_never_complete() {
    let cancel = AtomicBool::new(false);
    let data = vec![1; 200000];
    let digest = hex::encode(Sha256::digest(&data));
    let mut output = Vec::new();
    assert!(matches!(
        client::copy_verified(
            &mut Cursor::new(&data),
            &mut output,
            data.len() as u64,
            &digest,
            &cancel,
            |_| cancel.store(true, Ordering::Release)
        ),
        Err(Error::Cancelled)
    ));
    assert!(output.len() <= 65536);
    for (bytes, size, hash) in [
        (b"abc".as_slice(), 4, digest.as_str()),
        (b"abc".as_slice(), 2, digest.as_str()),
        (b"abc".as_slice(), 3, digest.as_str()),
    ] {
        assert!(matches!(
            client::copy_verified(
                &mut Cursor::new(bytes),
                &mut io::sink(),
                size,
                hash,
                &AtomicBool::new(false),
                |_| {}
            ),
            Err(Error::Checksum)
        ));
    }
}

#[test]
fn automatic_checks_require_a_full_hour() {
    let last_check = 10_000;
    for state in ["available", "current", "error", "cancelled"] {
        assert!(!check_allowed(state, true, last_check, last_check));
        assert!(!check_allowed(state, true, last_check, last_check + 3599));
        assert!(check_allowed(state, true, last_check, last_check + 3600));
        assert!(check_allowed(state, true, last_check, last_check + 3601));
        assert!(!check_allowed(state, true, last_check, last_check - 1));
    }
    // Missing or invalid metadata must remain recoverable even with a recent timestamp.
    assert!(check_allowed("idle", true, last_check, last_check));
}

#[test]
fn manual_checks_bypass_automatic_throttling() {
    let last_check = 10_000;
    for state in [
        "available",
        "current",
        "error",
        "cancelled",
        "idle",
        "ready",
    ] {
        assert!(check_allowed(state, false, last_check, last_check));
        assert!(check_allowed(state, false, last_check, last_check - 1));
    }
}

#[test]
fn busy_updater_rejects_manual_and_automatic_operations() {
    let (config, _, _) = fixture();
    let updater = Updater::new(config).unwrap();
    updater.shared.lock().unwrap().busy = true;
    let before = serde_json::to_value(updater.status()).unwrap();
    for (download, automatic) in [(false, false), (false, true), (true, false), (true, true)] {
        assert!(!updater.start(download, automatic));
        assert_eq!(serde_json::to_value(updater.status()).unwrap(), before);
    }
    assert!(updater.shared.lock().unwrap().busy);
}

#[test]
fn ready_package_survives_automatic_checks_and_is_rehashed_before_installation() {
    let directory = std::env::temp_dir().join(format!(
        "atv-update-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(directory.join("slot-1-1")).unwrap();
    let (mut config, _, _) = fixture();
    config.cache_dir = directory.clone();
    config.download_dir = directory.clone();
    let updater = Updater::new(config).unwrap();
    let content = b"fixture-apk";
    let path = directory.join("slot-1-1/update.apk");
    fs::write(&path, content).unwrap();
    {
        let mut state = updater.shared.lock().unwrap();
        state.status.state = "ready";
        state.status.ready_path = "slot-1-1/update.apk".into();
        state.candidate = Some(client::Candidate {
            version: 1007,
            name: "fixture".into(),
            url: "https://fixture.invalid/update.apk".parse().unwrap(),
            sha256: hex::encode(Sha256::digest(content)),
            bytes: content.len() as u64,
            release_url: String::new(),
        });
    }
    let before = serde_json::to_value(updater.status()).unwrap();
    assert_eq!(updater.status().last_check_at, 0);
    assert!(!check_allowed("ready", true, 10_000, 13_600));
    assert!(!check_allowed("ready", true, 0, u64::MAX));
    for download in [false, true] {
        assert!(!updater.start(download, true));
        assert_eq!(serde_json::to_value(updater.status()).unwrap(), before);
        assert!(!updater.shared.lock().unwrap().busy);
        assert!(updater.shared.lock().unwrap().candidate.is_some());
        assert_eq!(fs::read(&path).unwrap(), content);
    }
    updater.verify_ready().unwrap();
    fs::write(&path, b"changed-apk").unwrap();
    assert!(matches!(updater.verify_ready(), Err(Error::Checksum)));
    assert_eq!(updater.status().state, "error");
    assert!(updater.status().ready_path.is_empty());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn restart_retains_valid_update_and_throttles_only_when_cache_is_valid() {
    let directory = std::env::temp_dir().join(format!(
        "atv-update-cache-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let (mut config, manifest, mut release) = fixture();
    config.cache_dir = directory.clone();
    config.download_dir = directory.clone();
    let raw = serde_json::to_vec(&manifest).unwrap();
    release["assets"][1]["size"] = json!(raw.len());
    release["assets"][1]["digest"] = json!(format!("sha256:{}", hex::encode(Sha256::digest(&raw))));
    let cached = json!({"release":serde_json::to_string(&release).unwrap(), "manifest":base64::engine::general_purpose::STANDARD.encode(raw)});
    fs::write(
        directory.join("metadata.json"),
        serde_json::to_vec(&cached).unwrap(),
    )
    .unwrap();
    let timestamp = now();
    fs::write(directory.join("last-check"), timestamp.to_string()).unwrap();
    let updater = Updater::new(config.clone()).unwrap();
    assert_eq!(updater.status().state, "available");
    assert_eq!(updater.status().last_check_at, timestamp);
    let status = serde_json::to_value(updater.status()).unwrap();
    assert_eq!(status["lastCheckAt"], json!(timestamp));
    assert!(status.get("last_check_at").is_none());
    assert!(!updater.start(false, true));
    assert!(!check_allowed(
        updater.status().state,
        true,
        updater.status().last_check_at,
        timestamp + 3599,
    ));
    assert!(check_allowed(
        updater.status().state,
        true,
        updater.status().last_check_at,
        timestamp + 3600,
    ));
    config.current_version = 1007;
    let current = Updater::new(config.clone()).unwrap().status();
    assert_eq!(current.state, "current");
    assert_eq!(current.last_check_at, timestamp);
    fs::write(directory.join("metadata.json"), b"corrupted").unwrap();
    let recovered = Updater::new(config.clone()).unwrap().status();
    assert_eq!(recovered.state, "idle");
    assert_eq!(recovered.last_check_at, timestamp);
    assert!(check_allowed(
        recovered.state,
        true,
        recovered.last_check_at,
        timestamp,
    ));
    fs::write(directory.join("last-check"), b"invalid").unwrap();
    assert_eq!(Updater::new(config).unwrap().status().last_check_at, 0);
    fs::remove_dir_all(directory).unwrap();
}
