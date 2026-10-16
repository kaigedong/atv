//! Main-thread C ABI. Callbacks never outlive a handle or run on network threads.
//! Swift owns AVFoundation; Rust owns identity, authorization, HLS and CMG.
use atv_player::{MediaOutput, OutputError, OutputFactory, OutputStats, Player};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{CStr, c_char, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::Arc,
};

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AtvOutputStats {
    pub video_frames: u64,
    pub audio_frames: u64,
    pub position_us: i64,
    pub width: u32,
    pub height: u32,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
}
type Push = unsafe extern "C" fn(*mut c_void, *const u8, usize, i64) -> i32;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AtvCallbacks {
    pub abi_version: u32,
    pub context: *mut c_void,
    pub reset: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub video: Option<Push>,
    pub audio: Option<Push>,
    pub drain: Option<unsafe extern "C" fn(*mut c_void, *mut AtvOutputStats) -> i32>,
}
struct Output(AtvCallbacks);
fn result(code: i32) -> Result<(), OutputError> {
    match code {
        0 => Ok(()),
        1 => Err(OutputError::Backpressure),
        _ => Err(OutputError::Backend {
            operation: "Apple media callback",
            status: code,
        }),
    }
}
impl OutputFactory for AtvCallbacks {
    fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError> {
        result(unsafe { (self.reset.unwrap())(self.context) })?;
        Ok(Box::new(Output(*self)))
    }
}
impl MediaOutput for Output {
    fn push_video(&mut self, bytes: &[u8], pts: i64) -> Result<(), OutputError> {
        result(unsafe { (self.0.video.unwrap())(self.0.context, bytes.as_ptr(), bytes.len(), pts) })
    }
    fn push_audio(&mut self, bytes: &[u8], pts: i64) -> Result<(), OutputError> {
        result(unsafe { (self.0.audio.unwrap())(self.0.context, bytes.as_ptr(), bytes.len(), pts) })
    }
    fn drain(&mut self) -> Result<OutputStats, OutputError> {
        let mut stats = AtvOutputStats::default();
        result(unsafe { (self.0.drain.unwrap())(self.0.context, &mut stats) })?;
        Ok(OutputStats {
            video_frames: stats.video_frames,
            audio_frames: stats.audio_frames,
            position_us: stats.position_us,
            width: stats.width,
            height: stats.height,
            audio_sample_rate: stats.audio_sample_rate,
            audio_channels: stats.audio_channels,
            ..Default::default()
        })
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        unsafe {
            (self.0.reset.unwrap())(self.0.context);
        }
    }
}
thread_local! {
    static PLAYERS: RefCell<HashMap<u64, Player<AtvCallbacks>>> = RefCell::default();
}
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn boundary<T: Default>(action: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or_default()
}
unsafe fn input<'a>(value: *const c_char) -> Option<&'a str> {
    if value.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(value) }.to_str().ok()
    }
}
fn control(action: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or(-1)
}
fn with_player(handle: u64, action: impl FnOnce(&mut Player<AtvCallbacks>)) -> i32 {
    // A callback must not reenter this API. try_borrow_mut turns reentrancy and
    // use from the wrong thread into an error instead of a RefCell panic.
    PLAYERS.with(|players| {
        let Ok(mut players) = players.try_borrow_mut() else {
            return -1;
        };
        let Some(player) = players.get_mut(&handle) else {
            return -1;
        };
        action(player);
        0
    })
}

