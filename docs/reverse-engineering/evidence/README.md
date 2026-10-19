# 证据索引

本目录存放 `01`–`05` 各卷结论的原始依据。**所有内容均为源码摘录、表格转录或命令输出，不含任何 API key、token、secret 或凭据值**；遇到上游内置的字面量一律只记录其存在位置。

## 文件

| 文件 | 内容 | 对应卷 |
|---|---|---|
| `upstream-sources.md` | 本次分析实际读取过的上游源码路径清单（按目录分组） | 全部 |
| `service-endpoints.md` | 14 个非 AI 服务的端点 / 方法 / 请求体 / 长度限制 / 签名方案摘录 | `02-services.md` |
| `language-codes.md` | 各服务的语言代码表（含与 BCP-47 的差异） | `02-services.md`、`03-ai-services.md` |
| `prompt-templates.md` | LLM 服务的系统提示词、用户提示词模板与少样本清单 | `03-ai-services.md` |
| `port-module-map.md` | Windows 移植的模块 / 命令 / 事件 / 替换点清单 | `05-architecture.md` |

## 获取方式

| 证据 | 怎么得到的 |
|---|---|
| 上游源码路径 | 对 `C:\Users\12064\AppData\Local\Temp\ezsrc\Easydict-dev\Easydict\` 逐目录 `ls`/`find`，再用 Read/Grep 打开具体的 `.swift`/`.m`/`.h` 文件 |
| 端点与限制 | 直接读服务实现文件里的字符串常量（如 `kYoudaoTranslateURL`、`TEXT_LIMIT`）与请求构造代码；长度上限按上游的 `prefix(_:)` / `trimmingToMaxLength(_:)` 判定其计数单位后转录 |
| 语言代码表 | 逐服务的 `supportLanguagesDictionary`（Swift）或 `const XXX: &[(Language, &str)]`（移植）整理成表；差异项在表内加粗并加注 |
| 提示词 | 从 `Swift/Service/OpenAI/StreamService+Prompt.swift`、`AITool/PolishingService.swift`、`AITool/SummaryService.swift` 逐字复制系统提示词；少样本只登记条数与首行（体量原因），移植侧的逐字副本见 `app/src-tauri/src/service/llm/prompts.rs` |
| 移植侧信息 | 直接读 `app/src-tauri/src/**` 与 `app/src/**`，命令/事件名与常量取自源码字面量 |
| 界面常量 | 读 `objc/ViewController/**`（窗口、结果视图）、`Swift/View/**`（设置）、`App/EZConst.h` 与 `Swift/Utility/Constants/AppConstants.swift` |

## 说明

- 上游仓库为 `tisfeng/Easydict`（GPL-3.0），本次分析的本地副本位于 `C:\Users\12064\AppData\Local\Temp\ezsrc\Easydict-dev\`；证据文件里的路径均以该副本为准。
- 少数源码字面量在阅读链路中显示为脱敏占位符（例如签名算法名与某个待签字符串终结符）。本目录**只描述它们在算法中的位置与作用**，不复述其值，也不做还原；公开协议常量（如 TC3 的算法标识、Volcengine 的 `request` scope）按协议名描述。
- 上游内置密钥的去混淆结果见 `01-overview.md` §4，本目录不重复。
