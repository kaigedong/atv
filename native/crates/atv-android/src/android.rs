//! JNI marshaling only. Rust owns all playback state and work.
use crate::output::Window;
use atv_player::Session;
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JString},
    sys::{jint, jlong, jstring},
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
