# Easydict 逆向分析报告 — 总览

> 目标：以 `tisfeng/Easydict`（macOS, GPL-3.0）为蓝本，在 Windows 上重建一个行为等价的翻译/词典应用。
>
> 分析方法：使用 **REA**（`rea-agents` 6.5.0）提供的逆向工作流 + GitHub REST API 静态分析 `dev` 分支源码。
>
> 分析日期：2026-10-16

---

## 0. 方法与工具链（REA）

### 0.1 REA 是什么

`morluto/rea` 是一个 **MCP 逆向工程工具集**（MIT），把「分析原生二进制 / JavaScript-Electron / .NET / 网站」的能力接给 AI agent。

本机已安装并验证：

```bash
npm install --global rea-agents     # -> rea 6.5.0
rea --help
rea doctor
```

`rea doctor` 在 Windows 上的探测结果：

| 检查项 | 结果 | 说明 |
|---|---|---|
| `host` | ✅ `win32 x64` | 平台受支持 |
| `node` | ⚠️ `22.17.1` | REA 要求 `>=22.19` / `>=24.11`，仅告警不影响 CLI |
| `hopper` | ❌ | macOS 专有反汇编器，Windows 不可用 |
| `ghidra` | ❌ | 未设置 `GHIDRA_INSTALL_DIR`；**Windows 上可通过它做原生分析** |
| `ida-registration` | ❌ | 未配置 |
| `jadx` | ❌ | 未设置 `REA_JADX_MCP_JAR` |
| `skill:identity` / `registration:*` | ❌ | 未运行 `rea setup` |

结论：**Windows 上可用的 REA 能力**为
1. `analyze-javascript-application` / `analyze-web-bundle`（静态 JS/Electron 重建，**无需反汇编器**）
2. `inspect-managed-artifact` / `inspect-managed-members`（.NET PE/CLI，**无需反汇编器**）
3. `inspect-pe-resources`（PE 资源/图标）
4. `inspect-sqlite-database`
5. `inspect-go-binary` / `inspect-rust-binary`
6. 配置 Ghidra 后的全套原生分析（`decompile` / `function` / `xrefs` / `trace` …）

### 0.2 本次分析的实际技术路线

Easydict 是 **Swift + Objective-C** 的 macOS 原生应用：

- 仓库语言构成：Swift / Objective-C / Metal
- 发布物：`Easydict.dmg` (35.7 MB)、`Easydict.zip` (36.1 MB)，**仅 macOS 二进制**

这决定了分析策略：

| 路线 | 可行性 | 采用 |
|---|---|---|
| 下载 `.dmg`/`.zip` 做 Mach-O 原生反汇编 | 本机无 Hopper/IDA，且 github release 资产下载在本环境超时 | ❌ |
| 用 Ghidra 分析 Mach-O | Ghidra 未安装，且 Mach-O + Swift 元数据成本极高 | ❌ |
| **GitHub REST API 静态阅读源码**（等价于 REA 的静态重建工作流） | ✅ 完整、精确、可引用 | ✅ **主路线** |
| 抓取运行中应用的网络请求 | 无 macOS 运行环境 | ❌ |

> 说明：REA 在 macOS 上针对此类目标会走 `analyze`（原生）+ `observe-native-ui` + `inspect-packet-capture` 的组合。
> 在 Windows 且无原生分析引擎的条件下，**带证据的源码级静态重建**是等价且信息量更高的路径——它给出的是精确的 URL、密钥、签名算法与提示词，而非反汇编推断。
> 本文档所有结论均标注了来源文件路径，可逐条复核。

---

## 1. 产品定位

Easydict 是一个**划词/输入/截图三种触发方式**的词典 + 翻译聚合器：

- 20+ 服务并发查询，结果并列展示
- 自动语言识别
- 52 种语言
- 查询历史 / 收藏
- TTS 发音
- AI 翻译 / 润色 / 总结（流式 + Markdown 渲染）
- 本地词典（Apple Dictionary、MDict）

默认启用：**Youdao + DeepL + BuiltInAI**。

---

## 2. 架构总览

