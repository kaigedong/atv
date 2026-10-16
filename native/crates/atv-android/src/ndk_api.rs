//! API 21+ NDK codecs; load AAudio optionally on API 26+; AudioTrack fallback.
#![allow(non_snake_case)]
use std::ffi::{c_char, c_void};

#[repr(C)]
pub(crate) struct AMediaCodec {
    _private: [u8; 0],
}
#[repr(C)]
pub(crate) struct AMediaFormat {
    _private: [u8; 0],
}
#[repr(C)]
pub(crate) struct AAudioStreamBuilder {
    _private: [u8; 0],
}
#[repr(C)]
pub(crate) struct AAudioStream {
    _private: [u8; 0],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct BufferInfo {
    pub offset: i32,
    pub size: i32,
    pub pts: i64,
    pub flags: u32,
}
#[link(name = "mediandk")]
unsafe extern "C" {
    pub(crate) fn AMediaCodec_createDecoderByType(mime: *const c_char) -> *mut AMediaCodec;
    pub(crate) fn AMediaCodec_delete(codec: *mut AMediaCodec) -> i32;
    pub(crate) fn AMediaCodec_configure(
        codec: *mut AMediaCodec,
        format: *const AMediaFormat,
        window: *mut c_void,
        crypto: *mut c_void,
        flags: u32,
    ) -> i32;
    pub(crate) fn AMediaCodec_start(codec: *mut AMediaCodec) -> i32;
    pub(crate) fn AMediaCodec_stop(codec: *mut AMediaCodec) -> i32;
    pub(crate) fn AMediaCodec_dequeueInputBuffer(codec: *mut AMediaCodec, timeout_us: i64)
    -> isize;
    pub(crate) fn AMediaCodec_getInputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        size: *mut usize,
    ) -> *mut u8;
    pub(crate) fn AMediaCodec_queueInputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        offset: isize,
        size: usize,
        time: u64,
        flags: u32,
    ) -> i32;
    pub(crate) fn AMediaCodec_dequeueOutputBuffer(
        codec: *mut AMediaCodec,
        info: *mut BufferInfo,
        timeout_us: i64,
    ) -> isize;
    pub(crate) fn AMediaCodec_getOutputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        size: *mut usize,
    ) -> *mut u8;
    pub(crate) fn AMediaCodec_releaseOutputBuffer(
        codec: *mut AMediaCodec,
        index: usize,
        render: bool,
    ) -> i32;
    pub(crate) fn AMediaCodec_releaseOutputBufferAtTime(
        codec: *mut AMediaCodec,
        index: usize,
        timestamp_ns: i64,
    ) -> i32;
    pub(crate) fn AMediaCodec_getOutputFormat(codec: *mut AMediaCodec) -> *mut AMediaFormat;
    pub(crate) fn AMediaFormat_new() -> *mut AMediaFormat;
    pub(crate) fn AMediaFormat_delete(format: *mut AMediaFormat) -> i32;
    pub(crate) fn AMediaFormat_setString(
        format: *mut AMediaFormat,
        key: *const c_char,
        value: *const c_char,
    );
    pub(crate) fn AMediaFormat_setInt32(format: *mut AMediaFormat, key: *const c_char, value: i32);
    pub(crate) fn AMediaFormat_setBuffer(
        format: *mut AMediaFormat,
        key: *const c_char,
        value: *const c_void,
        size: usize,
    );
    pub(crate) fn AMediaFormat_getInt32(
        format: *mut AMediaFormat,
        key: *const c_char,
        value: *mut i32,
    ) -> bool;
}
#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}
fn audio_library() -> Option<*mut c_void> {
    static LIB: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let library = dlopen(c"libaaudio.so".as_ptr(), 2);
        if library.is_null() {
            return None;
        }
        if dlsym(library, c"AAudio_createStreamBuilder".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_delete".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setDirection".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setFormat".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setSampleRate".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setChannelCount".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setSharingMode".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_setPerformanceMode".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStreamBuilder_openStream".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_getSampleRate".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_getChannelCount".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_getFormat".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_requestStart".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_requestStop".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_write".as_ptr()).is_null() {
            return None;
        }
        if dlsym(library, c"AAudioStream_close".as_ptr()).is_null() {
            return None;
        }
        Some(library as usize)
    })
    .map(|p| p as *mut c_void)
}
pub(crate) unsafe fn AAudio_createStreamBuilder(builder: *mut *mut AAudioStreamBuilder) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut *mut AAudioStreamBuilder) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudio_createStreamBuilder".as_ptr()));
            function(builder)
        } else {
            crate::legacy_api::AAudio_createStreamBuilder(builder.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_delete(builder: *mut AAudioStreamBuilder) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_delete".as_ptr()));
            function(builder)
        } else {
            crate::legacy_api::AAudioStreamBuilder_delete(builder.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setDirection(
    builder: *mut AAudioStreamBuilder,
    value: i32,
) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_setDirection".as_ptr()));
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setDirection(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setFormat(builder: *mut AAudioStreamBuilder, value: i32) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_setFormat".as_ptr()));
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setFormat(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setSampleRate(
    builder: *mut AAudioStreamBuilder,
    value: i32,
) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_setSampleRate".as_ptr()));
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setSampleRate(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setChannelCount(
    builder: *mut AAudioStreamBuilder,
    value: i32,
) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_setChannelCount".as_ptr()));
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setChannelCount(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setSharingMode(
    builder: *mut AAudioStreamBuilder,
    value: i32,
) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) =
                std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_setSharingMode".as_ptr()));
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setSharingMode(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_setPerformanceMode(
    builder: *mut AAudioStreamBuilder,
    value: i32,
) {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStreamBuilder, i32) = std::mem::transmute(
                dlsym(lib, c"AAudioStreamBuilder_setPerformanceMode".as_ptr()),
            );
            function(builder, value)
        } else {
            crate::legacy_api::AAudioStreamBuilder_setPerformanceMode(builder.cast(), value)
        }
    }
}
pub(crate) unsafe fn AAudioStreamBuilder_openStream(
    builder: *mut AAudioStreamBuilder,
    stream: *mut *mut AAudioStream,
) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(
                *mut AAudioStreamBuilder,
                *mut *mut AAudioStream,
            ) -> i32 = std::mem::transmute(dlsym(lib, c"AAudioStreamBuilder_openStream".as_ptr()));
            function(builder, stream)
        } else {
            crate::legacy_api::AAudioStreamBuilder_openStream(builder.cast(), stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_getSampleRate(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_getSampleRate".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_getSampleRate(stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_getChannelCount(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_getChannelCount".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_getChannelCount(stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_getFormat(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_getFormat".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_getFormat(stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_requestStart(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_requestStart".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_requestStart(stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_requestStop(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_requestStop".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_requestStop(stream.cast())
        }
    }
}
pub(crate) unsafe fn AAudioStream_write(
    stream: *mut AAudioStream,
    data: *const c_void,
    frames: i32,
    timeout_ns: i64,
) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream, *const c_void, i32, i64) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_write".as_ptr()));
            function(stream, data, frames, timeout_ns)
        } else {
            crate::legacy_api::AAudioStream_write(stream.cast(), data, frames, timeout_ns)
        }
    }
}
pub(crate) unsafe fn AAudioStream_close(stream: *mut AAudioStream) -> i32 {
    unsafe {
        if let Some(lib) = audio_library() {
            let function: unsafe extern "C" fn(*mut AAudioStream) -> i32 =
                std::mem::transmute(dlsym(lib, c"AAudioStream_close".as_ptr()));
            function(stream)
        } else {
            crate::legacy_api::AAudioStream_close(stream.cast())
        }
    }
}
