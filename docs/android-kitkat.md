# Android 4.4、目标魔盒与性能验证

本页保留上一轮兼容性与性能记录（截至 `8c7aa58`）。后续播放缓冲优化、
当前 APK 校验值与新测量见 [直播内存优化](memory-optimization.md)；
`artifacts/` 中同名安装包已更新，应以新文档的校验值为准。

2026-10-16 更新。默认工程最低 API 19，包含 `armeabi-v7a`、`arm64-v8a`、
`x86`、`x86_64` 四个 ABI。编译通过不等于所有 Android 4.4 设备都能解码当前
直播流；本文件单独列出工具链、组件输出、真实直播及真机证据。

## 低内存设备目标

目标类别是 Android 4.4 / API 19、32 位 ARMv7、约 512 MiB 内存的电视盒子。
设备型号、系统指纹、连接地址及个人会话调查记录不随公开仓库发布。
目前没有实体盒子的安装、直播、厂商硬件解码或 CPU／内存验收结果。

## 兼容实现

旧实现直接依赖 API 21 的 `libmediandk.so` 和 API 26 的 `libaaudio.so`，
最低 API 26，不能通过改 Manifest 自动支持 4.4。

当前 32 位产物通过 Rust/JNI 调用 API 19 已有的 MediaCodec／MediaFormat／
AudioTrack。鉴权、网络、解密、CMG、队列、时间戳、同步及状态仍由 Rust 处理，
没有引入 Java 播放实现、JS、WebView、官方 WASM 或脚本引擎。
旧 MediaCodec 使用 `getInputBuffers`／`getOutputBuffers`，变化时刷新输出引用；
Surface 由引用所有者与输出共同持有，避免 factory 先销毁时悬空。

KitKat 没有带时间戳的视频 release，适配层只在输出工作线程等待目标时刻。
AudioTrack 在写入前用 playback head 和已写帧数检查空闲容量，避免旧阻塞式
write 等待音频播放而卡住视频线程；PCM 转换使用固定栈缓冲区和复用 Java 数组。
压缩视频输入缓冲区按图像尺寸估算并限制在 256 KiB–4 MiB，减少小分辨率的预留。

64 位产物链接 API 21 的 NDK MediaCodec，在运行时检查并加载 AAudio。系统缺少
AAudio 时使用 AudioTrack，覆盖 Android 5–7。新系统仍走原生 NDK 输出。
基础主题与图标有 API 19 资源，Material／vector 资源限定在 v21。