```
Easydict
├── App/                     应用入口、AppDelegate、URL Scheme、菜单栏
├── objc/
│   ├── Service/
│   │   ├── AudioPlayer/     EZAudioPlayer（TTS 播放与缓存）
│   │   ├── Language/        EZLanguageModel / EZLanguageManager
│   │   ├── Model/           EZEnumTypes / EZConstKey / EZOCRResult
│   │   └── WebViewTranslator/
│   ├── ViewController/      所有窗口控制器
│   └── Libraries/           DictionaryKit(TTTDictionary) / …
└── Swift/
    ├── Service/
    │   ├── Model/           QueryService / QueryResult / QueryServiceFactory
    │   ├── OpenAI/          StreamService / BaseOpenAIService  ← 所有 LLM 的基类
    │   ├── Claude/ Gemini/ DeepSeek/ Groq/ Zhipu/ MiniMax/ GitHub Models/
    │   ├── Ollama/ BuiltInAI/ AITool/ CustomOpenAI/
    │   ├── ClaudeCode/ CodexCLI/ GitHubCopilot/    ← CLI 类服务
    │   ├── Youdao/ DeepL/ Google/ Bing/ Baidu/ Caiyun/ Tencent/
    │   ├── Ali/ Volcano/ NiuTrans/ Doubao/
    │   ├── Apple/          系统翻译 + NLLanguageRecognizer + NSSpeechSynthesizer
    │   └── Dictionary/     AppleDictionary / MDict / DictionaryRendering
    ├── Model/               QueryModel / QueryError / DetectManager
    ├── Feature/             Configuration / Markdown / ActionManager / OCR / …
    └── Utility/             Constants / Extensions / AppPathManager
```

### 2.1 核心抽象

**`QueryService`（`@objc(EZQueryService)`）** —— 所有服务的基类：

```swift
open func serviceType() -> ServiceType
open func name() -> String
open func supportLanguagesDictionary() -> MMOrderedDictionary
open func translate(_ text: String, from: Language, to: Language) async throws -> QueryResult
open func detectText(_ text: String) async throws -> Language

open func startQuery(_ queryModel: QueryModel) async throws -> QueryResult
open func startQueryStream(_ queryModel: QueryModel) -> AsyncThrowingStream<QueryResult, Error>
```

`startQuery` 流程：
1. `prehandleQueryText(text, from:, to:)`
   - 繁简自动转换（若 `autoConvertTraditionalChinese()` 且两侧均为中文）→ **直接返回，不请求网络**
   - 不支持的语言 → `QueryError(.unsupportedLanguage)`
   - 免费额度耗尽 → `QueryError(.api, errorDataMessage: "insufficient_quota_prompt")`
2. 否则 `translate(...)`

**`QueryResult`** 是统一结果模型，关键字段：

```swift
var translatedResults: [String]?      // 译文分段
var translatedText: String?           // joined("\n")
var wordResult: EZTranslateWordResult?  // 词典结果
    //   phonetics / parts / exchanges / simpleWords / tags
    //   etymology / synonyms / antonyms / collocation
var htmlString: String?               // 本地词典 HTML
var error: QueryError?
var isLoading / isStreamFinished / validationMessage
```

**`EZQueryTextType`** 决定查询类型：

```objc
EZQueryTextTypeTranslation = 1 << 0   // 1
EZQueryTextTypeDictionary  = 1 << 1   // 2
EZQueryTextTypeSentence    = 1 << 2   // 4
```

分类逻辑（`String+Analysis.swift`）：

```swift
if shouldQueryDictionary(withLanguage:, maxWordCount:) { return .dictionary }
if shouldQuerySentence(withLanguage:), language == .english { return .sentence }
return .translation
```

- 中文：词数 ≤ maxWordCount 且是中文词/短语 → 词典
- 英文：长度 ≤ `englishWordMaxLength` 且只有 1 个词 → 词典

---

## 3. 服务清单（全部）

