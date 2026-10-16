# 直播播放链路内存优化

APK 压缩文件大小与运行内存是两个指标。本轮优先减少应用自己持有的媒体缓冲，
保留现有分辨率、两事件下载通道、音视频时间戳和缓冲字节硬上限。

## 代码变化

- TS 的 PES 重组缓冲在校验之后原地移除头部和填充，转交已有分配。
- 每次最多处理 128 个 TS 包（24,064 字节），在输出背压下暂停后续拆包。
  下载仍按完整 HLS 分片进行；跨批次、跨 HTTP 分片的未结束 PES 继续保留。
  单个 PES 仍可能较大，24,064 字节不是所有缓冲的总上限。
- Android 视频队列接管 `Vec<u8>`；背压时保留原包及其分配，接受后转移所有权。
  系统解码器的输入缓冲仍由系统管理。Apple/Linux 使用默认借用接口保持兼容。
- Android 视频队列增加 2 秒 PTS 跨度限制，避免原有 256 包队列提前持有过多视频。
  按最大与最小 PTS 计算，支持 B 帧乱序；背压时不丢帧、不消耗原包。
  完整分片下载缓存继续保留，音频/PCM 的原有硬上限继续生效。
- CMG 的起始码转义字节移除、解密后规范化与重新插入复用同一 NAL 缓冲，
  需要扩容时只扩展该缓冲，避免同时保存多份完整 NAL。
- 对已经通过大小检查的 HTTP Content-Length 预分配响应缓冲，减少几何增长的空余容量。

## 本地媒体分配比较

同一台 Linux 主机、Rust 1.99.1 release 构建。基线为 `8c7aa58`，
两版使用完全相同的 `profile_media_memory` 示例与本地授权 TS 文件。
两份输入共 1,370,708 字节，每版重放 20 次。

| 指标 | 优化前：整段处理 | 优化后：128 包批处理 |
| --- | ---: | ---: |
| 额外 Rust 请求堆峰值 | 1,394,055 字节 | 278,859 字节 |
| 分配与重分配次数 | 121,700 | 100,840 |
| 输出访问单元数 | 13,620 | 13,620 |
| 输出媒体字节数 | 25,449,140 | 25,449,140 |
| 输出数据与时间戳摘要 | 1997974763362482420 | 1997974763362482420 |
| 处理结束后的额外请求堆 | 0 | 0 |

该样本的额外处理堆峰值降低约 80%，分配次数降低约 17%。这不是整机内存降幅。
计量通过示例专用全局分配器记录请求大小，两版都排除了预先加载输入文件的分配。
不包含网络、系统解码器、Surface、分配器元数据，也不是 RSS/PSS 测量。
样本与计量结果保留在本地忽略目录，不提交媒体、签名地址或鉴权信息。

```sh
cd native
cargo run --release -p atv-player --example profile_media_memory -- \
  128 ../research/native-cmg/authorized-sample-1.ts \
  ../research/native-cmg/authorized-sample-2.ts
```

参数 `0` 表示整段处理；比较原始实现必须使用基线版本的源码，
仅在优化后源码上改成 `0` 不会恢复旧的 PES/CMG 实现。

## 渲染缓冲诊断

原生状态 JSON 新增 `buffers`，运行测试会采集这些字段：

| 字段 | 含义 |
| --- | --- |
| `fragmentCapacityBytes` | 当前处理中完整分片的 Vec 容量 |
| `demuxCapacityBytes` | TS 拆包器保留的 PSI/PES 字节缓冲容量 |
| `pendingPacketCapacityBytes` | 等待提交的访问单元字节缓冲容量 |
| `outputCapacityBytes` | Android 自有视频、AAC、PCM、ADTS、SPS/PPS 缓冲容量 |
| `renderCapacityBytes` | 上述容量之和 |
| `renderPeakCapacityBytes` | 本次播放在批次提交前后采样到的最大容量之和 |

计量排除下载通道、在途 HTTP/TLS、容器元数据与系统解码器、显示和音频分配。
峰值没有在每次临时分配时采样，不能解释为完整内存峰值。
Apple/Linux 输出适配器暂未上报平台队列容量，输出项返回 0 表示未计量。
暂停或失败释放播放链路后，当前容量归零；失败时保留最后一次播放的采样峰值。

`scripts/summarize-android-performance.py` 会在报告含这些字段时，
同时汇总渲染负载缓冲容量与已有的 PSS、Java/native heap、CPU、音视频进度。

## Android TV 同设备直播比较

SDK 36 官方 x86_64 Android TV 模拟器，同一 CCTV13 频道、960×540，
两版各完成 600 秒真实直播，排除最初 60 秒；各 110 个样本，计量时段约 540.6 秒。
旧 APK 为 `eb1c0d0185ff98dcba7f525ebf4d4613f225f5e925b3c274ca80aae35affead1`，
新 APK 与下面的四 ABI Debug 产物校验值一致。

