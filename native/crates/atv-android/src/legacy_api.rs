//! API 19 system media adapter. All scheduling/parsing stays in Rust.
//! The private compatibility names let the same bounded queue implementation use
//! Java MediaCodec/AudioTrack on KitKat and NDK MediaCodec/AAudio on newer systems.
#![allow(non_snake_case, unsafe_op_in_unsafe_fn)]
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JShortArray, JValue},
};
use std::{ffi::c_void, sync::OnceLock};

static VM: OnceLock<JavaVM> = OnceLock::new();
pub fn initialize(env: &JNIEnv) -> jni::errors::Result<()> {
    if VM.get().is_none() {
        let _ = VM.set(env.get_java_vm()?);
    }
    Ok(())
}
fn java<T>(action: impl FnOnce(&mut JNIEnv) -> jni::errors::Result<T>) -> Option<T> {
    let mut env = VM.get()?.attach_current_thread_permanently().ok()?;
    let result = env.with_local_frame(64, action);
    if result.is_err() {
        // Only system media calls enter this helper; no resource URLs, tickets
        // or authentication payloads are passed to any Java method here.
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.ok()
}
#[cfg(feature = "legacy-android")]
mod media {
    use super::*;
    use jni::objects::{JByteBuffer, JObject, JObjectArray};
    use std::ffi::{CStr, c_char};
    use std::ptr;
    unsafe fn string<'local>(
        env: &mut JNIEnv<'local>,
        value: *const c_char,
    ) -> jni::errors::Result<JObject<'local>> {
        Ok(env
            .new_string(CStr::from_ptr(value).to_string_lossy())?
            .into())
    }
    pub(crate) struct AMediaFormat {
        object: GlobalRef,
        valid: bool,
    }
    pub(crate) struct AMediaCodec {
        object: GlobalRef,
        info: GlobalRef,
        input: Vec<GlobalRef>,
        output: Vec<GlobalRef>,
    }
    #[derive(Clone, Copy, Default)]
    pub(crate) struct BufferInfo {
        pub offset: i32,
        pub size: i32,
        pub pts: i64,
        pub flags: u32,
    }

    pub(crate) unsafe fn AMediaFormat_new() -> *mut AMediaFormat {
        java(|env| {
            let obj = env.new_object("android/media/MediaFormat", "()V", &[])?;
            Ok(Box::into_raw(Box::new(AMediaFormat {
                object: env.new_global_ref(obj)?,
                valid: true,
            })))
        })
        .unwrap_or(ptr::null_mut())
    }
    pub(crate) unsafe fn AMediaFormat_delete(value: *mut AMediaFormat) -> i32 {
        drop(Box::from_raw(value));
        0
    }
    pub(crate) unsafe fn AMediaFormat_setInt32(
        f: *mut AMediaFormat,
        key: *const c_char,
        value: i32,
    ) {
        if java(|env| {
            let k = string(env, key)?;
            env.call_method(
                (*f).object.as_obj(),
                "setInteger",
                "(Ljava/lang/String;I)V",
                &[JValue::Object(&k), JValue::Int(value)],
            )?;
            Ok(())
        })
        .is_none()
        {
            (*f).valid = false;
        }
    }
    pub(crate) unsafe fn AMediaFormat_setString(
        f: *mut AMediaFormat,
        key: *const c_char,
        value: *const c_char,
    ) {
        if java(|env| {
            let k = string(env, key)?;
            let v = string(env, value)?;
            env.call_method(
                (*f).object.as_obj(),
                "setString",
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[JValue::Object(&k), JValue::Object(&v)],
            )?;
            Ok(())
        })
        .is_none()
        {
            (*f).valid = false;
        }
    }
    pub(crate) unsafe fn AMediaFormat_setBuffer(
        f: *mut AMediaFormat,
        key: *const c_char,
        value: *const c_void,
        size: usize,
    ) {
        if java(|env| {
            let k = string(env, key)?;
            let buffer = env
                .call_static_method(
                    "java/nio/ByteBuffer",
                    "allocateDirect",
                    "(I)Ljava/nio/ByteBuffer;",
                    &[JValue::Int(size as i32)],
                )?
                .l()?;
            let b = JByteBuffer::from(buffer);
            let dst = env.get_direct_buffer_address(&b)?;
            ptr::copy_nonoverlapping(value.cast::<u8>(), dst, size);
            env.call_method(
                (*f).object.as_obj(),
                "setByteBuffer",
                "(Ljava/lang/String;Ljava/nio/ByteBuffer;)V",
                &[JValue::Object(&k), JValue::Object(b.as_ref())],
            )?;
            Ok(())
        })
        .is_none()
        {
            (*f).valid = false;
        }
    }
    pub(crate) unsafe fn AMediaFormat_getInt32(
        f: *mut AMediaFormat,
        key: *const c_char,
        value: *mut i32,
    ) -> bool {
        java(|env| {
            let k = string(env, key)?;
            if !env
                .call_method(
                    (*f).object.as_obj(),
                    "containsKey",
                    "(Ljava/lang/String;)Z",
                    &[JValue::Object(&k)],
                )?
                .z()?
            {
                return Ok(false);
            }
            let result = env
                .call_method(
                    (*f).object.as_obj(),
                    "getInteger",
                    "(Ljava/lang/String;)I",
                    &[JValue::Object(&k)],
                )?
                .i()?;
            *value = result;
            Ok(true)
        })
        .unwrap_or(false)
    }
    pub(crate) unsafe fn AMediaCodec_createDecoderByType(mime: *const c_char) -> *mut AMediaCodec {
        java(|env| {
            let mime = string(env, mime)?;
            let object = env
                .call_static_method(
                    "android/media/MediaCodec",
                    "createDecoderByType",
                    "(Ljava/lang/String;)Landroid/media/MediaCodec;",
                    &[JValue::Object(&mime)],
                )?
                .l()?;
            let info = env.new_object("android/media/MediaCodec$BufferInfo", "()V", &[])?;
            Ok(Box::into_raw(Box::new(AMediaCodec {
                object: env.new_global_ref(object)?,
                info: env.new_global_ref(info)?,
                input: Vec::new(),
                output: Vec::new(),
            })))
        })
        .unwrap_or(ptr::null_mut())
    }
    pub(crate) unsafe fn AMediaCodec_configure(
        c: *mut AMediaCodec,
        f: *const AMediaFormat,
        window: *mut c_void,
        _: *mut c_void,
        flags: u32,
    ) -> i32 {
        if !(*f).valid {
            return -1;
        }
        java(|env| {
            let null = JObject::null();
            let surface = if window.is_null() {
                &null
            } else {
                (&*window.cast::<GlobalRef>()).as_obj()
            };
            env.call_method(
                (*c).object.as_obj(),
                "configure",
                "(Landroid/media/MediaFormat;Landroid/view/Surface;Landroid/media/MediaCrypto;I)V",
                &[
                    JValue::Object((*f).object.as_obj()),
                    JValue::Object(surface),
                    JValue::Object(&null),
                    JValue::Int(flags as i32),
                ],
            )?;
            Ok(0)
        })
        .unwrap_or(-1)
    }
    fn buffers(
        env: &mut JNIEnv,
        object: &GlobalRef,
        method: &str,
    ) -> jni::errors::Result<Vec<GlobalRef>> {
        let array = JObjectArray::from(
            env.call_method(object.as_obj(), method, "()[Ljava/nio/ByteBuffer;", &[])?
                .l()?,
        );
        let mut result = Vec::new();
        for i in 0..env.get_array_length(&array)? {
            let b = env.get_object_array_element(&array, i)?;
            result.push(env.new_global_ref(&b)?);
            env.delete_local_ref(b)?;
        }
        Ok(result)
    }
    pub(crate) unsafe fn AMediaCodec_start(c: *mut AMediaCodec) -> i32 {
        java(|env| {
            env.call_method((*c).object.as_obj(), "start", "()V", &[])?;
            (*c).input = buffers(env, &(*c).object, "getInputBuffers")?;
            (*c).output = buffers(env, &(*c).object, "getOutputBuffers")?;
            Ok(0)
        })
        .unwrap_or(-1)
    }
    pub(crate) unsafe fn AMediaCodec_stop(c: *mut AMediaCodec) -> i32 {
        java(|env| {
            env.call_method((*c).object.as_obj(), "stop", "()V", &[])?;
            Ok(0)
        })
        .unwrap_or(-1)
    }
    pub(crate) unsafe fn AMediaCodec_delete(c: *mut AMediaCodec) -> i32 {
        let result = java(|env| {
            env.call_method((*c).object.as_obj(), "release", "()V", &[])?;
            Ok(0)
        })
        .unwrap_or(-1);
        drop(Box::from_raw(c));
        result
    }
    pub(crate) unsafe fn AMediaCodec_dequeueInputBuffer(
        c: *mut AMediaCodec,
        timeout: i64,
    ) -> isize {
        java(|env| {
            env.call_method(
                (*c).object.as_obj(),
                "dequeueInputBuffer",
                "(J)I",
                &[JValue::Long(timeout)],
            )?
            .i()
        })
        .map_or(-1000, |i| i as isize)
    }
    unsafe fn buffer(c: *mut AMediaCodec, index: usize, size: *mut usize, input: bool) -> *mut u8 {
        java(|env| {
            let items = if input { &(*c).input } else { &(*c).output };
            let Some(item) = items.get(index) else {
                return Ok(ptr::null_mut());
            };
            let obj = JByteBuffer::from(env.new_local_ref(item.as_obj())?);
            *size = env.get_direct_buffer_capacity(&obj)?;
            env.get_direct_buffer_address(&obj)
        })
        .unwrap_or(ptr::null_mut())
    }
    pub(crate) unsafe fn AMediaCodec_getInputBuffer(
        c: *mut AMediaCodec,
        index: usize,
        size: *mut usize,
    ) -> *mut u8 {
        buffer(c, index, size, true)
    }
    pub(crate) unsafe fn AMediaCodec_getOutputBuffer(
        c: *mut AMediaCodec,
        index: usize,
        size: *mut usize,
    ) -> *mut u8 {
        buffer(c, index, size, false)
    }
    pub(crate) unsafe fn AMediaCodec_queueInputBuffer(
        c: *mut AMediaCodec,
        index: usize,
        offset: isize,
        size: usize,
        time: u64,
        flags: u32,
    ) -> i32 {
        java(|env| {
            env.call_method(
                (*c).object.as_obj(),
                "queueInputBuffer",
                "(IIIJI)V",
                &[
                    JValue::Int(index as i32),
                    JValue::Int(offset as i32),
                    JValue::Int(size as i32),
                    JValue::Long(time as i64),
                    JValue::Int(flags as i32),
                ],
            )?;
            Ok(0)
        })
        .unwrap_or(-1)
    }
    pub(crate) unsafe fn AMediaCodec_dequeueOutputBuffer(
        c: *mut AMediaCodec,
        info: *mut BufferInfo,
        timeout: i64,
    ) -> isize {
        java(|env| {
            let index = env
                .call_method(
                    (*c).object.as_obj(),
                    "dequeueOutputBuffer",
                    "(Landroid/media/MediaCodec$BufferInfo;J)I",
                    &[JValue::Object((*c).info.as_obj()), JValue::Long(timeout)],
                )?
                .i()?;
            if index >= 0 {
                let o = (*c).info.as_obj();
                *info = BufferInfo {
                    offset: env.get_field(o, "offset", "I")?.i()?,
                    size: env.get_field(o, "size", "I")?.i()?,
                    pts: env.get_field(o, "presentationTimeUs", "J")?.j()?,
                    flags: env.get_field(o, "flags", "I")?.i()? as u32,
                };
            }
            if index == -3 {
                (*c).output = buffers(env, &(*c).object, "getOutputBuffers")?;
            }
            Ok(index as isize)
        })
        .unwrap_or(-1000)
    }
    pub(crate) unsafe fn AMediaCodec_releaseOutputBuffer(
        c: *mut AMediaCodec,
        index: usize,
        render: bool,
    ) -> i32 {
        java(|env| {
            env.call_method(
                (*c).object.as_obj(),
                "releaseOutputBuffer",
                "(IZ)V",
                &[JValue::Int(index as i32), JValue::Bool(render as u8)],
            )?;
            Ok(0)
        })
        .unwrap_or(-1)
    }
    pub(crate) unsafe fn AMediaCodec_releaseOutputBufferAtTime(
        c: *mut AMediaCodec,
        index: usize,
        time: i64,
    ) -> i32 {
        // KitKat has no timestamped release. Delay on the render owner only; no UI sleep.
        let Ok(now) = crate::codec::now_ns() else {
            return -1;
        };
        let delta = time.saturating_sub(now);
        if delta > 0 {
            std::thread::sleep(std::time::Duration::from_nanos(delta.min(40_000_000) as u64));
        }
        AMediaCodec_releaseOutputBuffer(c, index, true)
    }
    pub(crate) unsafe fn AMediaCodec_getOutputFormat(c: *mut AMediaCodec) -> *mut AMediaFormat {
        java(|env| {
            let o = env
                .call_method(
                    (*c).object.as_obj(),
                    "getOutputFormat",
                    "()Landroid/media/MediaFormat;",
                    &[],
                )?
                .l()?;
            Ok(Box::into_raw(Box::new(AMediaFormat {
                object: env.new_global_ref(o)?,
                valid: true,
            })))
        })
        .unwrap_or(ptr::null_mut())
    }
}
#[cfg(feature = "legacy-android")]
pub(crate) use media::*;
pub(crate) struct AAudioStreamBuilder {
    rate: i32,
    channels: i32,
    format: i32,
}
pub(crate) struct AAudioStream {
    object: GlobalRef,
    scratch: GlobalRef,
    rate: i32,
    channels: i32,
    capacity_frames: u64,
    written_frames: u64,
    previous_head: u32,
    head_wrap: u64,
}
pub(crate) unsafe fn AAudio_createStreamBuilder(out: *mut *mut AAudioStreamBuilder) -> i32 {
    *out = Box::into_raw(Box::new(AAudioStreamBuilder {
        rate: 48000,
        channels: 2,
        format: 1,
    }));
    0
}
pub(crate) unsafe fn AAudioStreamBuilder_delete(b: *mut AAudioStreamBuilder) -> i32 {
    drop(Box::from_raw(b));
    0
}
pub(crate) unsafe fn AAudioStreamBuilder_setSampleRate(b: *mut AAudioStreamBuilder, v: i32) {
    (*b).rate = v;
}
pub(crate) unsafe fn AAudioStreamBuilder_setChannelCount(b: *mut AAudioStreamBuilder, v: i32) {
    (*b).channels = v;
}
pub(crate) unsafe fn AAudioStreamBuilder_setFormat(b: *mut AAudioStreamBuilder, v: i32) {
    (*b).format = v;
}
pub(crate) unsafe fn AAudioStreamBuilder_setDirection(_: *mut AAudioStreamBuilder, _: i32) {}
pub(crate) unsafe fn AAudioStreamBuilder_setSharingMode(_: *mut AAudioStreamBuilder, _: i32) {}
pub(crate) unsafe fn AAudioStreamBuilder_setPerformanceMode(_: *mut AAudioStreamBuilder, _: i32) {}
pub(crate) unsafe fn AAudioStreamBuilder_openStream(
    b: *mut AAudioStreamBuilder,
    out: *mut *mut AAudioStream,
) -> i32 {
    if (*b).format != 1 || !matches!((*b).channels, 1 | 2) {
        return -1;
    }
    java(|env| {
        let mask = if (*b).channels == 1 { 4 } else { 12 };
        let min = env
            .call_static_method(
                "android/media/AudioTrack",
                "getMinBufferSize",
                "(III)I",
                &[JValue::Int((*b).rate), JValue::Int(mask), JValue::Int(2)],
            )?
            .i()?;
        if min <= 0 {
            return Ok(-1);
        }
        let buffer_bytes = min.max((*b).rate * (*b).channels * 2 / 10);
        let obj = env.new_object(
            "android/media/AudioTrack",
            "(IIIIII)V",
            &[
                JValue::Int(3),
                JValue::Int((*b).rate),
                JValue::Int(mask),
                JValue::Int(2),
                JValue::Int(buffer_bytes),
                JValue::Int(1),
            ],
        )?;
        if env.call_method(&obj, "getState", "()I", &[])?.i()? != 1 {
            env.call_method(&obj, "release", "()V", &[])?;
            return Ok(-1);
        }
        let scratch = env.new_short_array(1024)?;
        *out = Box::into_raw(Box::new(AAudioStream {
            object: env.new_global_ref(obj)?,
            scratch: env.new_global_ref(scratch)?,
            rate: (*b).rate,
            channels: (*b).channels,
            capacity_frames: (buffer_bytes / ((*b).channels * 2)) as u64,
            written_frames: 0,
            previous_head: 0,
            head_wrap: 0,
        }));
        Ok(0)
    })
    .unwrap_or(-1)
}
pub(crate) unsafe fn AAudioStream_getSampleRate(s: *mut AAudioStream) -> i32 {
    (*s).rate
}
pub(crate) unsafe fn AAudioStream_getChannelCount(s: *mut AAudioStream) -> i32 {
    (*s).channels
}
pub(crate) unsafe fn AAudioStream_getFormat(_: *mut AAudioStream) -> i32 {
    1
}
pub(crate) unsafe fn AAudioStream_requestStart(s: *mut AAudioStream) -> i32 {
    java(|env| {
        env.call_method((*s).object.as_obj(), "play", "()V", &[])?;
        Ok(0)
    })
    .unwrap_or(-1)
}
pub(crate) unsafe fn AAudioStream_requestStop(s: *mut AAudioStream) -> i32 {
    java(|env| {
        env.call_method((*s).object.as_obj(), "pause", "()V", &[])?;
        env.call_method((*s).object.as_obj(), "flush", "()V", &[])?;
        Ok(0)
    })
    .unwrap_or(-1)
}
pub(crate) unsafe fn AAudioStream_write(
    s: *mut AAudioStream,
    data: *const c_void,
    frames: i32,
    _: i64,
) -> i32 {
    if !(0..=512).contains(&frames) {
        return -1;
    }
    java(|env| {
        // API 19 only has blocking write. Bound each write to already-free
        // frames so it cannot block the video owner waiting for audio playback.
        let head = env
            .call_method((*s).object.as_obj(), "getPlaybackHeadPosition", "()I", &[])?
            .i()? as u32;
        if head < (*s).previous_head {
            (*s).head_wrap += 1u64 << 32;
        }
        (*s).previous_head = head;
        let played = (*s).head_wrap + u64::from(head);
        let queued = (*s).written_frames.saturating_sub(played);
        let frames =
            u64::from(frames as u32).min((*s).capacity_frames.saturating_sub(queued)) as i32;
        if frames == 0 {
            return Ok(0);
        }
        let samples = frames * (*s).channels;
        let array = JShortArray::from(env.new_local_ref((*s).scratch.as_obj())?);
        let bytes = std::slice::from_raw_parts(data.cast::<u8>(), samples as usize * 2);
        let mut pcm = [0i16; 1024];
        for (output, bytes) in pcm.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *output = i16::from_le_bytes([bytes[0], bytes[1]]);
        }
        env.set_short_array_region(&array, 0, &pcm[..samples as usize])?;
        let written = env
            .call_method(
                (*s).object.as_obj(),
                "write",
                "([SII)I",
                &[
                    JValue::Object(array.as_ref()),
                    JValue::Int(0),
                    JValue::Int(samples),
                ],
            )?
            .i()?;
        Ok(if written < 0 {
            written
        } else if written > samples || written % (*s).channels != 0 {
            -1
        } else {
            let frames = written / (*s).channels;
            (*s).written_frames += frames as u64;
            frames
        })
    })
    .unwrap_or(-1)
}
pub(crate) unsafe fn AAudioStream_close(s: *mut AAudioStream) -> i32 {
    let result = java(|env| {
        env.call_method((*s).object.as_obj(), "release", "()V", &[])?;
        Ok(0)
    })
    .unwrap_or(-1);
    drop(Box::from_raw(s));
    result
}