| # | 服务 | ServiceType | 类型 | 密钥 | 流式 |
|---|---|---|---|---|---|
| 1 | Youdao 有道 | `Youdao` | 词典+翻译 | 无 | 否 |
| 2 | DeepL | `DeepL` | 翻译 | 无（内置网页模式） | 否 |
| 3 | Google | `Google` | 词典+翻译 | 无 | 否 |
| 4 | Bing | `Bing` | 词典+翻译 | 无 | 否 |
| 5 | Baidu 百度 | `Baidu` | 翻译 | 用户 | 否 |
| 6 | Tencent 腾讯 | `Tencent` | 翻译 | 用户 | 否 |
| 7 | Alibaba 阿里 | `Alibaba` | 翻译 | 用户 | 否 |
| 8 | Volcano 火山 | `Volcano` | 翻译 | 用户 | 否 |
| 9 | Caiyun 彩云 | `Caiyun` | 翻译 | 内置 | 否 |
| 10 | NiuTrans 小牛 | `NiuTrans` | 翻译 | 内置 | 否 |
| 11 | Doubao 豆包 | `Doubao` | 翻译 | 用户 | **是** |
| 12 | Apple 系统翻译 | `Apple` | 翻译 | 无（系统） | 否 |
| 13 | Apple Dictionary | `AppleDictionary` | 词典 | 无（系统） | 否 |
| 14 | MDict | `MDict` | 词典 | 无（本地文件） | 否 |
| 15 | OpenAI | `OpenAI` | AI | 用户 | 是 |
| 16 | CustomOpenAI | `CustomOpenAI` | AI | 用户 | 是（可关） |
| 17 | Gemini | `Gemini` | AI | 用户 | 是 |
| 18 | Claude | `Claude` | AI | 用户 | 是 |
| 19 | DeepSeek | `DeepSeek` | AI | 用户 | 是 |
| 20 | Groq | `Groq` | AI | 用户 | 是 |
| 21 | Zhipu 智谱 | `Zhipu` | AI | 用户 | 是 |
| 22 | MiniMax | `MiniMax` | AI | 用户 | 是 |
| 23 | GitHub Models | `GitHub` | AI | 用户 | 是 |
| 24 | Ollama | `Ollama` | AI 本地 | 无 | 是 |
| 25 | BuiltInAI 内置 AI | `BuiltInAI` | AI | 内置 | 是 |
| 26 | Claude Code CLI | `ClaudeCode` | AI CLI | CLI 登录 | 伪流式 |
| 27 | Codex CLI | `CodexCLI` | AI CLI | CLI 登录 | 伪流式 |
| 28 | GitHub Copilot CLI | `GitHubCopilot` | AI CLI | CLI 登录 | 是 |
| 29 | Polishing 润色 | `Polishing` | AI 工具 | 内置 | 是 |
| 30 | Summary 总结 | `Summary` | AI 工具 | 内置 | 是 |

---

## 4. 关键发现：内置密钥已还原

Easydict 把内置服务的密钥放在 `EncryptedSecretKeys.plist`，运行时 AES 解密：

```swift
// String+EncryptAES.swift
bundleName = Bundle.main.object(forInfoDictionaryKey: "CFBundleName")  // "Easydict"
key = String(bundleName.sha256().prefix(16))   // = "c198a7fbad420d5d"
iv  = key                                       // AES-128-CBC + PKCS7
```

按该方案复现解密后得到（**这是本次逆向最重要的产出之一**）：

| 键 | 明文值 |
|---|---|
| `builtInAIEndpoint` | `https://api.izual.me/v1/chat/completions` |
| `builtInAIAPIKey` | `已移除：上游内置的 OpenAI 兼容 API Key` |
| `caiyunToken` | `已移除：上游内置的彩云小译 Token` |
| `niutransAPIKey` | `已移除：上游内置的小牛翻译 API Key` |

> 上表中内置密钥的**取值已从本文档移除**：它们是第三方付费服务的凭据，GPL-3.0 只授权 Easydict 的代码，不授权这些密钥，随仓库分发会造成凭据再分发。需要复现时按上文方案解密上游 `EncryptedSecretKeys.plist`，或在源码中按「位置」列查找；本仓库的实现不内置这些密钥。


另有硬编码在源码里的常量：

| 常量 | 值 | 位置 |
|---|---|---|
| Youdao key-fetch `key` | `asdjnjfenknafdfsdfsd` | `YoudaoService+Translate.swift` |
| Youdao 词典签名 key | `已移除：上游硬编码的 MD5 签名密钥` | `YoudaoService+Dict.swift` |
| Youdao 固定 Cookie | `已移除：上游硬编码的会话 Cookie` | `YoudaoService.swift` |
| Bing 词典 appid | `371E7B2AF0F9B84EC491D731DF90A55719C7D209` | `BingConfig.swift` |
| Google 初始 TKK | `444000.1270171236` | `google-translate-sign.js` |
| Google TTS rpcid | `jQ1olc` | `GoogleService.swift` |
| DeepL App Store id | `1552407475` / `com.linguee.DeepLMobileTranslator` / `DeepL SE` | `DeepLWebAppVersionProvider.swift` |

> ⚠️ **许可与合规提示**：Easydict 为 GPL-3.0。内置密钥属于项目方自费提供的公共配额，直接复制到自研产品中会消耗他人资源，且服务条款可能不允许。本仓库的实现**默认不内置这些密钥**，用户需自行填写（各服务的凭据字段见 `05-architecture.md` 的服务表）。

---

## 5. 文档索引

