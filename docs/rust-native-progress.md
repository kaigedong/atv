# Rust 原生实现与验收进度

更新：2026-10-16。本报告记录原 `feature/rust-native` 的验收；现在已合并到
`main`。基线 `9f20444` 保留于标签 `baseline/webview-9f20444`，另保留 `artifacts/atv-webview-baseline-debug.apk`。

后续多平台重构原位于 `feature/multiplatform`，现同样合并到 `main`。共享代码已移至 `atv-player` /
`atv-media`。架构与新版本证据见 [multiplatform.md](multiplatform.md)。本文
保留 Android 专用版本的历史验收数据，下表源码路径已更新为现行位置。

## 组件替换

| 组件 | 当前源码 | 证据及边界 |
| --- | --- | --- |
| 频道目录 | `atv-core/directory.rs` | 解析官方 protobuf 与内置 73 频道元数据；包含观看权限标志 |
| 正常鉴权与签名 | `atv-auth` | 84 个官方算法差分输出；CCTV1、CCTV2、CCTV13、北京卫视真实解析成功；保留服务端拒绝与试看信息 |
| 签名／票据 WASM | `atv-auth` | 可读 Rust MD5 / AES / 票据生成，无 WASM 执行器；详见 `rust-auth.md` |
| CMG 媒体 WASM | `atv-cmg` | Live mode 1/2 稀疏 TEA / EPB 处理；154 组合成原组件差分、627 个实际 mode 1 NAL 差分；两段合计 250 帧 FFmpeg 无解码错误 |
| HLS / 网络 | `atv-core`、`atv-player/playback.rs` | HTTPS、大小限制、重定向资源基址、分片去重、取消、有限重试、AES-128-CBC |
| TS / PES / PTS | `atv-core/ts.rs` | 跨分片 PES 保留、CRC、连续性、PMT 移轨；6 项本地授权样本回归 |
| 播放状态与资源 | `atv-player/engine.rs`、`atv-android/output.rs` | 平台输出单线程拥有；独立下载线程；全进程最多两个在途下载任务；旧会话不能发布新状态 |
| 视频解码 | `atv-android/codec.rs` | NDK MediaCodec AVC / Surface；双 ABI 编译，模拟器真实 CCTV1 960×540 画面已验证 |
| 音频解码与输出 | `atv-android/codec.rs` | MediaCodec AAC → PCM → AAudio；实际已写 PCM 与外部模拟器 sink 均有非静音证据 |
| Android 入口与遥控器 | `app/` | 少量 Java 原生控件和 JNI；无 WebView 或网页资源 |

Android 在应用私有目录保存设备 GUID，全进程复用正常鉴权 resolver。
切台、后台恢复和重建 Surface 不重置设备身份。原始响应和带签名资源地址
不写入日志或仓库。预览从首次真实输出计时，并保留可验证的付费到期时间。

## 已完成的验证

- Rust workspace 45 个普通测试通过；原组件 oracle 仅用于开发期研究，最终
  APK 不携带、下载或执行这些研究脚本与原 WASM。
- 6 项本地授权 TS 回归通过。隔离恢复旧实现后，跨分片数据一致性和
  PMT 移轨回归失败，确认它们覆盖了实际修复。
- Rust `probe_stream` 通过正常匿名鉴权下载并处理 3 个真实直播分片，
  得到 366 个视频访问单元、631 个 AAC 包、2,206,457 字节 elementary 数据。
  这是协议和媒体处理证据，不代替 Android 音视频播放验收。
- 原生 `arm64-v8a` 与 `x86_64` APK 构建及 Android lint 通过。
  打包检查确认无 JS / HTML / WASM 资源和 WebView DEX 引用；仅含
  `libatv_native.so`，ELF LOAD 段满足 16 KiB 对齐，包含全部六个 JNI 方法。
- 早期未实现 CMG 时，模拟器原生 app 真实显示 `cmg` 错误、音视频帧数为零；
  没有将失败改报为播放成功。完成 CMG 后，最终 APK 已通过下述完整运行验收。
