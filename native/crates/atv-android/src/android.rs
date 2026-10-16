//! JNI marshaling only. Rust owns all playback state and work.
use crate::output::Window;
use atv_player::Session;
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString},
    sys::{jboolean, jint, jlong, jstring},
};
use std::{
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
};

static SESSIONS: OnceLock<Mutex<HashMap<i64, Arc<Session>>>> = OnceLock::new();
static NEXT: AtomicI64 = AtomicI64::new(1);
static RESOLVER: OnceLock<Mutex<Option<Arc<atv_auth::LiveResolver>>>> = OnceLock::new();
static UPDATES: OnceLock<Mutex<HashMap<i64, Arc<atv_update::Updater>>>> = OnceLock::new();
fn updates() -> &'static Mutex<HashMap<i64, Arc<atv_update::Updater>>> {
    UPDATES.get_or_init(Mutex::default)
}
fn update(handle: jlong) -> Option<Arc<atv_update::Updater>> {
    updates().lock().ok()?.get(&handle).cloned()
}
fn resolver(directory: &str) -> Option<Arc<atv_auth::LiveResolver>> {
    let mut shared = RESOLVER.get_or_init(Mutex::default).lock().ok()?;
    if let Some(resolver) = shared.as_ref() {
        return Some(resolver.clone());
    }
    let guid = atv_player::identity::load_or_create(Path::new(directory)).ok()?;
    let config = atv_auth::ResolverConfig {
        device_guid: Some(guid),
        ..Default::default()
    };
    let resolver = Arc::new(atv_auth::LiveResolver::new(config).ok()?);
    *shared = Some(resolver.clone());
    Some(resolver)
}
fn sessions() -> &'static Mutex<HashMap<i64, Arc<Session>>> {
    SESSIONS.get_or_init(Mutex::default)
}
fn session(handle: jlong) -> Option<Arc<Session>> {
    sessions().lock().ok()?.get(&handle).cloned()
}
fn boundary<T: Default>(action: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or_default()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_create(
    mut env: JNIEnv,
    _class: JClass,
    surface: JObject,
    _sample_rate: jint,
    storage_directory: JString,
) -> jlong {
    boundary(|| {
        if crate::legacy_api::initialize(&env).is_err() {
            return 0;
        }
        if surface.is_null() {
            return 0;
        }
        let Ok(directory) = env.get_string(&storage_directory).map(String::from) else {
            return 0;
        };
        let Some(resolver) = resolver(&directory) else {
            return 0;
        };
        let Ok(window) = Window::from_surface(&env, &surface) else {
            return 0;
        };
        let Ok(value) = Session::new(window, resolver) else {
            return 0;
        };
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        let Ok(mut registry) = sessions().lock() else {
            return 0;
        };
        registry.insert(handle, Arc::new(value));
        handle
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_select(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    pid: JString,
    stream_id: JString,
) {
    boundary(|| {
        let Some(session) = session(handle) else {
            return;
        };
        let Ok(pid) = env.get_string(&pid).map(String::from) else {
            return;
        };
        let Ok(stream_id) = env.get_string(&stream_id).map(String::from) else {
            return;
        };
        let _ = session.select(pid, stream_id);
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_pause(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    boundary(|| {
        if let Some(session) = session(handle) {
            let _ = session.pause();
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_resume(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    boundary(|| {
        if let Some(session) = session(handle) {
            let _ = session.resume();
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_destroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    boundary(|| {
        if let Ok(mut registry) = sessions().lock() {
            registry.remove(&handle);
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_status(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    boundary(|| {
        let snapshot = session(handle).map(|s| s.status()).unwrap_or_default();
        let json = serde_json::to_string(&snapshot)
            .unwrap_or_else(|_| "{\"state\":\"error\",\"message\":\"状态读取失败\"}".into());
        env.new_string(json)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_quality(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    value: JString,
) -> jboolean {
    boundary(|| {
        let Ok(value) = env.get_string(&value).map(String::from) else {
            return 0;
        };
        let Some(quality) = atv_player::Quality::parse(&value) else {
            return 0;
        };
        session(handle).is_some_and(|s| s.set_quality(quality).is_ok()) as jboolean
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_updateCreate(
    mut env: JNIEnv,
    _class: JClass,
    cache: JString,
    download: JString,
    version: jint,
    api: jint,
) -> jlong {
    boundary(|| {
        if crate::legacy_api::initialize(&env).is_err() || version < 1 || api < 19 {
            return 0;
        }
        let Ok(cache) = env.get_string(&cache).map(String::from) else {
            return 0;
        };
        let Ok(download) = env.get_string(&download).map(String::from) else {
            return 0;
        };
        let identity: serde_json::Value =
            match serde_json::from_str(include_str!("../../../../app/signing-certificate.json")) {
                Ok(value) => value,
                Err(_) => return 0,
            };
        let Some(certificate) = identity["certificateSha256"].as_str() else {
            return 0;
        };
        let abi = match std::env::consts::ARCH {
            "arm" => "armeabi-v7a",
            "aarch64" => "arm64-v8a",
            "x86" => "x86",
            "x86_64" => "x86_64",
            _ => return 0,
        };
        let config = atv_update::Config {
            current_version: version as u32,
            api: api as u32,
            abi: abi.into(),
            certificate: certificate.into(),
            cache_dir: Path::new(&cache).to_owned(),
            download_dir: Path::new(&download).to_owned(),
        };
        let Ok(updater) = atv_update::Updater::new(config) else {
            return 0;
        };
        let handle = NEXT.fetch_add(1, Ordering::Relaxed);
        let Ok(mut registry) = updates().lock() else {
            return 0;
        };
        registry.insert(handle, Arc::new(updater));
        handle
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_updateCommand(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    action: jint,
) -> jboolean {
    boundary(|| {
        let Some(updater) = update(handle) else {
            return 0;
        };
        (match action {
            0 => updater.start(false, false),
            1 => updater.start(true, false),
            2 => {
                updater.cancel();
                true
            }
            3 => updater.start(false, true),
            _ => false,
        }) as jboolean
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_updateStatus(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    boundary(|| {
        let Some(updater) = update(handle) else {
            return ptr::null_mut();
        };
        let Ok(json) = serde_json::to_string(&updater.status()) else {
            return ptr::null_mut();
        };
        env.new_string(json)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_updateVerify(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    boundary(|| update(handle).is_some_and(|u| u.verify_ready().is_ok()) as jboolean)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_cn_bobo_atv_nativeplayer_NativeBridge_updateDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    boundary(|| {
        if let Ok(mut registry) = updates().lock()
            && let Some(updater) = registry.remove(&handle)
        {
            updater.cancel();
        }
    });
}
