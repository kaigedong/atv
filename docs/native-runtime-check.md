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
正式签名 TV / API 36 已完成应用内 `1009 → 1010` 覆盖升级：实际检查并下载 Release、
进入“安装未知应用”授权页、手动允许后调用系统安装器确认更新，安装后清晰度偏好仍为超清。
该路径使用只读授权 content URI，没有通过 adb install 替代应用内安装。

API 19 / 512 MiB、无共享存储的模拟器已完成正式版应用内 `1011 → 1012` 覆盖升级：
检查并下载 Release 后，系统安装器通过 file URI 读取独立 APK 暂存目录，手动确认安装成功。
安装后系统返回桌面，重新打开应用确认版本为 `1012`、超清偏好保留，且检查提示已是最新版本。
TV / API 36 也完成 `1010 → 1012` 的 content URI 覆盖升级，系统“打开”按钮返回应用后
版本、超清偏好与最新版本检查均通过。这两次覆盖升级均未使用 adb install。
以上为模拟器上的真实安装流程验证，不代替目标电视的解码与性能验收。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=settings ./scripts/android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5574 ATV_TEST_MODE=settings-basic ./scripts/android-runtime-check.sh
```

默认 Rust 测试不请求直播或 GitHub 服务；真实检查/下载探针需要明确指定输出目录：

```sh
cargo run --manifest-path native/Cargo.toml -p atv-update --example probe_release -- /tmp/atv-update-probe 1 download
```

## 全尺寸播放与半透明浮层验收（2026-10-16）

Android 的播放器容器改为始终占满窗口，按原始视频比例显示。频道、顶部操作与提示为同层覆盖视图；
设置和清晰度为半透明原生弹窗，关闭全窗口调暗。打开设置暂时隐藏频道浮层，关闭后恢复原来的菜单状态。
频道浮层只在确认时切台，浏览和打开菜单不暂停直播；确认切台／切换清晰度仍会重新加载直播。
返回依次关闭清晰度、设置或频道浮层，没有浮层时退出应用。全屏确认／左键打开频道，右键／菜单键打开设置。

TV / API 36 真实 CCTV13 直播测试已验证：浏览频道、显示设置、显示清晰度选项、返回恢复频道以及
全屏右键设置期间，原生 session 与 Surface 坐标均保持不变（1920×1080），音视频帧及媒体时间持续推进。
真实确认切换 CCTV2 后继续输出双轨；最终退出后 render/network 线程归零。
API 19 / 512 MiB 验证同样的半透明菜单、返回层级和 Surface 坐标保持（1280×800）；
该基础模式主动暂停直播以单独验收布局，不声称 stock API 19 解码器可播放当前 High Profile 直播。
API 19 同时固定沉浸式布局尺寸，避免系统栏显示与隐藏造成播放器尺寸变化。
浮层版本还通过 TV `remote` 与 `settings` 回归：真实上下快捷切台、确认／返回，
超清 1280×720、蓝光 1920×1080、自动 960×540 切换，以及 Release 检查、取消重试、
完整下载、缓存重新校验和开发／正式签名不匹配拦截。清晰度或提示弹窗显示时隐藏底下的设置文字，
关闭后恢复，避免半透明面板间文字叠加。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=overlays ./scripts/android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5574 ATV_TEST_MODE=overlays-basic ./scripts/android-runtime-check.sh
```

本地报告与截图在忽略目录 `test-artifacts/overlays/`；未发布广播截图或设备运行记录。

## 当前方向导航与自动恢复

频道列表右键依次从央视到卫视、再进入设置；设置左键返回卫视，卫视左键返回央视。
从设置返回时在弹窗关闭后恢复频道焦点与滚动位置，浏览不改变正在播放的频道。

共享 Rust 播放核心对网络、列表、媒体、输出设备和无进展故障自动重新获取当前频道，
保留清晰度。首次输出等待上限为 20 秒，音视频单轨无进展检测仍为 12 秒。
重连间隔为 1、2、4、8、15 秒并保持 15 秒上限；连续正常播放 20 秒后重置退避。
暂停、选台、停止和销毁取消旧重连。权限拒绝、试看结束及未支持的格式/验证保持终止错误。
恢复等待阶段为 loading，UI 不显示手动重试按钮；分类诊断在设置中保留，开始新尝试时清除。
状态新增 reconnectAttempts，表示显式选台以来实际发起的自动重连次数，等待计时不会累加。

27 项播放核心单元测试覆盖精确退避、首帧前失败、启动超时、输出超时、稳定播放、
暂停/切台/销毁取消、清晰度、权限/试看结束和线程容量。模拟时钟测试不冒充真实网络验证。

