> 2026-10-16 更新：下文 API 26／双 ABI 描述保留为早期验收记录。当前默认构建为 API 19／四 ABI，32 位使用 NDK 25.2.9519653，64 位使用 NDK 29.0.14206865／API 21，并按需加载 AAudio。最新兼容性与实测状态见 [android-kitkat.md](android-kitkat.md)。

# Rust Android host and build

The current `main` uses the repository root Gradle project and `app/` native host.
The former `native-android/` project was migrated into that default location; the
legacy WebView application/build scripts were removed. Both old feature branches
are merged. See [main-native-build.md](main-native-build.md) for the migration audit.
The historical WebView baseline is tagged `baseline/webview-9f20444`.

On `feature/multiplatform`, shared playback/identity/session policy now lives in
`atv-player`, and portable parsers/output contracts in `atv-media`. The Android
crate contains only JNI, native-window ownership and NDK output. See
[multiplatform.md](multiplatform.md) for the current architecture and new artifact
evidence; the long-run results below retain their original APK hashes.

- Gradle project: repository root (`app/`)
- Application ID: `cn.bobo.atv.nativeplayer`
- Rust workspace: `native/`, Android crate `atv-android`
- Rust shared library: `libatv_native.so`
- Android minimum API: 26; compile/target API: 36
- ABIs: `arm64-v8a`, `x86_64`

The Java host only owns native Android widgets, the `SurfaceView`, remote
input, and Android lifecycle/JNI glue. It has no WebView, JavaScript engine,
WASM runtime, Media3 player, Java HTTP client, or Java media decoder.
The bundled JSON channel list supplies UI metadata; stream authorization,
network requests, media handling and playback state belong to Rust.

## JNI contract

Class: `cn.bobo.atv.nativeplayer.NativeBridge`.

```java
static native long create(Surface surface, int sampleRate, String storageDirectory);
static native void select(long handle, String pid, String streamId);
static native void pause(long handle);
static native void resume(long handle);
static native void destroy(long handle);
static native String status(long handle);
```

`create` returns 0 on failure. Control methods must return quickly and send
work to Rust-owned workers. The status method returns a short JSON snapshot:

`storageDirectory` is Android's private `getFilesDir()` path. Rust persists
the normal client GUID there and shares a resolver across sessions; recreating
a Surface or restarting playback must not reset an identity or preview allowance.

```json
{
  "state": "loading",
  "pid": "600001859",
  "positionMs": 0,
  "videoWidth": 0,
  "videoHeight": 0,
  "videoFrames": 0,
  "audioFrames": 0,
  "errorCode": "",
  "message": ""
}
```

States are `idle`, `loading`, `playing`, `paused`, `error`. `playing` must be
based on actual decoded/rendered video and successful native audio output,
never a queued compressed packet. Messages must not contain signed URLs,
tokens, tickets, or request payloads. Android polls the snapshot every 500 ms
while resumed.

Rust acquires the native window with `ANativeWindow_fromSurface` and retains
it until the worker and codecs have stopped. `SurfaceDestroyed` destroys the
session. A new Surface creates a fresh session and reselects the remembered
channel. Pause/resume and destroy must cancel or bound outstanding work.

## Toolchain

This machine has the official SDK under
`${HOME}/.local/share/android-sdk`, including fixed NDK
**29.0.14206865**. Rust lives under `${HOME}/.local/share/atv-rust`.
JDK 21 and Gradle Wrapper 8.13 are reused without changing the baseline.