| 文档 | 内容 |
|---|---|
| `01-overview.md` | 本文 — 方法论、架构、服务清单、密钥还原 |
| `02-services.md` | 14 个非 AI 服务的完整请求/签名/解析规格 |
| `03-ai-services.md` | 15 个 AI 服务的端点、请求体、**逐字提示词**、SSE 解析 |
| `04-ui-spec.md` | 窗口、视图层级、快捷键、设置项、字符串 |
| `05-architecture.md` | 查询编排、存储、OCR、划词、URL Scheme、本地化 |
| `06-port-gaps.md` | **移植缺口登记**：已实现但偏离上游、或声明了却不生效的功能（逐条带 `文件:行`） |
| `evidence/` | 原始证据文件（源码摘录、解密产物） |

---

## 6. 已知未确定项

以下内容在本次分析中**未能确定**，Windows 实现时需自行设计：

> **上游 Swift 源码现在可查**：本机 `%TEMP%\ezsrc\Easydict-dev`（428 个 `.swift`，含 `EasydictTests/`）。下面的条目已按源码逐条复核；仍未确定的都注明了原因，凡有源码可查的一律不再靠推测。

1. **`dictionary-result.html`** 的完整 JS（17 KB，未逐行转录）
2. **`String.handlingInputText()` 中的 URL / 文件路径识别**：管线本身（分词 → 去注释符 → 换行替换 → 末尾 trim）已按 `String+HandleInputText.swift` 实现于 `src/util/input_text.rs`，并接入查询入口（`commands.rs:526` 的 `build_query_model`，对应上游 `QueryModel.swift:99`）；上游还会在此识别 URL 与文件路径，该部分仍未实现。
3. **MDict LZO 解压器** 的完整状态机（仅确定算法族与块格式）
4. **Codex CLI 托管模式的内置模型清单**（运行时从外部 catalog 文件加载）
5. **`AppPathManager.audioCacheDirectory`** 的确切路径（macOS 侧）
6. Easydict 的 **Apple Dictionary 私有 API**（`DictionaryServices.framework`）在 Windows 上无对应物 —— 需替换为 MDict/自有词典格式
7. Easydict 的 **Apple 系统翻译/语音** 在 Windows 上无对应物 —— 需替换为其他服务或系统 SAPI
8. **`CharacterSet.urlQueryAllowed` 的确切成员集**：非 ASCII 与控制字符必须转义已可确定（集合只含 ASCII 可见字符，见 `util::url_encode_query_allowed`）；`&`、`/` 在集合内也已由上游代码印证（`String+Convenience.swift:70` 的 `encodeSafely(excluding: "&")` 只有在 `&` 本就在集合内时才不是空操作；DeepL 需另行替换 `/`）。**仍无法确定的是 `%` 与 `#`** —— 本机无 Foundation、无外网。当前实现沿用「除 11 个 ASCII 字符与空格外均不转义」的模型，即 `%`、`#` 原样保留。
9. **LLM 服务验证时的流式回退**：已由源码确认行为 —— `BaseOpenAIService.swift:85-113` 在流式校验遇到 Content-Type 不匹配时，会临时禁用流式重试一次；非流式成功则持久化 `enableStreaming = false`，并把 `validationMessage` 置为 `service.configuration.validation_success.streaming_disabled`（仅在服务本身暴露流式开关时走这条路）。**移植尚未实现**（`commands.rs:376-410` 只做单趟校验），因此 `ServiceValidation.validationNote` 目前恒为 `None`，且**一个只回答 JSON 的端点会校验失败**——即使用户把「流式输出」关掉后它能正常工作。报错文案已经提示用户关闭流式。
10. **Claude / DeepSeek / Doubao 的非流式响应形状**：上游这三个服务都把 `stream` **硬编码为 `true`**（`ClaudeService.swift:203`、`DeepSeekService.swift:142`、`DoubaoService.swift:165`），所以**上游没有可对照的非流式实现**——`nonStreamingTranslate` 只存在于 OpenAI 兼容基类，而这三家各有自己的请求构造。本移植的非流式分支是按各 provider 的公开 API 形状推定的：Claude 读 `content` 文本块（拼接全部 `text`，忽略 `thinking`/`tool_use`）、DeepSeek 走标准 chat-completions（与 OpenAI 共用解码器）、Doubao 读 Ark Responses 的 `output[].content[].text`。三者均有单元/集成测试覆盖，但**没有上游源码可佐证**，与 `BaseOpenAIService` 那条不同。Doubao 的端点是硬编码常量，其非流式路径只能靠解析器单测覆盖。

上述 6/7 属于平台能力差异，Windows 版将以等价替代方案实现并在文档中标注。