// mio uses epoll_create1, which bionic exposes from API 21. KitKat has the
// equivalent epoll_create + FD_CLOEXEC operations. Never exec a child process in
// the player; this also avoids a fork/exec race during these two calls.
#[cfg(feature = "legacy-android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn epoll_create1(flags: i32) -> i32 {
    unsafe extern "C" {
        fn epoll_create(size: i32) -> i32;
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn close(fd: i32) -> i32;
        fn __errno() -> *mut i32;
    }
    if flags & !0x80000 != 0 {
        *__errno() = 22;
        return -1;
    }
    let fd = epoll_create(1);
    if fd >= 0 && flags != 0 && fcntl(fd, 2, 1) < 0 {
        let error = *__errno();
        close(fd);
        *__errno() = error;
        return -1;
    }
    fd
}

// 32-bit ARM bionic only exports this diagnostic API from level 21. Preserve
// native behavior when it exists; KitKat cannot supply a loaded-module list to
// Rust's optional backtrace symbolizer. Playback and unwinding do not use this
// fallback (ARM EHABI uses __gnu_Unwind_Find_exidx from libdl).
#[cfg(all(feature = "legacy-android", target_arch = "arm"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dl_iterate_phdr(
    callback: unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> i32,
    data: *mut c_void,
) -> i32 {
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
    }
    let symbol = dlsym((-2isize) as *mut c_void, c"dl_iterate_phdr".as_ptr());
    if symbol.is_null() {
        return 0;
    }
    let function: unsafe extern "C" fn(
        unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> i32,
        *mut c_void,
    ) -> i32 = std::mem::transmute(symbol);
    function(callback, data)
}

// Keep cryptographic randomness on the maintained getrandom 0.2 Android
// syscall + entropy-ready /dev/urandom implementation. Unlike 0.3's default
// backend, it never globally dlsyms getrandom on old bionic loaders.
#[cfg(feature = "legacy-android")]
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom_custom::Error> {
    let bytes = unsafe { std::slice::from_raw_parts_mut(dest, len) };
    getrandom_legacy::getrandom(bytes).map_err(|_| getrandom_custom::Error::new_custom(1))
}