官方 API 边界见 [Android native APIs](https://developer.android.com/ndk/guides/stable_apis)
和 [MediaCodec](https://developer.android.com/reference/android/media/MediaCodec)。

## 工具链与维护边界

```sh
# SDK Manager 中安装以下 NDK，然后使用标准构建入口：
sdkmanager 'ndk;25.2.9519653' 'ndk;29.0.14206865'
./scripts/android-build.sh clean assembleDebug assembleRelease lintDebug
# 针对目标魔盒的单 ABI 包：
ATV_ANDROID_ABIS=armeabi-v7a ./scripts/android-build.sh assembleDebug
```

32 位使用 NDK r25c／API 19；64 位使用 NDK r29／API 21。Rust 固定 1.99.1，
Cargo.lock 固定依赖。现代官方 NDK 从 r26 起不支持 API 19，不能将本工程的
兼容维护误称为现代 Rust／NDK 官方保证支持 KitKat：
[NDK r26 变更](https://github.com/android/ndk/wiki/Changelog-r26)。

链接器使用 `-z defs`，要求所有强引用都能由所选最低 API stub 解析。
KitKat 的 `epoll_create1` 由 `epoll_create`＋`FD_CLOEXEC` 实现。本播放器不会
fork／exec 子进程。32 位 ARM 在 API 19 缺少 `dl_iterate_phdr`，仅在系统
提供时转发；否则 Rust 可选 backtrace symbolizer 得不到模块枚举。播放及
ARM EHABI 栈展开不依赖此可选模块枚举，不能声称 KitKat 上完整符号化可用。

32 位 getrandom 0.3 使用 custom backend 转发到已有 getrandom 0.2 的 Android
系统调用／熵就绪 `/dev/urandom` 路径，避开老 linker 的全局 dlsym 崩溃，
没有换成时间戳、固定值或非密码随机数；读取失败仍返回错误。

APK 必须通过资源、DEX、实际 ELF32／64、架构、16 KiB LOAD 对齐、JNI 导出与
直接依赖检查。API 19 库不能强依赖 mediandk／aaudio。测试探针只打包到可选
androidTest APK，正式 APK 只含频道 JSON 和四个 ABI 的播放器库。

## 已验证范围

1. 四 ABI Debug／Release 构建、最低 API 链接检查、APK 检查及 Android lint。
2. API 19 x86 官方系统镜像：安装／启动、真实正常鉴权和媒体获取已到解码阶段；
   旧 `OMX.google.h264.decoder` 拒绝当前 High Profile 直播，未把这项记为通过。
3. API 19 平台组件探针：FFmpeg 生成的 320×180／25 fps Baseline AVC＋
   44.1 kHz 双声道 AAC，通过同一生产输出适配层。首次完整通过时提交 113 帧
   视频、198656 个音频 sample frames、397237 个非零 PCM samples，Surface
   截图检测到 2584 个有明显色彩的采样点，随后输出和原播放工作线程均清理。
   **这是组件证据，不是直播鉴权／High Profile／目标 ARM 真机通过的证据。**
4. API 36 Android TV 官方模拟器：真实直播、切台、遥控器、全屏、后台恢复、
   Surface 重建、失败重试、清理及 10 分钟连续音视频已有本轮通过报告。

复现组件测试：

```sh
ATV_TEST_DEVICE=emulator-5574 ATV_TEST_MODE=component \
  ./scripts/android-runtime-check.sh
```

生成的素材位于 `app/src/androidTest/assets/baseline-avc-aac.ts`，测试库位于
`native/tests/android-output-probe`；无需原站媒体。探针库禁止导出生产
NativeBridge JNI 方法，避免把测试库的独立注册表当作正式播放器。

真实 High Profile 在目标魔盒上是否能使用厂商硬件解码、其实际性能与内存，
仍需设备上线后验证。其他 4.4 设备也受厂商 AVC profile／level 能力约束。

## 性能测量方法

`NativeRuntimeInstrumentation` 每约 5 秒采集进程 CPU 时间、PSS、private dirty、
native／Dalvik PSS、malloc heap、Java heap、工作线程和 AV 计数。
`scripts/summarize-android-performance.py` 排除前 60 秒预热，CPU 100% 表示一个
逻辑核心；区间 CPU 统计忽略不足 1 秒的强制结束快照。

第一轮兼容构建 `9e8e941f…` 的 10 分钟直播，960×540／25 fps，预热后约
540.5 秒、110 个样本：PSS 中位 62.25 MiB、P95 63.48 MiB、最大 64.58 MiB，
进程 CPU 平均 6.94% 单核心，输出约 25.00 fps。报告：
`test-artifacts/android19/tv-all/result.json`、`before-performance.json`。

这些是模拟器的**应用进程**数据，包含 instrumentation 开销，不能代表魔盒
ARM／物理硬件，也没有把共享系统解码服务和 GPU 的全部资源计入应用。
PSS 不是 RSS，短期稳定或两个时间窗的差值不能证明没有内存泄漏。
最终交付包的复测数据另附在本文件后续验收记录中。

## 最终构建与低内存组件复测

源码提交 `6ff616e` 已合并到 main。当时的四 ABI Debug 包（后续 32 位调度修正见下文）：
`7cd0d7a7cb5804aa4b79fa5c7ca4e73318c2ad0b697c02e9eeff0cbcf0d48824`；
当时的目标魔盒单 ARMv7 Debug 包：
`524997040655463f992398722a5d69897c48761f2f6098a8f9427e8eb92d6d72`，
1400320 字节，约 1.34 MiB。APK 位于仓库 `artifacts/`。

API 19 模拟器重启为 512 MiB RAM，guest MemTotal 为 511560 KiB。
四 ABI 组件测试包 `7103398a…` 在此环境再次通过同样的音视频、截图和清理
检查；该包与最终 `7cd0d7a7…` 的全部 22 个 ZIP entry payload 完全一致，
差别在 APK 容器。组件运行结束时应用 PSS 16768 KiB，清理后 13380 KiB，
只是 320×180 生成片段的一次快照，不能当作真实直播或魔盒内存占用。
证据：`test-artifacts/android19/final-component-512/result.json`、
`delivery/payload-equivalence.json`。

缓冲调整后的中间构建 `a17b451d…` 已再通过一轮完整 TV 检查及 10 分钟直播。
同为 960×540／25 fps，预热后 PSS 中位 60.29 MiB、P95 62.40 MiB、最大
64.08 MiB，CPU 平均 7.12% 单核心。相对首轮约少 1.96 MiB PSS 中位数，
CPU 略高，不能据这两轮就认定普遍收益。最终交付哈希的独立复测另行记录。

共享核心与 Apple ABI 共 54 个 Rust 测试通过；核心／Apple／Android ARMv7、
x86、arm64 Clippy 检查通过。公开 C 头文件与真实 Rust staticlib 的链接、
线程和缓冲区约定检查通过。依赖门禁负例能拒绝错误 ELF 位数及新增脚本运行库。
这些静态／宿主检查不扩大成 Apple 或魔盒实机验收。

## 交付验收及旧系统调度修正

32 位真实直播测试发现，KitKat 的 timed-release 兼容等待可能在 drain 中连续
等待数个视频帧，导致 AudioTrack 得不到及时补充。现在旧适配层把未来视频帧
保持 pending，到期再提交；NDK 时间戳提交仍允许提前 40 ms 排队。

修正后：API 19／512 MiB 组件音视频、实际截图和清理再次通过；API 23／32 位
同一旧适配层完整切台、遥控器、失败重试、生命周期及 120 秒真实直播通过。
44.1 kHz 音频帧提交在后续采样区间约为 44100 frames/s，视频约 25 fps。
API 23 有可用的 High Profile 软件解码器，不能将其验收替代为目标魔盒或
API 19 模拟器当前 High Profile 直播通过。

最终通用 Debug APK：
`eb1c0d0185ff98dcba7f525ebf4d4613f225f5e925b3c274ca80aae35affead1`。
魔盒专用 ARMv7 Debug APK：
`69929f00d651f37ace0ad459772aaa7c5f305180013256c7844828edf214c1b0`。

最终通用包与 API 19／API 23 测试包 `f05baccb…` 的 22 个 ZIP entry payload
全部相同，容器哈希不同。相对完成十分钟 TV 验收的 `7cd0d7a7…`，只更新
32 位库；arm64／x86_64 库、DEX、Manifest、频道资源与 compiled resources
字节完全一致，另对最终包执行独立 TV smoke。
身份对照记录在 `delivery/final-component-payload-equivalence.json` 和
`delivery/timing-payload-equivalence.json`，不混用整个 APK 的运行哈希。

`7cd0d7a7…` 的完整 TV 检查及 600 秒直播通过，排除预热后约 540.6 秒：

| 测量项 | 结果 |
| --- | --- |
| 应用进程 PSS 中位／P95／最大 | 62.22／64.28／66.04 MiB |
| 应用进程 CPU 平均 | 7.08% 单逻辑核心 |
| 视频输出 | 25.00 frames/s，960×540 |
| 音频提交 | 44099.87 sample frames/s，44.1 kHz 双声道 |
| 最后与最初窗口 PSS 中位差 | +2.80 MiB，不能单独据此判断泄漏 |

三轮 10 分钟测试的 PSS 中位约 60.3–62.2 MiB，CPU 约 6.9–7.1% 单核心。
测试期间有其他模拟器和宿主构建活动；不是受控的 ARM 硬件基准，也不能断言
缓冲调整普遍节省 2 MiB。系统媒体／音频／合成服务另有 playing 状态快照，
没有把这些共享服务总占用累加成应用专属开销。

最终报告路径：`main-tv-all/result.json`、`main-performance.json`、
`legacy-timing-api19-component/result.json`、`legacy-timing-api23-all/result.json`、
`delivery-tv-smoke/result.json`（均位于 `test-artifacts/android19/`）。

32 位 API 23 软件解码场景另行测得：修正后 120 秒直播排除前 60 秒，应用进程
PSS 中位 41.90 MiB、P95 42.54 MiB、最大 42.61 MiB，CPU 平均约 39.19% 单核心，
视频约 25 fps、音频约 44105 sample frames/s。旧调度那轮音频仅约 42783 frames/s；
两轮画面内容不同，不能把 CPU 差异单独归因于调度修改。
该旧系统的 OMX 软件解码器运行在应用进程，CPU 统计包含软件解码；64 位 TV
的系统解码服务资源有相当部分在应用进程之外，两个 CPU 百分比不能直接横比。
这也不替代魔盒硬件解码性能。数据在 `legacy-timing-comparison.json`。
