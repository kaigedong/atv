> 2026-10-16：当前默认构建已增加 API 19／四 ABI 兼容与 Apple 原生宿主。下文保留此前验收记录；最新构建与性能状态见 [android-kitkat.md](android-kitkat.md)，Apple 验证边界见 [apple-native.md](apple-native.md)。

# main 默认 Android 原生构建迁移

日期：2026-10-16。已依次将 `feature/rust-native` / `8538fbb` 和
`feature/multiplatform` / `5df60d6` 快进合并到 `main`。旧 WebView 基线
`9f20444` 仍保存在 Git 历史，另建标签 `baseline/webview-9f20444`。
两个开发分支的提交全部是 main 的祖先；未删除分支，也没有向远端推送。

## review 为什么会看到 JS

review 对此前默认构建的判断正确。`5df60d6` 中仍存在两套 Gradle 工程：

- 根目录 `app/` 使用旧 `cn.bobo.atv.MainActivity`，加载 WebView；
  `app/build.gradle` 依赖 `androidx.webkit`，通过 `syncWebAssets` 打包
  `index.html`、`app.bundle.js` 和 `live-resolver.vendor.js`。
- `scripts/android-build.sh` 仍执行 npm / 网页构建，然后调用根目录 Gradle。
  根目录 `package.json` 的 `android:debug` 也指向这个入口。
- 已验证的 Rust 原生 APK 来自另一套 `native-android/` 工程，只有明确使用
  `native-android-build.sh` 才走该工程。当时的原生 APK 验证属实，但没有
  完成默认入口迁移，不能据此声称根目录默认 Android 构建也已原生化。

这次同时修正源码树、默认构建图、脚本、CI 与 README，避免继续保留这两种
含义不同的 Android 构建入口。

## 当前唯一构建链路

```text
./gradlew assembleDebug / Android Studio / scripts/android-build.sh
    → :app:buildRust
    → Python 构建调度 → Cargo / NDK Clang → libatv_native.so
    → Android 原生 Java 控件、Surface、遥控、生命周期与 JNI
    → packageDebug
    → verifyNativeDebugApk
```

Release 同样强制执行 APK 检查。Rust 下载／鉴权／签名／CMG／HLS／TS／播放
状态代码继续共享，Android 解码与音频输出仍为 NDK MediaCodec / AAudio。
Gradle / Python 是编译工具，不进入 APK；应用运行代码不调用它们。

- 原生 `MainActivity`、`NativeBridge`、资源与运行测试迁入根目录 `app/`。
- 旧 WebView Activity、debug ReferenceActivity、网页、npm manifest、
  esbuild／prepare-player／serve 脚本从当前生产源码树删除。
- 删除重复的 `native-android/` Gradle 工程。旧 native 构建／测试脚本名称
  仅作为兼容别名，转发到唯一的根目录工程。
- Rust JNI 库从源码生成到 `app/build/generated/rustJniLibs`，不从源码树
  拼装预先准备的二进制。没有 UI-only 绕过选项。
- Android 的 Java runtime classpath 无第三方依赖。DEX 中只有本项目
  `R`、`BuildConfig`、`MainActivity`／其内部与 lambda 类以及 `NativeBridge`。
- 应用资源资产仅 `assets/channels.json`；每 ABI 仅一份 `libatv_native.so`。
  动态依赖仅 `libandroid.so`、`libmediandk.so`、`libaaudio.so`、`libdl.so`、
  `libc.so`。APK 检查拒绝浏览器／JS／WASM 资源、脚本运行时 DEX 引用、
  非预期资产、非系统原生依赖、缺失 JNI 导出及无效 ELF 对齐。

原组件行为对照脚本迁到 `research/oracles/`。它使用 Node／原 WASM 的目的
仅为研究期生成合成 oracle fixtures，不属于构建或运行链路。普通 Rust
测试直接对照已提交的 JSON，不执行这些研究脚本。旧缓存已从根目录移到
忽略的研究缓存目录。协议配置中的官方网页 URL 仅为签名元数据，不会加载
网页或运行网页播放器。

## 本次实际验证

证据目录：`test-artifacts/main-native/`。这些是本机实际执行结果，新增 CI
配置尚未在远端运行。

| 验收 | 实际结果与证据 |
| --- | --- |
| 根目录直接 Gradle 干净构建 | 用失败桩禁止 node / npm / npx / esbuild，执行 `./gradlew --no-daemon clean assembleDebug assembleRelease lintDebug`；退出 0，四个工具均没有被调用。`no-node-build.json` / `direct-gradle-no-node.log` |
| 默认脚本 | `scripts/android-build.sh` Debug、Release、lint 成功；native 别名指向同一脚本 |
| 强制构建门禁反向测试 | 临时注入 `.js` 资源，真实 `assembleDebug` 在 APK 检查阶段失败；finally 删除后重新构建成功。`reject-js-asset.json` / `reject-js-asset.log` / `restore-native-build.log` |
| 原生依赖反向测试 | 在真实 ELF 的 DT_NEEDED 字符串中将系统库替换为等长的非系统库名，检查器拒绝；未执行修改后的代码 |
| APK 检查 | Debug / Release 均无 JS、HTML、WASM、WebView／脚本引擎引用；双 ABI、六个 JNI 导出、16 KiB LOAD 对齐、系统动态依赖通过。`debug-apk.json` / `release-apk.json` |
| Java runtime 依赖 | `:app:dependencies --configuration debugRuntimeClasspath` 返回 `No dependencies`；SDK 系统 API 不属于第三方依赖。`gradle-runtime-dependencies.txt` |
| 共享 Rust 测试 | 50 项普通测试通过；不需要 Node 或原 WASM |
| Android TV 实际运行 | 根目录构建的 Debug APK 在官方 API 36 Android TV x86_64 模拟器上 14 步 smoke 全通过：两轮八次切台、遥控器／全屏切台／返回、服务端拒绝／重试、后台恢复、Surface 重建与退出清理。`tv-smoke/result.json` |

本次移动工程没有更改生产 Rust 算法或 Java 播放界面逻辑。新 Debug APK
所有 ZIP 条目内容与上一轮已验收的 `b7aaff…` 原生 APK 相同；归档／签名
容器哈希因构建工程迁移不同。本次仍单独验证新默认 APK，没有把旧 WebView
运行结果充作新版本的证明。

十分钟及外部非静音音频等历史记录分别见
[native-runtime-check.md](native-runtime-check.md) 和
[multiplatform.md](multiplatform.md)，保留各自 APK 哈希。
实体 Android TV 与物理硬件解码仍未验证。

## 当前产物

| 产物 | SHA-256 |
| --- | --- |
| `artifacts/atv-android-debug.apk` | `f06b4f144f7ccf27ff1592a42b81b885cf9577cb2c06d4bb9c4938c605727428` |
| `artifacts/atv-android-release-unsigned.apk` | `7a48d3ef3a45f281f6d7e8446717cd87aa105decd79e731f59f932b073270e68` |

Debug 可直接安装；Release 尚未配置正式签名。默认输出目录为
`app/build/outputs/apk/`。两个新产物旁附 `.sha256` 文件。
