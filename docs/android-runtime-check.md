# Android runtime verification

> 历史 WebView 基线记录，仅适用于标签 `baseline/webview-9f20444`。其中的网页／Node 构建命令须在该历史版本的独立工作树使用。当前 main 的默认应用为原生版本，见 [main-native-build.md](main-native-build.md)。


Verified on **2026-10-16**, using the API 35 Google APIs x86_64 emulator
`emulator-5570` with Android System WebView **124.0.6367.219**. No real device
was accessed.

## Results

The full check started after `adb -s emulator-5570 shell pm clear cn.bobo.atv`,
so the successful run had no cookies or local storage seeded by the reference
website. It completed in **112 seconds**.

| Check | Observed result |
|---|---|
| Genuine official page context | `originOk=true`, `minimalHost=true`, own UI ready |
| CCTV1, PID `600001859` | 960×540; video time **0.86 → 75.88 s**, never paused during the sustained check |
| CCTV13, PID `600001811` | Trusted remote selection; video time **0.50 → 10.50 s**, 960×540 |
| Fullscreen | Real DPAD CENTER activation; `document.fullscreenElement` and CSS fullscreen both became true |
| Back from fullscreen | Both fullscreen flags became false; activity remained open |
| Background lifecycle | HOME triggered pause; video was paused, released its media, and time remained 0 across a 3-second observation |
| Foreground lifecycle | Same CCTV13 resumed; video time **0.50 → 10.50 s**, 960×540 |
| Beijing Satellite TV, PID `600002309` | Final packaged build; switched groups with real remote input; video time **0.51 → 5.51 s**, 960×540 |
| Official authorization responses | `auth`: HTTP 200 / code 0; `get_live_info`: HTTP 200 / code 0 / iretcode 0 / `hd` / previewTime 0 |

The final satellite check also reopened CCTV13 and decoded video before
switching groups. That final package contains exactly four web assets:
`index.html`, `app.css`, `app.bundle.js`, and `vendor/live-resolver.vendor.js`.
It refuses WebView versions below 100 with an explicit upgrade message.

## Artifacts

- [Full numeric/state evidence](../test-artifacts/android/result.json)
- [Initial CCTV1 picture](../test-artifacts/android/01-initial.png)
- [CCTV1 after sustained playback](../test-artifacts/android/02-sustained-75s.png)
- [CCTV13 after switching](../test-artifacts/android/03-cctv13.png)
- [Trusted fullscreen](../test-artifacts/android/04-fullscreen.png)
- [Playback after lifecycle resume](../test-artifacts/android/05-resumed.png)
- [Satellite evidence](../test-artifacts/android/satellite/result.json)
- [Beijing Satellite TV picture](../test-artifacts/android/satellite/06-beijing-satellite.png)

Screenshots come from `UiAutomation.takeScreenshot()`. Probes read only
whitelisted DOM state, video state, numeric API result codes, and sanitized
JavaScript errors. They do not retain signed stream URLs, authorization
headers, request payloads, player tokens, or tickets.

## Reproduce

With the task emulator running:

```bash
# Clear only the task app to reproduce the cold-start boundary.
adb -s emulator-5570 shell pm clear cn.bobo.atv
./scripts/android-runtime-check.sh

# Short additional source-group check; does not repeat the 75-second hold.
ATV_TEST_MODE=satellite ./scripts/android-runtime-check.sh
```

The script generates the web bundle, builds both APKs, explicitly targets the
chosen emulator, installs them, and runs the custom instrumentation. It
removes the previous generated result before the run to avoid stale success
artifacts. `ATV_TEST_DEVICE` and `ATV_TEST_ARTIFACTS` override the device and
artifact destination. `ATV_TEST_MODE=probe` only checks initial playback.

## Diagnostic control and limits

Before the official initializer was exported, the minimal host received
HTTP 401 for both authorization endpoints and produced no decoded video.
The debug-only `ReferenceActivity` loaded the complete original site without
blocking or UI replacement and played 1920×1080 video with HTTP 200 responses.
This isolated the problem to missing normal client initialization. Calling
the unchanged official `Hc.initCookie()` initializer resolved the issue;
the cold run above verified that successful playback no longer depended on
the control activity's stored state.

The reference component is under `src/debug`, is not exported, and is excluded
from release builds. It can be reproduced with
`ATV_TEST_MODE=reference ./scripts/android-runtime-check.sh`. Its evidence is
[here](../test-artifacts/android/reference/result.json).

Two harmless errors were observed: the minimal host's blocked original Trace
bootstrap attempted to read `undefined.default`; the original player also
emitted `a0b is not defined` while continuing to decode video. Neither prevented
the verified playback or controls.

This verifies CCTV1, CCTV13 and one satellite channel in the current emulator
and network. It does not establish availability of every listed channel,
restricted content, other regions, or every Android device. Playback still
depends on the official HTML, SDK, authorization service and media CDNs.

## Build validation

`assembleDebug`, `assembleDebugAndroidTest`, and `lintDebug` passed for the final
package. `apksigner verify --verbose` passed APK Signature Scheme v2; the merged
APK declares only `android.permission.INTERNET`.

Final debug APK SHA-256:

```text
3a885f4ed2e69a8cce691893a31014df599989ed2286a595a539c6bca590fef5
```
