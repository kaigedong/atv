# ATV · 原生电视直播

默认 Android 工程是根目录 `app/`，直播核心使用 Rust，界面使用 Android 原生控件与少量 Java/JNI。生产运行不依赖 JS、WebView、官方 WASM 或脚本引擎。

## 自动构建与下载

推送 `main`、提交 PR 或手动运行 [GitHub Actions](https://github.com/kaigedong/atv/actions/workflows/native-portability.yml) 会执行发布检查、测试和多平台构建。
在成功运行页面的 **Artifacts** 中下载 Android 通用及单 ABI APK、Linux 原生宿主、macOS / iOS / tvOS 应用归档。
Android 支持 `armeabi-v7a`、`arm64-v8a`、`x86`、`x86_64`；Apple 包默认未签名，真机安装需要自己的签名配置。
main 另外生成 `atv-android-signed`，提供使用固定项目密钥签名的五种 Android release APK；密钥仅存于 GitHub Environment Secrets。
具体目标、产物与限制见 [自动构建](docs/ci.md)，公开内容审查规则见 [发布与隐私](docs/publication.md)。

已验证的 [完整成功构建](https://github.com/kaigedong/atv/actions/runs/38859480388) 包含 12 项通过的检查与 8 个下载 artifact；编译通过与真机播放验收分别记录。

## Android 构建

```sh
./scripts/android-build.sh
# 或直接从根目录使用 Gradle / Android Studio：
./gradlew assembleDebug lintDebug
```

需要 Rust 1.99.1、JDK 21、Python 3、Android SDK 36、Build Tools 35.0.0 、NDK 29.0.14206865（64 位）及 25.2.9519653（32 位）。不需要 Node.js、npm 或 esbuild。工具路径可通过标准环境变量指定，本机路径由脚本识别。

默认生成 `app/build/outputs/apk/debug/app-debug.apk`，应用 ID 为 `cn.bobo.atv.nativeplayer`，最低 API 19（Android 4.4），包含 `armeabi-v7a`、`arm64-v8a`、`x86` 与 `x86_64`。

Gradle 必须先从 Rust 源码构建 `libatv_native.so`，然后检查 Debug／Release APK 的资源、DEX、JNI 导出和 ELF 对齐。若含 JS、HTML、WASM 或 WebView／脚本引擎引用，assemble 会失败。Rust 编译不能以 UI-only 模式跳过。

```sh
./scripts/android-build.sh assembleRelease          # 未签名 release APK
ATV_TEST_DEVICE=emulator-5572 ./scripts/android-runtime-check.sh
python3 scripts/check-native-architecture.py
```

兼容入口 `scripts/native-android-build.sh` 与 `scripts/native-android-runtime-check.sh` 只转发到上述入口，不存在第二套 Android 工程。

GitHub 自动构建提供通用包与单 ABI 包；旧 ARMv7 盒子请选择 `armeabi-v7a`。
本地 `artifacts/` 为忽略目录，不随源码发布。默认构建迁移记录见 [docs/main-native-build.md](docs/main-native-build.md)。

## 多平台核心

共享 Rust 库负责目录、正常鉴权、签名／票据、CMG 压缩媒体变换、HLS、TS 解复用、播放状态、取消和试看限制。

- Android 适配层：32 位通过 Rust/JNI 调用系统 MediaCodec／AudioTrack；64 位使用 NDK MediaCodec，运行时加载 AAudio，Android 5–7 回退 AudioTrack。
- Linux 桌面适配层：GStreamer 解码、SDL2 原生窗口／音频／键盘，已验证真实音视频。
- Apple 适配层：共享 Rust 核心＋C ABI＋Swift/AVFoundation，macOS、iOS、tvOS 及模拟器五个目标已由 GitHub Mac runner 实际编译并生成未签名应用；设备安装与播放尚未验收。
- Windows：共享核心测试与检查已在 CI 通过；尚无 Windows 桌面宿主。

```sh
./scripts/native-desktop.sh
./scripts/native-desktop.sh --build-only --release
```

Linux 需要 SDL2 / GStreamer 系统库和 AVC/AAC 插件。左右切台，Enter 全屏，Escape 返回，P 暂停／恢复，R 重试，Q 退出；目前为最小播放宿主，尚无图形频道侧栏。详见 [多平台架构与平台状态](docs/multiplatform.md)。

## Apple 构建

在安装 Xcode、XcodeGen 和 Rust 1.99.1 的 Mac 上：

```sh
python3 scripts/apple-build.py --platform macos
python3 scripts/apple-build.py --platform ios-simulator
python3 scripts/apple-build.py --platform tvos-simulator
# 真机目标（默认仅编译未签名应用；安装需本机签名配置）：
python3 scripts/apple-build.py --platform tvos --team YOUR_TEAM_ID
```

Apple TV 使用 tvOS。iOS／tvOS 最低 16，macOS 最低 13；ARM64 tvOS 模拟器使用独立 Rust target。
构建说明和未验证范围见 [Apple 原生适配](docs/apple-native.md)。

## 验证与支持边界

Android 4.4 兼容层已通过 Baseline AVC/AAC 组件输出测试；旧模拟器的软件解码器拒绝当前 High Profile 直播，目标魔盒的实际播放仍需真机验证。详见 [Android 4.4 与性能验证](docs/android-kitkat.md)。

直播链路已增加小批拆包、PES/CMG 缓冲复用和 Android 视频缓冲所有权转移。
优化测量口径与复现方法见 [直播内存优化](docs/memory-optimization.md)。

Android TV 模拟器已有真实直播、画面、非静音音频、切台、遥控器、全屏、后台恢复和失败清理证据。实体 Android TV 与物理硬件解码尚未验证。当前支持 AVC/AAC MPEG-TS HLS；其他格式及未观察的远端挑战分支不冒充已完成。

原生算法和历史运行报告：[鉴权](docs/rust-auth.md)、[CMG](docs/rust-cmg.md)、[原生验收](docs/native-runtime-check.md)、[Android 工具链](docs/rust-android.md)。

## 历史与研究

公开仓库从清理后的原生源码快照开始。旧 WebView 实验、原始开发历史、运行日志和抓包留在本地；文档中的旧提交编号对应本地开发记录。
`research/oracles/` 仅保留对照原组件的研究脚本与接口清单；Rust 测试使用已提交的合成 fixtures，生产构建不执行研究脚本，也不打包研究目录。