- 原生 APK 已在模拟器实际播放 CCTV1。09:02:32 到 09:03:17 的状态
  从 80／141,312 个视频／音频帧推进到 1,204／2,123,776，媒体时间
  3.167 → 48.121 秒；截图确认真实 960×540 广播画面。
- 认证 gRPC 从模拟器实际音频出口采集 10.165 秒，仅保留统计和哈希：
  431 包、960,748 个 PCM 样本、948,714 个非零样本，48 kHz stereo S16LE，
  normalized RMS 0.003872。原始音频没有保存；详见 `rust-android.md`。
- 最终双 ABI APK SHA-256：
  `cdab6965adeeafec6cc370b8300157d3b96c9f027ff05441992d5197cdb2171e`。
  API 35 通用 Android 模拟器的持续播放观察通过：墙钟 600.741 秒、
  媒体时间推进 605.800 秒、视频增加 15,145 帧、音频增加 26,716,160 个
  PCM sample frames。122 个状态快照无播放错误，三个时点截图画面不同，
  中段外部音频出口仍非静音。报告：
  `test-artifacts/native/soak-389a119/result.json`。
- 独立官方 Android TV API 36 模拟器 `emulator-5572`，确认
  `android.software.leanback` 与 `android.hardware.type.television` 为 true。
  同一 APK 通过连续播放：墙钟 600.756 秒、媒体推进 605.716 秒、视频
  新增 15,143 帧、音频新增 26,712,064 sample frames；122 个状态快照无错误，
  三个时点真实画面不同，首／中／尾外部音频出口均非静音。
- 通用 Android 与 Android TV 的完整交互验收均通过：两轮八次央视／卫视
  切台、遥控器、全屏／返回、服务端拒绝／重试、后台恢复和 Surface 重建。
  TV 还验证全屏上下切台；通用 Android 独立验证真实断网与恢复。
  退出及旧 Surface 销毁后，旧 JNI 句柄 idle、render/network 线程均为零；
  TV 退出后的五秒外部音频出口对照为零包、零样本。

## 交付与验证范围

最终 APK 为 `artifacts/atv-rust-native-debug.apk`，包含 arm64-v8a 与 x86_64。
完整验收报告、目标逐项核对及复现方法见
[native-runtime-check.md](native-runtime-check.md)。2026-10-16 再次执行双 ABI
构建、Android lint、APK 检查和 45 项 Rust 普通测试通过；重建产物哈希不变。

已分别通过 API 35 通用 Android 与 API 36 Android TV x86_64 模拟器验收。
尚无实体 Android TV 播放、物理硬件解码或实体扬声器验证。早期 `-no-audio`
环境只证明内部输出；最终环境的认证 gRPC 音频出口与静默对照证据另行保存，
没有用内部 AAudio 写入计数代替外部声音证据。

## 明确的支持边界

已实现 AVC CMG Live mode 1/2、AAC ADTS 和 MPEG-TS HLS。Mode 2 的
组件／glue 行为已对照，尚未取得实际 mode 2 频道运行证据。
VOD、HEVC／AVS、HLS fragmented MP4／外置音频组会明确失败。
正常公开频道请求未遇到非空 `chanll`；该远端 JS 挑战
目前明确拒绝执行。它们不应被列为已完成或静默使用旧组件兜底。

## 复核命令

```sh
cargo test --manifest-path native/Cargo.toml --workspace
# 本地存在 normally-authorized research 样本时：
cargo test --manifest-path native/Cargo.toml -p atv-core --test ts_segments -- --ignored
cargo run --manifest-path native/Cargo.toml -p atv-player --example probe_stream
./scripts/native-android-build.sh
python3 scripts/verify-native-apk.py app/build/outputs/apk/debug/app-debug.apk
```

工具路径、JNI 契约和音频探针详见 [rust-android.md](rust-android.md)。
