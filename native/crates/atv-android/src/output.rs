//! Android window ownership never crosses into the shared player API.
use crate::codec::NativeOutput;
use atv_media::{MediaOutput, OutputError, OutputFactory};
use std::ffi::c_void;
use std::sync::Arc;

#[cfg(not(feature = "legacy-android"))]
#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_release(window: *mut c_void);
}

/// Owns a Surface reference shared with every output created by this factory.
pub struct Window(Arc<WindowReference>);
struct WindowReference(*mut c_void);
// The reference itself is immutable and Android permits cross-thread Surface
// lifetime management. Actual decoding stays on the output's owning thread.
unsafe impl Send for WindowReference {}
unsafe impl Sync for WindowReference {}
impl Drop for WindowReference {
    fn drop(&mut self) {
        #[cfg(not(feature = "legacy-android"))]
        unsafe {
            ANativeWindow_release(self.0)
        };
        #[cfg(feature = "legacy-android")]
        unsafe {
            drop(Box::from_raw(self.0.cast::<jni::objects::GlobalRef>()))
        };
    }
}
impl Window {
    /// Acquire an output Surface reference using the selected system adapter.
    pub fn from_surface(
        env: &jni::JNIEnv,
        surface: &jni::objects::JObject,
    ) -> Result<Self, atv_media::OutputError> {
        crate::legacy_api::initialize(env)
            .map_err(|_| atv_media::OutputError::InvalidData("Java VM unavailable"))?;
        if surface.is_null() {
            return Err(atv_media::OutputError::InvalidData("null Surface"));
        }
        #[cfg(feature = "legacy-android")]
        let pointer = Box::into_raw(Box::new(env.new_global_ref(surface).map_err(|_| {
            atv_media::OutputError::InvalidData("Surface reference unavailable")
        })?))
        .cast::<c_void>();
        #[cfg(not(feature = "legacy-android"))]
        let pointer = {
            unsafe extern "C" {
                fn ANativeWindow_fromSurface(
                    env: *mut jni::sys::JNIEnv,
                    surface: jni::sys::jobject,
                ) -> *mut c_void;
            }
            unsafe { ANativeWindow_fromSurface(env.get_native_interface(), surface.as_raw()) }
        };
        if pointer.is_null() {
            return Err(atv_media::OutputError::InvalidData(
                "Surface window unavailable",
            ));
        }
        Ok(Self(Arc::new(WindowReference(pointer))))
    }
}
impl OutputFactory for Window {
    fn create(&mut self) -> Result<Box<dyn MediaOutput>, OutputError> {
        Ok(Box::new(OwnedOutput {
            output: unsafe { NativeOutput::new(self.0.0) }?,
            _window: self.0.clone(),
        }))
    }
}
// Field order guarantees codec/audio teardown precedes releasing the Surface.
struct OwnedOutput {
    output: NativeOutput,
    _window: Arc<WindowReference>,
}
impl MediaOutput for OwnedOutput {
    fn push_video(&mut self, bytes: &[u8], pts: i64) -> Result<(), OutputError> {
        self.output.push_video(bytes, pts)
    }
    fn push_audio(&mut self, bytes: &[u8], pts: i64) -> Result<(), OutputError> {
        self.output.push_audio(bytes, pts)
    }
    fn push_video_owned(&mut self, bytes: &mut Vec<u8>, pts: i64) -> Result<(), OutputError> {
        self.output.push_video_owned(bytes, pts)
    }
    fn buffered_capacity(&self) -> usize {
        self.output.buffered_capacity()
    }
    fn drain(&mut self) -> Result<atv_media::OutputStats, OutputError> {
        self.output.drain()
    }
}
