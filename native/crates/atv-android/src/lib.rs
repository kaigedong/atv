//! Android adapter: JNI, native window lifetime, MediaCodec and AAudio.
#[cfg(all(target_os = "android", not(feature = "output-only")))]
mod android;
#[cfg(target_os = "android")]
mod codec;
#[cfg(target_os = "android")]
pub mod output;

#[cfg(target_os = "android")]
mod legacy_api;
#[cfg(all(target_os = "android", not(feature = "legacy-android")))]
mod ndk_api;
