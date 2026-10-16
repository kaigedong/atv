//! Explicit opt-in real GitHub check/download; default tests never use the network.
use atv_update::{Config, Updater};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn main() {
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().expect("absolute output directory"));
    let current_version = args.next().unwrap_or_else(|| "1".into()).parse().unwrap();
    let download = args.next().as_deref() == Some("download");
    let identity: serde_json::Value =
        serde_json::from_str(include_str!("../../../../app/signing-certificate.json")).unwrap();
    let updater = Updater::new(Config {
        current_version,
        api: 19,
        abi: "armeabi-v7a".into(),
        certificate: identity["certificateSha256"].as_str().unwrap().into(),
        cache_dir: directory.clone(),
        download_dir: directory,
    })
    .unwrap();
    assert!(updater.start(false, false));
    let started = Instant::now();
    loop {
        assert!(started.elapsed() < Duration::from_secs(90));
        let status = updater.status();
        if !matches!(status.state, "checking" | "downloading") {
            if status.state == "available" && download {
                assert!(updater.start(true, false));
            } else {
                println!("{}", serde_json::to_string(&status).unwrap());
                assert!(matches!(status.state, "available" | "current" | "ready"));
                if status.state == "ready" {
                    updater.verify_ready().unwrap();
                }
                break;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}