| 指标 | 优化前 | 优化后 |
| --- | ---: | ---: |
| 应用进程 PSS 中位 | 57.15 MiB | 55.99 MiB |
| 应用进程 PSS P95 | 59.07 MiB | 57.35 MiB |
| 应用进程 PSS 最大 | 60.15 MiB | 58.04 MiB |
| Native heap allocated 中位 | 27.95 MiB | 28.49 MiB |
| Java heap used 中位 | 3.53 MiB | 3.62 MiB |
| CPU，单核心百分比 | 6.41% | 5.44% |
| 视频输出帧/秒 | 25.00 | 25.00 |
| 音频输出采样帧/秒 | 44,097 | 44,100 |

本轮 PSS 中位减少约 1.17 MiB（约 2%），整体收益较小。
Native heap allocated 与 PSS 的统计口径不同，本轮前者略升，不能相加、相减归因。
固定本地样本的临时处理堆与分配次数收益应单独理解。
实时直播码率、内容、系统共享页和后台负载会波动，各一次测试不保证所有频道或设备同等收益。
进程 PSS/CPU 不包含全部系统 codec 服务和 GPU 占用，也不能证明长期无泄漏。

优化后，输出自有媒体缓冲容量中位 0.877 MiB，
所计量渲染负载缓冲容量之和中位 1.714 MiB、采样峰值 2.631 MiB。
计量范围见前文，下载通道、在途请求和系统分配不包含在这些容量中。
原始报告保留在本地 `test-artifacts/memory-optimization/`，
比较汇总为 `performance-comparison.json`。

## 当前构建产物

| 文件（位于 `artifacts/`） | 字节数 | SHA-256 |
| --- | ---: | --- |
| `atv-android-debug.apk`，四 ABI | 6,840,164 | `09023b5e6fe496d2b4c0383f000e74526761cc87d60e0bc53c7131ed5ad559be` |
| `atv-android-kitkat-armeabi-v7a-debug.apk`，单 ARMv7 | 1,402,485 | `0cdb102e7c40e38a73d7b18ea912e352a9a12b92b4a3e1db471ddab00b00d4d8` |
| `atv-android-release-unsigned.apk`，四 ABI | 6,436,363 | `939f7db62e851c42e81fc6287af54351b1f94abca730fee3d6c4b1fdd7d5d3db` |

单 ARMv7 APK 仍约 1.34 MiB。Release 文件未签名，只完成构建与包检查。
四 ABI 与单 ARMv7 包都通过最低 API、ELF 架构、16 KiB 段对齐和无脚本运行时检查。

## 兼容性回归

- 58 项共享 Rust/Apple C ABI 测试通过；另外 6 项本地真实 TS 回归通过，
  覆盖跨分片 PES、重复包、丢包、discontinuity 与 PMT 轨道删除。
  CMG 保持既有官方模式 1/2 测试向量的逐字节结果，
  增加转义扩容边界测试与原始分配在背压重试后转移的检查。
- Linux 桌面 `cargo check` 与共享库 Clippy 通过，ARMv7 legacy Android Clippy 通过。
  Apple 平台框架未在本机编译，不能把 Rust/C ABI 测试解释为 Apple 播放验证。
- API 19 的 512 MiB x86 模拟器通过生成的 Baseline AVC/AAC 组件输出检查；
  视频 Surface 有彩色内容，音频 PCM 非零，退出后原生工作线程为零。
  这不是当前 High Profile 央视频直播在 Android 4.4 上的验证。
- API 23 的 32 位 x86 模拟器通过 15 项真实直播、切台、遥控、重试、生命周期与退出检查，
  包含 120 秒连续直播。排除前 60 秒后，PSS 中位 40.58 MiB、P95 41.03 MiB，
  CPU 37.14% 单核心，25.00 fps、44,103 音频采样帧/秒。
  输出自有负载缓冲容量中位 0.877 MiB，渲染侧所计量负载容量之和中位 1.754 MiB。
  该模拟器使用进程内软件解码，不能与 64 位 TV 模拟器直接比较性能。
- 最终 APK 在 SDK 36 Android TV 上通过 10 分钟直播及独立的 14 步切台、遥控、重试、
  生命周期和退出检查。在已有 SDK 35 Android 模拟器上通过真实断网与 UI 重试恢复，
  原始 Wi-Fi/data 状态已恢复。TV 模拟器走以太网，Wi-Fi/data 断网用例在手机模拟器执行。

目标魔盒仍未连接，本轮没有 ARM 盒子的运行内存或厂商硬件解码结果。
所有 Android 实测结果对应四 ABI Debug APK 的精确 SHA-256；单 ARMv7 包完成交叉编译与检查。