/// # Safety
/// `callbacks` and `storage` point to valid C values. The callback context must
/// remain valid until destroy returns. All calls for a handle use its creating
/// thread. Callback data is borrowed only for that synchronous callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atv_player_create(
    callbacks: *const AtvCallbacks,
    storage: *const c_char,
) -> u64 {
    boundary(|| {
        if callbacks.is_null() {
            return 0;
        }
        let callbacks = unsafe { *callbacks };
        if callbacks.abi_version != 1
            || callbacks.reset.is_none()
            || callbacks.video.is_none()
            || callbacks.audio.is_none()
            || callbacks.drain.is_none()
        {
            return 0;
        }
        let Some(storage) = (unsafe { input(storage) }) else {
            return 0;
        };
        let Ok(guid) = atv_player::identity::load_or_create(Path::new(storage)) else {
            return 0;
        };
        let config = atv_auth::ResolverConfig {
            device_guid: Some(guid),
            user_agent: "ATV-Native/0.1 (Apple; Rust)".into(),
            device_platform: "Apple".into(),
            ..Default::default()
        };
        let Ok(source) = atv_auth::LiveResolver::new(config) else {
            return 0;
        };
        let handle = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        PLAYERS.with(|p| {
            p.borrow_mut()
                .insert(handle, Player::new(callbacks, Arc::new(source)))
        });
        handle
    })
}
/// # Safety
/// `pid` and `stream` are valid null-terminated UTF-8 identifiers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atv_player_select(
    handle: u64,
    pid: *const c_char,
    stream: *const c_char,
) -> i32 {
    control(|| {
        let (Some(pid), Some(stream)) = (unsafe { input(pid) }, unsafe { input(stream) }) else {
            return -1;
        };
        let mut selected = false;
        let found = with_player(handle, |p| {
            selected = p.select(pid.into(), stream.into()).is_ok();
        });
        if found == 0 && selected { 0 } else { -1 }
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn atv_player_tick(handle: u64) -> i32 {
    control(|| with_player(handle, Player::tick))
}
#[unsafe(no_mangle)]
pub extern "C" fn atv_player_pause(handle: u64) -> i32 {
    control(|| with_player(handle, Player::pause))
}
#[unsafe(no_mangle)]
pub extern "C" fn atv_player_resume(handle: u64) -> i32 {
    control(|| with_player(handle, Player::resume))
}
#[unsafe(no_mangle)]
pub extern "C" fn atv_player_destroy(handle: u64) -> i32 {
    control(|| {
        PLAYERS.with(|p| {
            let Ok(mut p) = p.try_borrow_mut() else {
                return -1;
            };
            if p.remove(&handle).is_some() { 0 } else { -1 }
        })
    })
}
/// # Safety
/// If non-null, `out` is writable for `capacity` bytes. Returns required capacity
/// including NUL. A too-small buffer is untouched; zero means invalid handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atv_player_status(
    handle: u64,
    out: *mut c_char,
    capacity: usize,
) -> usize {
    boundary(|| {
        let mut json = None;
        if with_player(handle, |p| json = serde_json::to_vec(p.status()).ok()) != 0 {
            return 0;
        }
        let Some(json) = json else {
            return 0;
        };
        unsafe { write_bytes(&json, out, capacity) }
    })
}
unsafe fn write_bytes(bytes: &[u8], out: *mut c_char, capacity: usize) -> usize {
    let size = bytes.len() + 1;
    if !out.is_null() && capacity >= size {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast(), bytes.len());
            *out.add(bytes.len()) = 0;
        }
    }
    size
}
/// # Safety
/// Same output buffer contract as atv_player_status. No player handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atv_channels_json(out: *mut c_char, capacity: usize) -> usize {
    unsafe {
        write_bytes(
            include_bytes!("../../atv-core/assets/channels.json"),
            out,
            capacity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_wrong_thread_handles_cannot_access_state() {
        assert_eq!(atv_player_tick(0), -1);
        assert_eq!(atv_player_destroy(0), -1);
        assert_eq!(unsafe { atv_player_status(0, std::ptr::null_mut(), 0) }, 0);
        assert_eq!(
            unsafe { atv_player_create(std::ptr::null(), std::ptr::null()) },
            0
        );
    }
    #[test]
    fn c_output_buffer_probe_never_partially_writes() {
        let required = unsafe { atv_channels_json(std::ptr::null_mut(), 0) };
        let mut small = [42i8; 4];
        assert_eq!(
            unsafe { atv_channels_json(small.as_mut_ptr(), 4) },
            required
        );
        assert_eq!(small, [42; 4]);
        let mut output = vec![0i8; required];
        unsafe { atv_channels_json(output.as_mut_ptr(), required) };
        let text = unsafe { CStr::from_ptr(output.as_ptr()) }.to_str().unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(text)
                .unwrap()
                .is_array()
        );
    }
    #[test]
    fn callback_backpressure_preserves_packet_for_retry() {
        #[derive(Default)]
        struct Context {
            attempts: Vec<(Vec<u8>, i64)>,
            resets: usize,
        }
        unsafe extern "C" fn reset(context: *mut c_void) -> i32 {
            unsafe {
                (*(context.cast::<Context>())).resets += 1;
            }
            0
        }
        unsafe extern "C" fn push(
            context: *mut c_void,
            bytes: *const u8,
            len: usize,
            pts: i64,
        ) -> i32 {
            let context = unsafe { &mut *context.cast::<Context>() };
            context.attempts.push((
                unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec(),
                pts,
            ));
            if context.attempts.len() == 1 { 1 } else { 0 }
        }
        unsafe extern "C" fn drain(_: *mut c_void, _: *mut AtvOutputStats) -> i32 {
            -1
        }
        let mut context = Context::default();
        let mut factory = AtvCallbacks {
            abi_version: 1,
            context: (&mut context as *mut Context).cast(),
            reset: Some(reset),
            video: Some(push),
            audio: Some(push),
            drain: Some(drain),
        };
        let mut output = factory.create().unwrap();
        let packet = [0, 0, 1, 0x65, 0xaa];
        assert!(matches!(
            output.push_video(&packet, -40_000),
            Err(OutputError::Backpressure)
        ));
        output.push_video(&packet, -40_000).unwrap();
        assert_eq!(
            context.attempts,
            vec![(packet.to_vec(), -40_000), (packet.to_vec(), -40_000)]
        );
        drop(output);
        assert_eq!(context.resets, 2);
    }

    #[test]
    fn handle_is_thread_owned_and_destroy_is_final() {
        unsafe extern "C" fn reset(_: *mut c_void) -> i32 {
            0
        }
        unsafe extern "C" fn push(_: *mut c_void, _: *const u8, _: usize, _: i64) -> i32 {
            0
        }
        unsafe extern "C" fn drain(_: *mut c_void, out: *mut AtvOutputStats) -> i32 {
            unsafe {
                *out = AtvOutputStats::default();
            }
            0
        }
        let directory = std::env::temp_dir().join(format!(
            "atv-apple-{}-{}",
            std::process::id(),
            NEXT.load(std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let storage = std::ffi::CString::new(directory.to_str().unwrap()).unwrap();
        let callbacks = AtvCallbacks {
            abi_version: 1,
            context: std::ptr::null_mut(),
            reset: Some(reset),
            video: Some(push),
            audio: Some(push),
            drain: Some(drain),
        };
        let handle = unsafe { atv_player_create(&callbacks, storage.as_ptr()) };
        assert_ne!(handle, 0);
        std::thread::spawn(move || {
            assert_eq!(atv_player_tick(handle), -1);
            assert_eq!(atv_player_destroy(handle), -1);
            assert_eq!(
                unsafe { atv_player_status(handle, std::ptr::null_mut(), 0) },
                0
            );
        })
        .join()
        .unwrap();
        assert_eq!(atv_player_pause(handle), 0);
        let size = unsafe { atv_player_status(handle, std::ptr::null_mut(), 0) };
        let mut bytes = vec![0i8; size];
        assert_eq!(
            unsafe { atv_player_status(handle, bytes.as_mut_ptr(), size) },
            size
        );
        let text = unsafe { CStr::from_ptr(bytes.as_ptr()) }.to_str().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text).unwrap()["state"],
            "paused"
        );
        assert_eq!(atv_player_destroy(handle), 0);
        assert_eq!(atv_player_destroy(handle), -1);
        assert_eq!(atv_player_tick(handle), -1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
