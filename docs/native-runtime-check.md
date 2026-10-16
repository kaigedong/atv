> 2026-10-16：当前默认构建已增加 API 19／四 ABI 兼容与 Apple 原生宿主。下文保留此前验收记录；最新构建与性能状态见 [android-kitkat.md](android-kitkat.md)，Apple 验证边界见 [apple-native.md](apple-native.md)。

# Rust 原生直播运行验收（历史产物）

当前默认工程已迁到根目录 `app/`，见 [main-native-build.md](main-native-build.md)。
本报告保留下述 APK 哈希对应的历史验收。

测试日期：2026-10-16。生产组件源码见 `389a119`；测试工具见
`0d5b1ec` 及后续测试修正。所有测试调用真实正常鉴权、真实直播和原生输出，
没有测试播放器、模拟视频、JavaScript、官方 WASM 或 WebView 播放后门。

## 可安装产物

- APK：`artifacts/atv-rust-native-debug.apk`
- SHA-256：`cdab6965adeeafec6cc370b8300157d3b96c9f027ff05441992d5197cdb2171e`
- 应用 ID：`cn.bobo.atv.nativeplayer`
- ABI：`arm64-v8a`、`x86_64`；最低 API 26，目标 API 36。
- 本次所有生产播放测试使用同一 APK 哈希。修正测试输入准备没有更改生产代码。

包内只有自己的原生 Android 入口、频道元数据、资源和每 ABI 一份
`libatv_native.so`。没有 JS / HTML / WASM 资源、WebView DEX 引用或其他
原生媒体／脚本引擎库。ELF 含全部六个静态 JNI 导出，LOAD 段 16 KiB 对齐。
已检查 arm64 实际动态依赖仅为 `libandroid.so`、`libmediandk.so`、
`libaaudio.so`、`libdl.so`、`libc.so`。运行时应用内 WebViews 数量为 0。

## API 35 通用 Android 模拟器

设备 `emulator-5570`，x86_64。以下结果不冒充实体电视或物理硬件解码验证。

| 验收项 | 结果 | 本机证据 |
| --- | --- | --- |
| 真实画面 | CCTV1 清晰 960×540，持续观察的三张截图画面不同 | `test-artifacts/native/soak-389a119/screen-*.png` |
| 真实音频出口 | 10 秒 431 包、953,291 个非零 PCM 样本，48 kHz stereo | `audio-sink-final-build.json` |
| 暂停音频对照 | 5 秒无音频包、无样本 | `audio-sink-paused-mode1.json` |
| 连续至少 10 分钟 | 墙钟 600.741 秒，媒体推进 605.800 秒，视频新增 15,145 帧，音频新增 26,716,160 sample frames | `soak-389a119/result.json` |
| 长测中段声音 | 外部 sink 非静音，未干预前台／播放 | `audio-sink-soak-middle.json` |
| 多次切台 | CCTV1、CCTV2、CCTV13、北京卫视，两轮共八次，均有双轨与媒体时间推进 | `smoke-input-fixed/result.json` |
| 遥控器／全屏／返回 | 真实方向键与确认键；全屏上键切 CCTV1、下键切回 CCTV2，继续播放并保持全屏，返回退出全屏 | `remote-fullscreen-switch/result.json` |
| 服务器拒绝与重试 | 不存在的合法格式频道被服务端拒绝，显示 authorization 错误，UI 重试回正常频道 | `smoke-input-fixed/result.json` |
| 真实断网与恢复 | 临时关闭模拟器 Wi-Fi / data，出现 network 错误且网络线程退出；恢复原状态后 UI 重试双轨恢复 | `network/result.json` |
| 前后台恢复 | 后台 Surface 释放，旧句柄 idle、render/network 线程均 0；恢复后双轨推进 | `smoke-input-fixed/result.json` |
| Surface 销毁／重建 | 隐藏 Surface 后旧句柄失效、线程退出；显示后创建新句柄并正常播放 | `smoke-input-fixed/result.json` |
| 退出清理 | Activity 销毁、Java 句柄 0、旧 JNI 句柄 idle、原生 render/network 线程均 0 | 上述各 runner 的 finish 步骤 |

表中本机证据目录统一为 `test-artifacts/native/`，它们包含有限状态与哈希，
没有服务令牌、带签名地址或广播音频。长测应用 PSS 从约 55 MiB 到 64 MiB；
该观测用于描述本次运行，不推断任意时长均无内存增长。

首次 smoke 的八次切台通过，随后测试代码在触摸模式下直接 requestFocus
失败。补充诊断确认按钮可见、已附着、可聚焦，但 `inTouchMode=true`；
发送真实 DPAD_UP 后为 false，完整原断言及额外全屏切台断言通过。
没有修改产品焦点属性、使用模拟媒体或降低断言。首次失败报告保留在
`smoke/result.json`。

## Android TV API 36

独立官方 Android TV x86_64 revision 4，设备 `emulator-5572`，1920×1080。
已确认 Leanback 与 television 系统 feature。相同 APK 的同频道长测通过：
墙钟 600.756 秒、媒体推进 605.716 秒、视频新增 15,143 帧、音频新增
26,712,064 sample frames。122 个状态快照没有播放错误，三个时点的截图
显示不同的真实直播画面。报告在 `test-artifacts/native/tv-soak-389a119/result.json`。