以下运行模式分别检查真实遥控器导航、启动前断网与播放中断网恢复；无线断网测试应选择
有 Wi-Fi/移动网络的开发模拟器，有线连接的 TV 模拟器关闭 Wi-Fi 不代表网络中断。
network/startup-network 会在 finally 中恢复原来的无线设置，恢复后不点击重试或发送恢复指令，
直接等待原频道双轨推进，并检查正常播放不继续重连。普通 smoke/all 不修改网络设置。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=groups ./scripts/android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5570 ATV_TEST_MODE=startup-network ./scripts/android-runtime-check.sh
ATV_TEST_DEVICE=emulator-5570 ATV_TEST_MODE=network ./scripts/android-runtime-check.sh
```

当前版本已通过 TV 模拟器 groups 导航与 feedback 加载反馈验证；无线 Android 模拟器的
network、startup-network、lifecycle 和 retry 模式均通过。两种断网场景都记录到 2 次自动重连，
恢复后没有用户操作，原 session/频道继续输出音视频，随后约 8 秒推进中没有额外重连；
退出后 render/network 线程归零。启动卡死 20 秒和单轨停滞 12 秒的精确边界由模拟时钟单元测试验证。

本地运行结果保存在忽略目录 test-artifacts/native/auto-recovery/；未发布广播画面、鉴权数据或设备身份。

## 重新打开后的音频与系统焦点

Activity 恢复或用户明确播放时请求音乐音频焦点；只有在前台且已获得焦点时才允许 Rust 恢复输出。
退出/进入后台释放焦点，旧回调不能恢复已经关闭的播放器。暂时失去焦点时暂停，重新获得后自动恢复；
永久失去焦点会清理旧请求，由再次进入窗口或明确播放重新申请。Surface 自动重建不抢占仍在等待的焦点。
API 26+ 使用媒体 AudioFocusRequest，API 19 使用 STREAM_MUSIC 兼容接口。
API 19 AudioTrack 在 play 后检查 PLAYSTATE_PLAYING，失败交由已有播放错误恢复策略处理。
实现不修改系统音量或强制取消用户静音。

新增 audio-lifecycle 模式检查连续三次 Activity 关闭/重开，保持原频道且不通过切台或自动重连修复输出，
并检查临时/永久焦点竞争、失焦期间 Surface 重建，以及释放后的过期回调。
audio-focus-basic 在 API 19 上只检查重开和焦点兼容性；该开发镜像无法解码实际直播的 AVC profile，
因此它的通过不代表 API 19 真实直播声音通过。

宿主机验收工具会关联同一台模拟器的 instrumentation 与真实输出 sink，分别采样首次播放、三次重开、
失去焦点的静音对照和重新获得焦点后的声音。静音对照先单独排空混音器尾音，再要求完整采样窗口没有非零样本。
每次结果都绑定 APK SHA-256，只保留样本统计和 PCM 哈希，不保存音频。采集必须使用不存在的新输出目录，避免读取旧结果。

```sh
# 使用已有安装 grpcio / cryptography 的开发虚拟环境 Python。
python scripts/android-audio-reopen-check.py \
  --serial emulator-5572 --endpoint 127.0.0.1:8556 \
  --discovery-file "${XDG_RUNTIME_DIR}/avd/running/pid_EMULATOR_PID.ini" \
  --output test-artifacts/native/audio-reopen/final-tv
ATV_TEST_DEVICE=emulator-5574 ATV_TEST_MODE=audio-focus-basic ./scripts/android-runtime-check.sh
```

旧版在 TV 模拟器上的三次重开基线也有非静音输出，因此尚未在开发模拟器复现用户电视的静音。
已确认的缺口是原实现没有管理音频焦点；物理电视的音频服务 dumpsys 被 DUMP 权限拒绝，不能用于判断焦点或静音状态。
本轮未在物理电视安装或操作播放。焦点生命周期和真实模拟器输出验证不能代替该电视更新后的实际试听。

本轮最终 APK 的四 ABI 构建、lint、API 19 audio-focus-basic、TV audio-lifecycle、groups 和 lifecycle 全部通过。
六个实际输出采样阶段均通过：首次播放、三次重开和重新获得焦点后均有非零声音样本；失焦后排空尾音的对照窗口为零样本。
本地结果位于 test-artifacts/native/audio-reopen/verified-tv/，旧版兼容结果位于同目录下的 api19/。

## 频道列表闲置收起

频道行持有焦点且窗口在前台时，连续 5 秒没有操作会收起频道列表并回到全屏。
遥控按键（包括长按重复事件）和频道焦点变化会重新计时；焦点移到顶部按钮、打开设置、窗口失焦或进入后台时取消计时。
重新打开列表或从设置返回后从头计时。收起只改变界面，不选择频道或重建播放会话。

新增 channel-idle 验收：检查延迟收起、按键续时、非频道焦点豁免、设置豁免与返回后的新倒计时，
并检查收起前后 session/频道不变、音视频计数继续推进。TV 模拟器的 channel-idle、groups 和 remote 均通过，构建和 lint 通过。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=channel-idle ./scripts/android-runtime-check.sh
```

## 播放页双次返回退出

全屏首次返回显示底部居中的深色圆角提示“再按一次退出”，2.5 秒内再次返回才结束 Activity。
超时只隐藏提示，继续留在播放页；频道列表和设置仍优先关闭。其他按键、打开菜单、窗口失焦、暂停或销毁会清除确认状态。
提示不获取焦点、不拦截点击，独立叠加在视频上。

新增 `back-exit` 验收在 Android TV API 36 模拟器通过：关闭浮层不触发退出确认、提示文案与底部居中位置、
首次返回和超时后 session/频道不变、过期后重新确认、其他操作和暂停恢复取消确认，以及连续两次返回退出并释放原生线程。
此用例不等待远端直播成功，不作为音视频输出验收。四 ABI Debug APK、设备测试 APK 构建及 `lintDebug` 均通过。

```sh
ATV_TEST_DEVICE=emulator-5572 ATV_TEST_MODE=back-exit ./scripts/android-runtime-check.sh
```
