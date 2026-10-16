# 公开发布与隐私边界

公开仓库从清理后的当前原生源码快照开始。原始开发历史保留在本地私有归档分支和
忽略目录中的 Git bundle，不上传旧分支、标签或包含个人邮箱的提交元数据。
本地历史报告中的旧提交编号不代表公开仓库中的同名提交。

## 发布内容

- 原生 Rust 核心、Android 界面/JNI、Linux 宿主、Apple Swift/C ABI 与构建脚本。
- 公开频道目录、接口事实清单和不含鉴权响应的匿名性能摘要。
- 固定时钟和 `fixture-*` 输入生成的合成鉴权测试向量、合成 CMG 向量。
- FFmpeg 生成的 Baseline AVC/AAC 组件样本；不包含原站直播录制。
- Gradle Wrapper、锁定依赖与 CI 工作流。

测试数据的 GUID、票据和签名为合成输出。鉴权测试完整文件及两个必要二进制输入
由发布检查固定 SHA-256，改变样本后需要重新审查并更新摘要。
浏览器发布的固定客户端协议参数保留在实现中；运行时账号 Cookie、SDK Token、
播放 Token、签名媒体地址和持久化设备 GUID 不属于这些常量。

应用 ID `cn.bobo.atv.nativeplayer` 与 Apple bundle ID 是已有公开产品标识，
保留以维持安装兼容；它们不包含账号凭证。个人 home 目录路径已改为环境变量或标准发现。

## 排除内容

原站 JS/WASM 二进制及提取副本、HAR/PCAP、真实直播样本、Cookie/Token/签名 URL、
运行身份文件、个人会话记录、设备型号/指纹/连接地址调查、原始运行日志、
签名 keystore/私钥/描述文件、环境文件、SDK/NDK/缓存和本地产物不进入源码仓库。
CI 只上传明确指定的全新构建产物，不上传整个工作目录、缓存或运行鉴权数据。

Android debug key 由 runner 临时生成，仅用于试装；keystore 不上传。
Android 正式 release 使用 GitHub `android-release` Environment 中的专用项目签名密钥，
仅 main 的独立任务可读取；源码只公开证书指纹，私钥与密码不在本地或 Git 中保存。
runner 为签名临时还原密钥，使用后清理；产物不含密钥库，PR 不接触 Secrets。
Apple 应用继续不使用个人签名凭证。详见 [固定签名配置](ci.md#android-固定发布签名)。

## 检查与后续提交

```sh
python3 scripts/check-publication.py
python3 scripts/check-publication.py --history
python3 -m unittest discover -s scripts/tests -v
python3 scripts/install-publication-hook.py
```

本地 pre-push 钩子检查每个拟上传 ref 的完整可达历史，包括标签和旧开发分支。
检查器副本存放在 `.git/` 中，切换到旧分支后仍有效，避免 `push --all` 意外公开旧历史。
GitHub CI 再执行同样的历史检查、Gitleaks 与 workflow 语法检查；输出只包含位置和规则，
不回显敏感值。公开提交使用 GitHub noreply 邮箱。

Gitleaks 只豁免已审查的合成向量文件，以及浏览器客户端那个精确固定协议常量；
没有关闭通用密钥扫描。合成文件另有完整摘要检查约束。
这些检查不能识别所有形式的个人信息，新样本、日志、配置与例外仍需要人工审查。
本地 `test-artifacts/publication/` 保存审查记录和原始文档备份，不随公开仓库上传。
