//! Portable release checks/downloads. OS package inspection and installation belong to the host.
pub mod client;
#[cfg(test)]
mod tests;
use client::Candidate;
use serde::Serialize;
use std::{
    fs::{self, File},
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("update network request failed")]
    Network,
    #[error("invalid release metadata")]
    Metadata,
    #[error("update checksum mismatch")]
    Checksum,
    #[error("update storage failed")]
    Storage,
    #[error("update cancelled")]
    Cancelled,
    #[error("release does not support this device")]
    Incompatible,
    #[error("update request rate limited")]
    RateLimited,
}
impl Error {
    fn message(&self) -> &'static str {
        match self {
            Self::Network => "更新服务器连接失败，请稍后重试",
            Self::Metadata => "发布信息不完整或校验信息异常",
            Self::Checksum => "安装包校验失败，请重新下载",
            Self::Storage => "存储空间不足或更新文件无法写入",
            Self::Cancelled => "已取消更新任务",
            Self::Incompatible => "新版本暂不支持当前设备",
            Self::RateLimited => "检查更新请求受限，请稍后重试",
        }
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
#[derive(Clone)]
pub struct Config {
    pub current_version: u32,
    pub api: u32,
    pub abi: String,
    pub certificate: String,
    pub cache_dir: PathBuf,
    pub download_dir: PathBuf,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: &'static str,
    pub message: &'static str,
    pub version_code: u32,
    pub version_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub ready_path: String,
    pub certificate_sha256: String,
    pub release_url: String,
    pub last_check_at: u64,
}
struct Shared {
    status: Status,
    candidate: Option<Candidate>,
    busy: bool,
}
pub struct Updater {
    config: Config,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
}
static WORKERS: AtomicUsize = AtomicUsize::new(0);
static FILE_ID: AtomicU64 = AtomicU64::new(1);
struct Worker;
impl Drop for Worker {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
const AUTOMATIC_CHECK_INTERVAL_SECS: u64 = 60 * 60;

fn last_check_at(config: &Config) -> u64 {
    fs::read_to_string(config.cache_dir.join("last-check"))
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

fn check_allowed(state: &str, automatic: bool, last_check: u64, current_time: u64) -> bool {
    !automatic
        || (state != "ready"
            && (state == "idle"
                || current_time.saturating_sub(last_check) >= AUTOMATIC_CHECK_INTERVAL_SECS))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Updater {
    pub fn new(config: Config) -> Result<Self, Error> {
        if !config.cache_dir.is_absolute()
            || !config.download_dir.is_absolute()
            || config.api < 19
            || !matches!(
                config.abi.as_str(),
                "armeabi-v7a" | "arm64-v8a" | "x86" | "x86_64"
            )
            || config.certificate.len() != 64
            || !config.certificate.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Metadata);
        }
        let mut status = Status {
            state: "idle",
            message: "尚未检查更新",
            version_code: 0,
            version_name: String::new(),
            downloaded_bytes: 0,
            total_bytes: 0,
            ready_path: String::new(),
            certificate_sha256: config.certificate.clone(),
            release_url: String::new(),
            last_check_at: last_check_at(&config),
        };
        let cached = client::cached(&config);
        let candidate = match cached {
            Ok(Some(candidate)) => {
                status.state = "available";
                status.message = "发现新版本";
                status.version_code = candidate.version;
                status.version_name = candidate.name.clone();
                status.total_bytes = candidate.bytes;
                status.release_url = candidate.release_url.clone();
                Some(candidate)
            }
            Ok(None) => {
                status.state = "current";
                status.message = "当前已是最新版本";
                status.version_code = config.current_version;
                None
            }
            Err(_) => None,
        };
        Ok(Self {
            config,
            shared: Arc::new(Mutex::new(Shared {
                status,
                candidate,
                busy: false,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn status(&self) -> Status {
        self.shared.lock().expect("update state").status.clone()
    }
    pub fn cancel(&self) {
        let mut shared = self.shared.lock().expect("update state");
        if shared.busy {
            self.cancel.store(true, Ordering::Release);
            shared.status.state = "cancelling";
            shared.status.message = "正在取消更新任务…";
        }
    }
    /// One background operation per controller; at most two process-wide across activity recreation.
    pub fn start(&self, download: bool, automatic: bool) -> bool {
        let mut shared = self.shared.lock().expect("update state");
        if shared.busy {
            return false;
        }
        let timestamp = if automatic {
            last_check_at(&self.config)
        } else {
            0
        };
        if !check_allowed(shared.status.state, automatic, timestamp, now()) {
            return false;
        }
        let candidate = if download {
            let Some(candidate) = shared.candidate.clone() else {
                return false;
            };
            Some(candidate)
        } else {
            None
        };
        if WORKERS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .is_err()
        {
            shared.status.state = "error";
            shared.status.message = "上次更新任务正在结束，请稍后重试";
            return false;
        }
        self.cancel.store(false, Ordering::Release);
        shared.busy = true;
        shared.status.state = if download { "downloading" } else { "checking" };
        shared.status.message = if download {
            "正在下载更新…"
        } else {
            "正在检查更新…"
        };
        shared.status.downloaded_bytes = 0;
        shared.status.ready_path.clear();
        let state = self.shared.clone();
        let cancel = self.cancel.clone();
        let config = self.config.clone();
        let guard = Worker;
        let spawned = thread::Builder::new()
            .name("atv-update".into())
            .spawn(move || {
                let _guard = guard;
                let result = if let Some(candidate) = candidate {
                    download_file(&config, &candidate, &cancel, &state).map(Some)
                } else {
                    client::discover(&config, &cancel).and_then(|candidate| {
                        check_cancel(&cancel)?;
                        let timestamp = now();
                        let _ = fs::create_dir_all(&config.cache_dir);
                        let _ =
                            fs::write(config.cache_dir.join("last-check"), timestamp.to_string());
                        let mut state = state.lock().expect("update state");
                        state.status.last_check_at = timestamp;
                        state.candidate = candidate.clone();
                        if let Some(candidate) = candidate {
                            state.status.state = "available";
                            state.status.message = "发现新版本";
                            state.status.version_code = candidate.version;
                            state.status.version_name = candidate.name;
                            state.status.total_bytes = candidate.bytes;
                            state.status.release_url = candidate.release_url;
                        } else {
                            state.status.state = "current";
                            state.status.message = "当前已是最新版本";
                            state.status.version_code = config.current_version;
                            state.status.version_name.clear();
                            state.status.total_bytes = 0;
                            state.status.release_url.clear();
                        }
                        Ok(None)
                    })
                };
                let mut state = state.lock().expect("update state");
                state.busy = false;
                match result {
                    Ok(Some(path)) if !cancel.load(Ordering::Acquire) => {
                        state.status.state = "ready";
                        state.status.message = "下载完成，校验通过，可以安装";
                        state.status.ready_path = path;
                    }
                    Ok(_) if cancel.load(Ordering::Acquire) => {
                        state.status.state = "cancelled";
                        state.status.message = "已取消更新任务";
                    }
                    Ok(_) => {}
                    Err(error) => {
                        state.status.state = if matches!(error, Error::Cancelled) {
                            "cancelled"
                        } else {
                            "error"
                        };
                        state.status.message = error.message();
                    }
                }
            });
        if spawned.is_err() {
            shared.busy = false;
            shared.status.state = "error";
            shared.status.message = "更新任务无法启动";
            return false;
        }
        true
    }
    /// Hosts call this off the UI thread immediately before OS package/signature inspection.
    pub fn verify_ready(&self) -> Result<(), Error> {
        let (candidate, path) = {
            let mut state = self.shared.lock().expect("update state");
            if state.busy || state.status.state != "ready" {
                return Err(Error::Metadata);
            }
            let candidate = state.candidate.clone().ok_or(Error::Metadata)?;
            let path = self.config.download_dir.join(&state.status.ready_path);
            state.busy = true;
            state.status.state = "verifying";
            state.status.message = "正在验证安装包…";
            (candidate, path)
        };
        let result = File::open(path)
            .map_err(|_| Error::Storage)
            .and_then(|mut file| {
                client::copy_verified(
                    &mut file,
                    &mut io::sink(),
                    candidate.bytes,
                    &candidate.sha256,
                    &self.cancel,
                    |_| {},
                )
            });
        let mut state = self.shared.lock().expect("update state");
        state.busy = false;
        if let Err(error) = &result {
            state.status.state = "error";
            state.status.message = error.message();
            state.status.ready_path.clear();
        } else {
            state.status.state = "ready";
            state.status.message = "安装包校验通过";
        }
        result
    }
}
impl Drop for Updater {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir(self.0.parent().unwrap());
    }
}
fn download_file(
    config: &Config,
    candidate: &Candidate,
    cancel: &AtomicBool,
    shared: &Mutex<Shared>,
) -> Result<String, Error> {
    check_cancel(cancel)?;
    fs::create_dir_all(&config.download_dir).map_err(|_| Error::Storage)?;
    // Delete only updater-created cache directories more than a day old. Never touch user media.
    if let Ok(entries) = fs::read_dir(&config.download_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let old = name
                .strip_prefix("slot-")
                .and_then(|s| s.split('-').next())
                .and_then(|s| s.parse::<u64>().ok());
            if old.is_some_and(|time| now().saturating_sub(time) > 86400)
                && entry.file_type().is_ok_and(|t| t.is_dir())
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    let (slot, directory) = loop {
        let slot = format!("slot-{}-{}", now(), FILE_ID.fetch_add(1, Ordering::Relaxed));
        let directory = config.download_dir.join(&slot);
        match fs::create_dir(&directory) {
            Ok(()) => break (slot, directory),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(Error::Storage),
        }
    };
    let partial = Partial(directory.join("update.part"));
    let mut file = File::create(&partial.0).map_err(|_| Error::Storage)?;
    client::download(candidate, &mut file, cancel, |count| {
        let mut state = shared.lock().expect("update state");
        state.status.downloaded_bytes = count;
    })?;
    file.sync_all().map_err(|_| Error::Storage)?;
    drop(file);
    check_cancel(cancel)?;
    fs::rename(&partial.0, directory.join("update.apk")).map_err(|_| Error::Storage)?;
    Ok(format!("{slot}/update.apk"))
}