The build script selects the NDK Clang API 26 linker for each Rust target,
sets the corresponding C/C++/archiver tools, and requests 16 KiB ELF segment
alignment. See the official [NDK toolchain documentation](https://developer.android.com/ndk/guides/other_build_systems).

```bash
export CARGO_HOME=${HOME}/.local/share/atv-rust/cargo
export RUSTUP_HOME=${HOME}/.local/share/atv-rust/rustup
export PATH="$CARGO_HOME/bin:$PATH"
rustup target add aarch64-linux-android x86_64-linux-android
./scripts/native-android-build.sh
```

This builds Rust first, verifies all six static JNI exports, copies the
shared libraries into `app/build/generated/rustJniLibs`, and runs
`assembleDebug lintDebug`. The output is
`app/build/outputs/apk/debug/app-debug.apk`. Set
`ATV_ANDROID_ABIS=x86_64` for an emulator-only package. Standard environment
variables can override toolchain paths; `ATV_ANDROID_ABIS` accepts the two
supported ABI names separated by a comma.

## Historical runtime evidence and current build boundary

The integrated JNI/session, HLS acquisition and native output code has built
for both supported ABIs; the original workspace's 45 ordinary tests passed
(50 after the portable engine split). The original APK identified below passed
sustained playback and interaction/lifecycle checks on the dedicated Android TV
emulator. Current root-build APK hashes and checks are recorded in
[main-native-build.md](main-native-build.md).
Rust compilation is mandatory in the root Gradle task graph; there is no
UI-only production build switch. Missing native exports fail the build.

Both NDK API 26 toolchains successfully compiled and linked an isolated
header/symbol probe against `mediandk`, `aaudio`, and `android`. The probe
was not packaged or run as playback. The real native runtime, ten-minute live
playback and external audio sink evidence below are separate checks. Physical
device validation remains outstanding. Native results are separate from the
[WebView baseline evidence](android-runtime-check.md).

## Native codec boundary

`native/crates/atv-android/src/codec.rs` has no third-party runtime dependency.
It uses NDK MediaCodec for AVC Surface output and AAC decoding, plus AAudio
for PCM output. It accepts a complete Annex-B video access unit with its PTS;
an audio call can contain multiple ADTS frames or a partial frame continued
by the next call. ADTS frames get individual sample-rate-based timestamps.
SPS/PPS configure AVC dimensions and codec-specific buffers; AAC ADTS metadata
configures AudioSpecificConfig. Codec output format changes are handled.

The renderer must repeatedly drain on its own worker. Bounded input queues
return `CodecError::Backpressure` without accepting the new packet. Surface
output is scheduled against monotonic time; a shared positive NDK timestamp
bias preserves negative normalized A/V offsets and B-frame timestamps.
All codecs and AAudio streams are stopped/deleted/closed by Drop before the
parent releases its native window.

Seven parser/timestamp/PCM-measurement tests passed. Both ABI variants passed
Rust cross-compilation and an isolated link probe that references the real
NDK calls. These checks establish build/format behavior, not live playback.

`OutputStats.video_frames` counts successful Surface render releases;
`audio_frames` counts PCM sample frames successfully accepted by AAudio.
Additional `audio_samples`, `audio_nonzero_samples`, `audio_peak` and
`audio_energy` describe only successfully written PCM. RMS can be calculated
as `sqrt(audio_energy / audio_samples)`. These are **internal output evidence**;
they do not establish that a physical or emulated speaker sink emitted sound.
Debug builds emit only the safe Rust status snapshot under log tag `ATVNative`
at most once every five seconds; release builds omit it.

## Emulator audio sink evidence

The first task emulator used `-no-audio`, so its results cannot establish an
external audio sink. A later `-no-window -audio pa` launch also failed: the
installed headless QEMU contains PulseAudio placeholder implementations that
return failure. This is confirmed by the installed binary's
`Using headless implementation` string and the official
[headless PulseAudio source](https://android.googlesource.com/platform/external/qemu/+/refs/heads/emu-master-dev/audio/paaudio-headless-impl.c).
Setting `PULSE_SERVER` alone cannot fix those placeholders.

The development emulator was restarted without wiping userdata, using the full
Qt QEMU with its window hidden and its offscreen platform. This connects to the
existing PulseAudio-compatible PipeWire socket without modifying host audio
configuration. Boot completed, the QEMU PulseAudio client appeared, and there
was no PulseAudio initialization error. The launch configuration is:

```bash
env PULSE_SERVER=unix:${XDG_RUNTIME_DIR}/pulse/native QT_QPA_PLATFORM=offscreen \
  ANDROID_HOME=${HOME}/.local/share/android-sdk \
  ANDROID_USER_HOME=/tmp/atv-android-tools/user-home \
  ANDROID_AVD_HOME=/tmp/atv-android-tools/avd \
  ${HOME}/.local/share/android-sdk/emulator/emulator \
  -avd atv-api35 -port 5570 -qt-hide-window -no-boot-anim -no-snapshot \
  -gpu swiftshader -memory 2048 -cores 2 -audio pa \
  -grpc 8554 -grpc-use-jwt
```

Do not run this while another instance of that AVD is active. After a restart,
check boot completion and the actual listening address before testing.

The installed version binds an unauthenticated
`-grpc` port to all interfaces, so JWT must be enabled and the actual listener
checked. With JWT, it restricts the controller to loopback. The capture tool
can register an ephemeral public ES256 key via `--discovery-file` and grants
its short-lived JWT only the read-only `streamAudio` method; the private key
stays in memory and its public registration is removed afterward. A preissued
client bearer can instead be supplied through `ATV_EMULATOR_BEARER`, which is
never printed. This follows the official [emulator gRPC authentication flow](https://android.googlesource.com/platform/external/qemu/+/686efa16baf59d776cadc3f975d12570fe44bbb9/android/android-grpc/docs/).
The installed SDK's
`emulator/lib/emulator_controller.proto` defines the read-only
`EmulatorController.streamAudio` RPC, which returns 20–30 ms PCM packets.
A two-second silent-control probe completed with `DEADLINE_EXCEEDED`, zero
packets and no authentication error. That checks authentication and bounded
waiting only; it does not establish playback or non-silent audio output.

With the real native mode-1 CCTV1 session playing, a subsequent ten-second
capture returned 431 packets / 960,748 signed PCM samples at 48 kHz stereo,
including 948,714 nonzero samples, normalized RMS 0.003872 and peak 0.023651.
The safe counts and PCM hash are recorded in
[`audio-sink-playing-mode1.json`](../test-artifacts/native/audio-sink-playing-mode1.json);
no audio was retained. Concurrent native status showed media time and actual
video/audio output counters advancing. This establishes a non-silent emulator
output path for that session. After a real Home key paused the native Activity,
a five-second isolated control produced zero packets and zero samples:
[`audio-sink-paused-mode1.json`](../test-artifacts/native/audio-sink-paused-mode1.json).
The final build's long-run evidence is recorded separately below; physical
device checks remain outstanding.

```bash
python3 -m venv /tmp/atv-audio-probe
/tmp/atv-audio-probe/bin/pip install grpcio cryptography
/tmp/atv-audio-probe/bin/python \
  scripts/capture-emulator-audio.py \
  --endpoint 127.0.0.1:8554 --seconds 10 \
  --discovery-file ${XDG_RUNTIME_DIR}/avd/running/pid_EMULATOR_PID.ini \
  --output test-artifacts/native/audio-sink.json
```

The development-only probe uses a bounded-deadline read-only RPC and records
packet/sample counts, nonzero samples, RMS, peak and a SHA-256 of the PCM.
It does not save audio. Run it while only the native app produces media and
compare against a paused/control interval; system tones or another app's audio
must not be attributed to this player. A nonzero sink capture is stronger than
AAudio-write counters, but physical speaker audibility and real-device behavior
still require their own checks.

## Dedicated Android TV test environment

The official SDK repository currently provides Android TV / Google TV x86_64
images at API 36 (revision 4). API 31, 33 and 34 TV images in the queried list
offer x86 or arm64 variants instead. For dedicated TV compatibility tests,
`system-images;android-36;android-tv;x86_64` was installed and a separate
`atv-tv-api36` AVD created using the `tv_1080p` profile. The image occupies
approximately 8.2 GB; the temporary AVD uses a 4 GB sparse userdata partition,
2 GB RAM and two virtual cores.

The TV instance runs on `emulator-5572`, with authenticated loopback gRPC on
8556, using the same hidden full-Qt/audio settings as above. Boot completed;
the guest reports Android 16 / SDK 36 and both `android.software.leanback` and
`android.hardware.type.television` as present. The API 35 generic Android
instance on 5570 was left running throughout installation and startup.

The final APK (`cdab6965adeeafec6cc370b8300157d3b96c9f027ff05441992d5197cdb2171e`)
was installed and normally launched on the TV system. Its real CCTV1 Surface
image was inspected; the native process created `c2.goldfish.h264.decoder`
and `c2.android.aac.decoder`. These are an emulator video component and Android
AAC software decoder, respectively; they do not establish physical TV hardware
decoding. A ten-second external sink capture produced 430 packets and 902,770
nonzero PCM samples, RMS 0.001668. The screenshot, safe audio statistics and
TV features/build information are in
[`tv-soak-389a119`](../test-artifacts/native/tv-soak-389a119/).
The 600-second same-channel TV playback gate passed in the same process and
channel: 600.756 seconds of observed wall time, 605.716 seconds of media-time
advance, 15,143 additional rendered video frames and 26,712,064 additional PCM
sample frames accepted by AAudio. Start/middle/end screenshots show changing
real Surface content. The external audio sink remained non-silent in separate
middle and end probes; the final five-second probe contained 474,448 nonzero
samples with RMS 0.001765. Process total PSS was 58,230 KB at the start and
63,074 KB at the end. See the monitor's `result.json` and bounded sink JSONs
in that artifact directory.

The independent TV interaction/lifecycle smoke test also passed all 14 steps
with the same APK hash. It covers two complete rounds of CCTV1/CCTV2/CCTV13/
Beijing satellite switching, real DPAD navigation and fullscreen/back, a real
server rejection followed by UI retry, Home pause/resume, Surface destruction/
recreation, and final Activity cleanup. Each checked destroyed handle reported
`idle`, with zero remaining native render/network worker threads. See
[`tv-smoke/result.json`](../test-artifacts/native/tv-smoke/result.json).
A separate five-second external sink probe after final Activity/worker release
returned zero packets and zero samples, providing the silent control for the
TV session: [`audio-sink-after-release.json`](../test-artifacts/native/tv-smoke/audio-sink-after-release.json).

Physical Android TV devices have not been tested, and these emulator checks do
not prove their hardware codec performance or physical-speaker audibility.