长测首段、中段、末段均从外部音频出口取得非静音 PCM；末段 5.166 秒
含 215 包、474,448 个非零样本。Activity 和原生线程释放后，独立 5.164 秒
对照为零包、零样本。统计与哈希见该目录的 `audio-sink-*.json` 和
`test-artifacts/native/tv-smoke/audio-sink-after-release.json`，没有保存音频。

电视交互测试 14 步全部通过：两轮八次央视／卫视切台，真实方向／确认键，
全屏上下切台和返回，真实服务端拒绝与 UI 重试，Home 后台／恢复，Surface
销毁／重建，最终 Activity 退出。旧 JNI 句柄返回 idle，render/network
线程均已退出。报告在 `test-artifacts/native/tv-smoke/result.json`。

模拟器使用 `c2.goldfish.h264.decoder` 与 `c2.android.aac.decoder`。
这证明 NDK MediaCodec 链路在实际 TV 系统运行，不证明实体设备的硬件性能。
尚未连接实体 Android TV 做播放验收。

## 目标逐项核对

| 原目标 | 当前证据 |
| --- | --- |
| 1. 保留可运行基线 | 基线 `9f20444` 和 APK 原样保留；现在 `main` 已合并原生实现，旧版本由历史标签访问 |
| 2. 调查实际播放链路 | `rust-auth.md`、`rust-cmg.md` 分别列出固定原组件哈希、REA 证据与实际调用职责；CMG 变换不承担像素解码 |
| 3. 可读 Rust 替代与行为对照 | `atv-auth` 84 项差分输出，`atv-cmg` 154 组合成差分及 627 个真实 NAL 对照；服务端权限拒绝测试和实际运行通过 |
| 4. 原生网络／协议／媒体／状态 | `native/crates/` 四个 crate；实际双轨输出，MediaCodec / AAudio；Java 只承载控件、Surface、输入与生命周期 |
| 5. 最终无网页／JS／原 WASM | APK 检查六个 JNI 导出、双 ABI、资源／DEX／ELF；真实播放使用同一最终 APK，未切换回基线 |
| 6. 编译和真实运行验收 | 本文两类模拟器的长测、画面、外部音频、切台、遥控、后台和失败清理证据；实体设备明确未验证 |
| 7. 分阶段提交与交付 | `389a119` 实现、`0d5b1ec` 验收工具及后续测试／文档提交；可安装 APK、哈希与重现命令见本文 |

上述验收对应已观察到的 AVC/AAC MPEG-TS HLS 直播链路。格式与未观察
协议分支边界见 [rust-native-progress.md](rust-native-progress.md)，不据此声称
所有频道、格式、未来服务端变化或实体电视都已验证。

## 重现

```sh
./scripts/native-android-build.sh
python3 scripts/verify-native-apk.py artifacts/atv-rust-native-debug.apk

# 指定自己拥有的开发设备；普通 smoke 不更改网络设置。
ATV_TEST_DEVICE=emulator-5570 ./scripts/native-android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5570 ATV_TEST_MODE=remote ./scripts/native-android-runtime-check.sh

# network 模式会临时修改开发模拟器网络，并 finally 恢复原状态。
ATV_TEST_DEVICE=emulator-5570 ATV_TEST_MODE=network ./scripts/native-android-runtime-check.sh

# 已正常启动播放器后，单独观察同一频道，校验实际安装包哈希：
python3 scripts/monitor-native-live.py --serial emulator-5570 --seconds 600 \
  --apk artifacts/atv-rust-native-debug.apk --output test-artifacts/native/soak-repeat
```

音频出口采集和 SDK / NDK / Rust 工具链说明见 [rust-android.md](rust-android.md)。
签名和 CMG 算法差分分别见 [rust-auth.md](rust-auth.md)、[rust-cmg.md](rust-cmg.md)。
原始 WebView commit `9f20444` 已保留于历史标签 `baseline/webview-9f20444`，基线 APK 已保留，哈希仍为
`3a885f4ed2e69a8cce691893a31014df599989ed2286a595a539c6bca590fef5`。

## 设置、清晰度和更新新增验收（2026-10-16）

Android TV / API 36 模拟器已通过菜单键、顶部按钮、全屏右键打开设置，返回先关闭设置、
再退出全屏；打开设置保持原 Surface/session。实际 CCTV13 的超清、蓝光、自动依次输出
1280×720、1920×1080、960×540，并保存清晰度偏好；选项来自服务端视频格式，排除纯音频。

Android TV 与 API 19 / 512 MiB 模拟器均已完成真实 GitHub Release 检查、取消后重试、
完整 APK 流式下载与重新校验，以及当前开发签名不能覆盖正式签名包的拦截提示。
API 19 这里只验收设置与更新组件，不冒充其 stock 软件解码器支持当前 High Profile 直播。
本地结果位于忽略目录 `test-artifacts/settings/`；未将设备标识、抓包或下载 APK 放入源码。
正式签名包经系统安装器完成覆盖升级的验收将在正式构建后补充。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=settings ./scripts/android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5574 ATV_TEST_MODE=settings-basic ./scripts/android-runtime-check.sh
```

默认 Rust 测试不请求直播或 GitHub 服务；真实检查/下载探针需要明确指定输出目录：

```sh
cargo run --manifest-path native/Cargo.toml -p atv-update --example probe_release -- /tmp/atv-update-probe 1 download
```
