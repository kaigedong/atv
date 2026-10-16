# 原 WASM 接口与逻辑确认范围

核对日期：2026-10-16。对象是此前固定哈希的三个原始 WASM 模块；本次复用
已保留的 REA / WABT 证据，重新校验原文件 SHA-256、长度和 decoded WAT
哈希，并整理全部 typed interface entries。没有重新下载或执行 WASM。
本次原 JS 包装层的合成探针只检查包装函数的调用转发，不充当 WASM 函数
返回语义或真实播放证据。

## 可以完整确认的接口层

完整机器可读清单：[wasm-interfaces.json](../research/oracles/wasm-interfaces.json)。
清单保存每个导入／导出的模块名、名称、种类、函数／内存／表／全局索引、
签名或限制、原 WAT 声明以及已能直接对应的 JS 包装名称。

| 固定模块 | 导入项 | 导出项 | typed interface 状态 |
| --- | --- | --- | --- |
| CMG 媒体模块 | 58：51 个函数、5 个 global、1 个 memory、1 个 table | 38 个函数 | complete，issues 为空 |
| OpenAPI keygen | 3 个函数 | 6 个函数、1 个 memory | complete，issues 为空 |
| ticket | 39 个函数、1 个 memory、1 个 table | 8 个函数 | complete，issues 为空 |

这证明固定模块的全部导入／导出 ABI 已列出，不代表所有内部函数或经 table
间接调用的行为均已理解，也不代表其他版本的接口完全相同。

| 模块 | 原始 SHA-256 | REA evidence |
| --- | --- | --- |
| CMG | `dba01d8debba97420dd1a498059459bde7107a2aaf4bb003ae7e04cf39c1ed35` | `ev_a9265bffaafd165171559dc3b07392dfd8d43df464c583003453203126a2a5c2` |
| OpenAPI | `d977605f3e99879658a332376c55b1dd9e4c89fdc8108623d6252f72a9efed5a` | `ev_ec6bcd7ea870500a775e1a2768ad0c59296e240c1ba6331dc4e511bf729115f4` |
| ticket | `87c07fafe19e59ceb39a216913a00da5d10b14aa20177beb9d60562efff0cd34` | `ev_c9cbb1460067df7263fc1d5b811c47de8d9d5e91b0b7ccc89f79fb9c02e2b795` |

## 参数和返回值需要分两层

ABI 中的 `i32` 能明确确定为 32 位整数，却不会自动注明它表示地址、长度、
标志、句柄还是输出结构地址。`void` 也不意味着没有结果：函数可能修改
内存、输出缓冲区或全局状态。需要结合 JS 包装、内存读写、导入函数和
不同输入的行为对照才能确认业务含义。

已经核对的具体例子：

| 导出／JS 包装 | 机器签名 | 已确认的业务契约／边界 |
| --- | --- | --- |
| OpenAPI `get_signature` / `get_token_rnd` | `(i32) → void` | 参数为输出结构地址；结果通过该地址的 `ptr`、`len` 两个 i32 写回，包装层读取 UTF-8 字符串、释放字符串内存并恢复栈。计算还读取宿主提供的状态，并非只有这个显式参数 |
| ticket `T` / `_aes_encrypt_ctr` | `(i32 × 7) → void` | 包装／oracle 传入六个字符串地址和输出缓冲区；在正常票据路径写回 CTR 字节，不直接返回票据字符串。实际算法输入及时间依赖见 `rust-auth.md` |
| CMG `ma` / `_CMG_jsdecLive8` | `(i32 × 4) → i32` | JS 转发名称已确定；真实 AVC Live 路径已做内存输入／输出对照及 Rust 重写。所有调用状态、错误码、异常输入的语义未据此宣布完备 |
| CMG `ba/ca/da` | `(i32) → i32` | 包装名称依次为 `_CMG_InitPlayer`、`_CMG_UnInitPlayer`、`_CMG_UpdatePlayer`；包装层构建／释放内存和调用次序可见，不能仅凭名称认定完整状态机与返回码含义 |
| CMG `Ca/Ea/Aa/Ba` | `(i32) → i32` 或 `(i32) → void` | 对应 `_jsmalloc/_malloc/_free/_jsfree`；属于运行时内存管理，不是四个独立直播算法 |

ticket 的全部 JS 导出别名已从明确的转发语句对应到 P–W：
`___wasm_call_ctors`、`___em_js__is_browser_environment`、`_malloc`、
`_get_encrypt_str_len`、`_aes_encrypt_ctr`、`_aes_decrypt_ctr`、
`___errno_location`、`_free`。别名对应是静态事实，不代表每个函数内部逻辑
均已验证。

CMG 的 38 个导出均能对应 JS 包装名，包括生命周期函数、
`_CMG_jsdecLive0..8`、`_CMG_jsdecVOD0..8`、内存／栈／异常处理和
`dynCall` 辅助函数。HLS glue 中的包装名称有混淆或误导性：例如
`MpegAudio` 实际转发至当前 AVC 路径使用的 `_CMG_jsdecLive8`。
因此判断职责依据实际调用与数据变化，不依据函数名猜测。

## 逻辑确认程度

| 层次 | 当前证据 | 不能扩大成的结论 |
| --- | --- | --- |
| 导入／导出 ABI | 三个模块完整 typed interface、固定原文件及 WAT 哈希 | 所有高级参数语义和全部内部逻辑已理解 |
| 正常签名／票据 | JS/WASM 包装、静态分析、12 组合成输入共 84 项原组件差分、真实正常鉴权成功 | 任意 SDK 版本、测试模式和未出现的动态 JS challenge 均支持 |
| AVC CMG Live 1/2 算法 | 154 组合成 NAL 差分、627 个真实 mode 1 NAL 对照、实际正常直播解码／播放 | VOD、HEVC／AVS、所有音频变换、真实 mode 2 广播及所有异常分支均已验证 |
| 完整 CMG 内部逻辑 | 已识别 440 个内部函数、解释器、由 JS 初始化的程序／数据区域，并追踪所需 Live 路径 | 440 个函数及外部程序每个分支均已逐一完成语义证明 |

CMG 的部分逻辑在自定义解释器及 JS 初始化的程序／数据中，所以仅将 WASM
解码成 WAT 仍不足以理解整个算法。需要把包装层、内存布局、宿主导入、
调用历史和解释器程序一起研究。已有 Live 路径已替换为可读 Rust 算法，
没有把这个解释器或原程序数据移植进生产应用。

后续逐函数核对可按“完整 ABI → 参数与内存契约 → 正常／边界／失败样本
→ 内部算法 → Rust 对照测试”推进。完整接口表已经具备；全部函数的高级
语义与全部分支逻辑仍是部分完成，不能承诺静态反编译自动恢复原工程。

算法细节及验证范围见 [rust-auth.md](rust-auth.md)、[rust-cmg.md](rust-cmg.md)。
REA 的静态接口与语义证明边界见其
[WASM artifact workflow](https://github.com/morluto/rea/blob/main/docs/wasm-artifacts.md)。
本报告和清单仅用于研究，不改变应用源码、依赖或 APK。
