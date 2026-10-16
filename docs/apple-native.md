# Apple 原生适配与验收状态

2026-10-16：已实现 macOS／iOS／tvOS 的原生源码、Rust C ABI、Swift/AVFoundation
媒体适配层和 Xcode 构建入口。GitHub Mac runner 已实际编译 macOS、iOS、
iOS 模拟器、tvOS 和 tvOS 模拟器五个目标，并上传未签名应用归档。
**当前本机仍是 Linux；Apple 设备安装、画面、音频及实际播放尚未验收。**

## 架构

- `atv-core/auth/cmg/media/player` 继续负责目录、正常鉴权、密码算法、CMG、
  HLS、TS、PTS、取消、状态和试看限制。
- `native/crates/atv-apple` 将主线程驱动的 `Player` 暴露为版本 1 C ABI。
- `apple/Bridge/atv.h` 是 Swift 与 Rust 之间唯一的公开协议。
- `apple/Sources/AppleMedia.swift` 用 CoreMedia 封装 AVC／AAC sample buffer，
  交给 `AVSampleBufferDisplayLayer`、`AVSampleBufferAudioRenderer` 和
  `AVSampleBufferRenderSynchronizer` 解码、输出及同步。
- `NativePlayer.swift` 负责宿主时钟、应用生命周期及桥接；`ATVApp.swift`
  使用 SwiftUI 和 AppKit／UIKit 提供频道列表、切台、重试、全屏及 tvOS 返回键。

没有网页播放器、WKWebView、JS／WASM 运行时，也没有把签名直播 URL 交给另一个
联网播放器处理。媒体输出回调只接收解密及变换后的 AVC／ADTS 字节和归一化 PTS。

## C ABI 所有权约定

同一个 handle 的创建、tick、控制、状态读取、销毁和全部媒体回调必须在同一
线程。注册表按线程隔离，错误线程／已销毁 handle 返回错误。Swift 在主线程
创建媒体对象，以 10 ms 定时器驱动 tick；鉴权／下载仍在 Rust 的有界后台线程。

回调收到的 Rust 字节仅在当前调用中有效。Swift 创建 CoreMedia 自有内存副本。
返回 1 表示背压且没有消费输入；负值表示输出失败。context 必须一直存活到
destroy 返回，回调不得重入 `atv_player_*`。销毁会先移除当前输出、取消下载，
不会在 context 释放后继续发出媒体回调。

字幕、其他编码格式、账户登录和后台音频不是本次实现范围。Apple 输出计数要求 renderer 已进入 rendering 状态，并由
AVFoundation 接收状态与展示时钟维护，**不是独立的画面／非零 PCM 输出证据**；
不能把这些数字当作 Apple 播放验收已通过。

## 在 Mac 上构建

需要 Xcode（包含 iOS、tvOS SDK）、Rust 1.99.1、Python 3、XcodeGen。
例如先用 `brew install xcodegen` 安装工程生成工具，选好 Xcode 的开发者目录，
再从仓库根目录执行：

```sh
python3 scripts/apple-build.py --platform macos
python3 scripts/apple-build.py --platform ios-simulator
python3 scripts/apple-build.py --platform tvos-simulator
python3 scripts/apple-build.py --platform ios
python3 scripts/apple-build.py --platform tvos
```

默认构建未签名应用用于编译检查；真机安装需要 Mac 本地可用的开发者签名及
配置文件，可传 `--team YOUR_TEAM_ID`。脚本不代替配置签名或实际安装。
Apple TV 使用 **tvOS**，不是 iOS。

| 目标 | Rust target | 本工程部署最低版本 |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`、`x86_64-apple-darwin` | 13 |
| iOS 真机 | `aarch64-apple-ios` | 16 |
| iOS 模拟器 | `aarch64-apple-ios-sim`、`x86_64-apple-ios` | 16 |
| Apple TV 真机 | `aarch64-apple-tvos` | 16 |
| Apple TV 模拟器 | `aarch64-apple-tvos-sim` | 16 |

tvOS 模拟器当前生成 ARM64 包，未提供 Intel tvOS 模拟器构建。
Rust 官方文档要求对应的 Xcode SDK；ARM64 tvOS 的 std 可用 rustup 安装：
[Rust tvOS target 支持](https://doc.rust-lang.org/rustc/platform-support/apple-tvos.html)。

脚本依次编译真实 Rust staticlib、按 SDK 合并架构、用 `project.yml` 生成
Xcode 工程、编译 Swift 应用，再检查 app 资源及 Mach-O 直接依赖。
输出位于 `apple/build/DerivedData/Build/Products/`，构建及依赖报告在
`apple/build/*-build.json`／`*-dependencies.json`。报告把构建成功和播放成功分开。

## 已验证与待验收

本机已通过共享 Rust 测试、Apple ABI 的 Rust 测试，以及使用公开 C 头文件链接
真实 Rust release staticlib 的 C 程序检查。覆盖布局、字符串输出缓冲区、
线程归属、无效 handle 和销毁后的访问；没有启动虚假的 Apple 解码器。

五个 Apple 目标已在 [GitHub Actions 成功构建](https://github.com/kaigedong/atv/actions/runs/38859480388)，
包含 Rust 静态库、Swift／AVFoundation 的真实 Xcode 编译、Mach-O 依赖与资源检查。
首次云端编译发现的 tvOS 键盘快捷键 API 问题已修正，tvOS 全屏切台改用遥控移动命令。
AAC sample buffer 输出、遥控器焦点、A/V 同步、后台恢复和长时间播放仍需在 Mac／Apple TV 上验收。
Linux 构建入口遇到 Apple 目标会明确报缺少 Mac／SDK，不生成占位安装包。

使用的 Apple API 约定参考：
[Audio renderer](https://developer.apple.com/documentation/avfoundation/avsamplebufferaudiorenderer)、
[sample enqueue](https://developer.apple.com/documentation/avfoundation/avqueuedsamplebufferrendering/enqueue(_:))、
[AAC sample buffer](https://developer.apple.com/documentation/coremedia/cmaudiosamplebuffercreatereadywithpacketdescriptions(allocator:databuffer:formatdescription:samplecount:presentationtimestamp:packetdescriptions:samplebufferout:))。
