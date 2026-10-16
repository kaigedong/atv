# GitHub 自动构建

工作流为 `.github/workflows/native-portability.yml`，在 main push、面向 main 的 PR
以及 Actions 手动触发时执行。先通过发布与隐私检查，再并行测试和构建各平台。
所有 Actions 固定完整提交 SHA；工作流仅有 `contents: read` 权限，checkout 不持久化凭证。

2026-10-16 的 [完整成功运行](https://github.com/kaigedong/atv/actions/runs/38863693642)
对应代码提交 `8870dfc`：13 项检查全部通过，9 个 artifact 已上传，
其中 `atv-android-signed` 提供五种正式签名 APK。
本轮后的纯文档状态同步提交使用 `[skip ci]` 避免重复编译，应用与构建源码仍与该成功运行一致。

| 目标 | 产物 / Actions artifact | 构建范围 |
| --- | --- | --- |
| Android 4.4+ | `atv-android` | 通用四 ABI 和 ARMv7、ARM64、x86、x86_64 单 ABI；各含 debug 与 unsigned release APK |
| Linux x86_64 / ARM64 | `atv-linux-x86_64`、`atv-linux-aarch64` | 原生 release 宿主、安装说明、摘要；Ubuntu 24.04 系统库 |
| macOS 13+ | `atv-apple-macos` | ARM64 + x86_64 通用 `.app.tar.gz` |
| iOS 16+ | `atv-apple-ios`、`atv-apple-ios-simulator` | 真机 ARM64；模拟器 ARM64 + x86_64 |
| tvOS 16+ / Apple TV | `atv-apple-tvos`、`atv-apple-tvos-simulator` | 真机 ARM64；模拟器 ARM64 |
| Windows / macOS / Linux 核心 | 不生成 Windows 桌面应用 | Rust 单元测试、Clippy、格式与平台边界检查；Linux 额外运行真实 C ABI consumer |

在 GitHub 成功运行页面底部 **Artifacts** 中下载。产物保留 14 天；过期后可以手动重跑。
Android 包名标明 ABI 与 debug / release-unsigned；选择旧 ARMv7 盒子时使用 `armeabi-v7a`。
每个产物目录包含 manifest 与 SHA256SUMS；Android 额外包含 APK 的资源/DEX/ELF 依赖检查。

Debug APK 使用临时 runner key，不同运行之间可能需要卸载重装。
Unsigned release APK 需要自己的签名后安装。main 的新增签名任务另外提供
`atv-android-signed` artifact，包含五种 ABI 组合的正式签名 release APK。
该任务等待隐私检查、三个宿主的共享核心检查和 Android 构建全部通过；PR 不执行签名。
Apple 真机产物为未签名 `.app` 归档，
不是可直接安装的已签名 IPA；需要对应开发签名与设备配置。macOS 归档未做公证。
应用归档使用 tar 保留执行权限和符号链接，并去掉构建用户的 uid/gid/用户名。

## Android 固定发布签名

正式签名身份保存在 GitHub `android-release` Environment，只允许 `main` 分支部署，
不要求每次人工批准。环境 Secrets 为 `ATV_ANDROID_KEYSTORE_BASE64` 和
`ATV_ANDROID_KEYSTORE_PASSWORD`。PKCS12 密钥库采用 RSA 3072 位、50 年证书，
别名与公开证书 SHA-256 指纹固定在 `app/signing-certificate.json`。
初始化时密钥在内存中生成，经 stdin 交给 GitHub CLI 加密上传；没有创建本地私钥文件。
初始化工具不属于仓库或日常 CI，日常任务缺少 Secrets 会失败，绝不会生成替代密钥。

只有独立签名步骤读取 Secrets，在临时 runner 的私有目录恢复密钥库，完成后清理，
失败或取消时另有清理步骤。签名环境不缓存密钥、不上传私钥或密码、不输出工具私密诊断。
GitHub Secrets 不能直接读回，删除或覆盖唯一的密钥后无法继续给旧安装签名升级。

五个正式 APK 均检查 v1/v2/v3 签名、固定证书指纹、16 KiB ZIP 对齐，并重新运行
资源/DEX/ELF 依赖检查。v1 为 Android 4.4 所需；SHA256SUMS 与 manifest 对应签名后文件。
公开产物只有 APK、检查 JSON、manifest 和 SHA256SUMS。原有 Debug 与 unsigned artifact
继续保留用于调试；临时 Debug 包切换到正式签名包需要卸载重装，应用本地数据随卸载清除。

此步骤只接入固定签名和可安装构建产物；按 commit 递增版本、GitHub Release 发布以及
客户端检查更新仍是 issue #1 的后续工作，当前 `versionCode` 尚未自动递增。

Rust 固定 1.99.1。Android 使用 JDK 21、SDK 36、Build Tools 35.0.0，
32 位 NDK 25.2.9519653/API 19，64 位 NDK 29.0.14206865/API 21。
每种 APK 都经过正式构建图的强符号链接、JNI 导出和原生依赖检查。
Apple 使用 macOS runner 的 Xcode 与 XcodeGen，Swift 语言兼容模式为 5。

CI 不调用央视频服务、不提供账号 Cookie、不执行原始 JS/WASM，也不伪造真实直播验收。
产物构建成功只证明编译、链接和包检查成功；真机播放、厂商解码能力和性能仍需运行测试。
具体本地测试结果见 [Android 兼容记录](android-kitkat.md) 和 [内存优化测量](memory-optimization.md)。

本地复现产物构建：

```sh
python3 scripts/ci-build-android.py
cargo build --release --locked --manifest-path native/Cargo.toml -p atv-desktop
python3 scripts/ci-package-native.py --platform linux --architecture x86_64
# 在 Mac 上：
python3 scripts/apple-build.py --platform tvos
python3 scripts/ci-package-native.py --platform tvos
```

输出位于忽略目录 `ci-dist/`。只通过普通测试的合成 fixture 是公开输入，运行身份文件和
真实媒体抓包不进入 workflow artifacts。详细规则见 [发布与隐私](publication.md)。
