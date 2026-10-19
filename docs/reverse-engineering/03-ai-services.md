# Easydict 逆向分析报告 — AI / LLM 服务

> 目标：固定 `01-overview.md` §3 服务清单第 15–30 项中 **13 个 LLM 服务** 的类层级、共用传输、提示词、SSE 解析与逐服务端点；另说明 3 个 Agent CLI 服务为何未进入 Windows 版。
>
> 上游源码：`Easydict/Swift/Service/{OpenAI,OpenAI/*,CustomOpenAI,Gemini,Claude,DeepSeek,Groq,Zhipu,MiniMax,"GitHub Models",Ollama,BuiltInAI,AITool}/`
> 移植实现：`app/src-tauri/src/service/llm/{mod,openai,claude,deepseek,prompts,ai_tool}.rs`
>
> 分析日期：2026-10-16

---

## 0. 类层级与公共契约

### 0.1 继承链

```
QueryService（@objc(EZQueryService)）
└── StreamService（@objc(EZStreamService)）                 Swift/Service/OpenAI/StreamService.swift
    ├── BaseOpenAIService（@objc EZBaseOpenAIService）      Swift/Service/OpenAI/BaseOpenAIService.swift
    │   ├── OpenAIService                                  Swift/Service/OpenAI/OpenAIService.swift
    │   │   ├── GeminiService        Gemini/GeminiService.swift
    │   │   ├── GroqService          Groq/GroqService.swift
    │   │   ├── ZhipuService         Zhipu/ZhipuService.swift
    │   │   ├── MiniMaxService       MiniMax/MiniMaxService.swift
    │   │   └── GitHubService        GitHub Models/GitHubService.swift
    │   ├── CustomOpenAIService      CustomOpenAI/CustomOpenAIService.swift
    │   ├── OllamaService            Ollama/OllamaService.swift
    │   ├── DeepSeekService          DeepSeek/DeepSeekService.swift   ← 覆写全部传输层
    │   └── BuiltInAIService         BuiltInAI/BuiltInAIService.swift
    │       └── AIToolService        AITool/AIToolService.swift
    │           ├── PolishingService AITool/PolishingService.swift
    │           └── SummaryService   AITool/SummaryService.swift
    └── ClaudeService                Claude/ClaudeService.swift       ← 直接继承 StreamService，自带 SSE 解析
```

Windows 移植用 Rust trait 表达同一契约（`app/src-tauri/src/service/mod.rs` 的 `QueryService` + `service/llm/mod.rs` 的 `LlmTransport`），服务结构由 `openai_compatible_service!` 宏批量生成（`service/llm/openai.rs`）。

### 0.2 `StreamService` 提供的公共能力

| 成员 | 说明 |
|---|---|
| `isStream()` | 恒为 `true` |
| `apiKeyRequirement()` | `.userProvided`（除 `Ollama` 为 `.none`、`BuiltInAI`/`Polishing`/`Summary` 为 `.builtIn`） |
| `hasPrivateAPIKey()` | `!apiKey.isEmpty` |
| `contentStreamTranslate(_:from:to:)` | 子类必须实现，返回 `AsyncThrowingStream<String, Error>` |
| `translate(...)` | 抽干流并返回最后一个结果（带 generation 校验） |
| `translateStream(...)` | 包一层节流后的 `AsyncThrowingStream<QueryResult, Error>` |
| `queryType(text:from:to:)` | 见 §0.3 |
| `chatMessageDicts(_:)` | 自定义提示词优先，否则 `builtInChatMessageDicts` |
| `replaceCustomPromptWithVariable(_:)` | 变量替换，见 §0.4 |
| `supportedQueryType()` | 三个开关（translation / dictionary / sentence）按位或 |

配置槽位（`ServiceConfigurationKey`，键名规则 `"EZ" + ServiceType + Key + ["_" + uuid + "_"] + "Key"`）：

| 槽位 | 默认值 |
|---|---|
| `model` / `SupportedModels`(=`AvailableModels`) / `ValidModels` | 见各服务 |
| `apiKey`(=`API`) / `endPoint`(=`EndPoint`) | 见各服务 |
| `translation` / `dictionary` / `sentence` | `"1"` / `"0"` / `"0"` |
| `enableCustomPrompt` / `systemPrompt` / `userPrompt` | `false` / `""` / `""` |
| `thinkTag`（→ `hideThinkTagContent`） | `true` |
| `temperature` | `0.3` |
| `enableStreaming` | `true` |
| `reasoningEffort` | `.off` |
| `serviceUsageStatus` | `.default` |

模型一致性：`model` 读不到有效值时**静默改写**为 `validModels.first` 并落盘；编辑 `supportedModels` 会把 `model` 重置为 `validModels[0]`。移植在 `service/llm/mod.rs` 的 `resolve_model` 中复刻。

### 0.3 查询类型路由

```
1. 若服务被显式指定了 queryType → 直接使用
2. 否则若 dictionary 开启 且 shouldQueryDictionary(withLanguage:maxWordCount: 2) → .dictionary
3. 否则若 sentence 开启 且 源语言为英语 且 shouldQuerySentence → .sentence
4. 否则 → .translation
```

上游用 `maxWordCount: 2`（比非 AI 服务的 1 更宽松），且句法分析**只对英语源文本**开启。移植实现为 `service/llm/mod.rs::routed_query_type`，此处有一个刻意的差异：移植让服务配置里的 `dictionary`/`sentence` 显式布尔值优先（因为 `ServiceConfig` 把上游的字符串开关存成 `bool`），条件判断顺序与上游一致。

### 0.4 自定义提示词与变量

`enableCustomPrompt = true` 时：

- 系统消息 = `replaceCustomPromptWithVariable(systemPrompt)`（为空则不发送 system 消息）
- 用户消息 = `replaceCustomPromptWithVariable(userPrompt)`；若 userPrompt 为空则回落为 `queryModel.queryText`

可替换变量（**注意双花括号语法，与下文占位符无关**）：

| 变量 | 替换为 |
|---|---|
| `$queryFromLanguage` | 源语言 `queryLanguageName` |
| `$queryTargetLanguage` | 目标语言 `queryLanguageName` |
| `$queryText` | 查询文本 |
| `$firstLanguage` | `MyConfiguration.shared.firstLanguage.rawValue` |

`queryLanguageName` 映射：`.classicalChinese → "简体中文文言文"`、`.simplifiedChinese → "简体中文白话文"`、`.traditionalChinese → "繁体中文白话文"`，其余取 `Language` 原始字符串（如 `English`、`Japanese`）。

---

## 1. OpenAI 兼容传输（共用）

### 1.1 请求

```
POST <endpoint>
Content-Type: application/json
Authorization: Bearer <apiKey>        # 仅当 key 非空
api-key: <apiKey>                     # 仅当 key 非空；为 Azure 兼容而对所有服务都发
```

- 超时 `AppConstants.llmRequestTimeoutInterval = 60` 秒（流式与非流式一致；`URLRequest.timeoutInterval` 是**空闲**超时，收到字节即重置）。
- 端点校验：`URL(string:)` 必须 scheme 与 host 齐全，否则 `QueryError(.parameter, "\`<Type>\` endpoint is invalid")`；需要密钥而 key 为空则 `QueryError(.missingSecretKey, "API key is empty")`。
- 移植同：`LlmTransport::new(endpoint, api_key, model, temperature)` 同时发 `Authorization: Bearer` 与 `api-key`（`service/llm/mod.rs`，常量 `LLM_TIMEOUT_SECS = 60`）。

### 1.2 请求体

流式把 `stream` 置 `true`，非流式置 `false`，其余相同：

```json
{
  "messages": [ {"role":"system","content":"…"}, {"role":"user","content":"…"} ],
  "model": "<model>",
  "temperature": 0.3,
  "stream": true
}
```

`ChatQuery` 里其余字段（`max_tokens`、`top_p`、`stop`、`response_format`、`tools` …）在 Easydict 中**恒为 nil，因此不会出现在 JSON 里**。移植的 `ChatCompletionRequest` 字段顺序为 `messages, model, temperature, stream`（与上游一致，便于逐字节对比）。

### 1.3 SSE 分帧规则（**后缀**判定）

这是最容易写错的一处：上游**不扫描**第一个空行，而是**只在缓冲区末尾**判定分隔符，命中才把「分隔符之前的全部内容」当作一个事件。

```swift
if buffer.suffix(4) == [13,10,13,10]                        → 分隔符长 4（CRLFCRLF）
else if buffer.suffix(2) == [10,10] || buffer.suffix(2) == [13,13] → 分隔符长 2（LFLF 或 CRCR）
else → 继续读
```

后果：一次读取里到达两个事件时，它们会被合并成一个「事件」，其 `data:` 行用 `\n` 连接——通常 JSON 解码失败。移植 `service/llm/openai.rs` 头注释**明确保留**该后缀语义（并注明「扫描式读取器永远不会在 CRLF 上触发」），`SseReader`（`service/stream.rs`）对 Doubao 等自定义解析仍用扫描式，两者刻意共存。

### 1.4 事件 → 载荷与严格解码

- 逐行判断 `line.hasPrefix("data:")`，取其后并 `trimmingCharacters(in: .whitespaces)`。
- 多行 `data:` 用 `"\n"` 连接后再整体 `trim`。
- 载荷 == `"[DONE]"` → **立即结束读取并正常返回**（不抛错，也不处理缓冲区剩余内容）。
- 否则 `JSONDecoder().decode(ChatStreamResult.self, from: data)`；**解码是严格的**：`id` / `object` / `created` / `model` / `choices` 缺一不可（SDK 未配置 `ParsingOptions`）。失败时若能被 `APIErrorResponse` 解出，就抛出该 API 错误，否则抛出原始 `DecodingError`。
- 文本增量取 `choices[0].delta.content`；**推理增量被忽略**（OpenAI 兼容路径不读 `delta.reasoning`）。

### 1.5 HTTP 校验与流中错误的分类

```swift
非 2xx → 先读最多 64 KiB 错误体（maximumErrorBodySize），尝试解 APIErrorResponse，
         否则抛 QueryError(.api, "HTTP <code>", errorDataMessage: <body 文本>)
2xx    → mimeType?.lowercased() 必须等于 "text/event-stream"，
         否则抛 incorrectContentType(mime, url)：
         "Incorrect Content-Type: <mime>, acceptable type is text/event-stream.
          This may be caused by a wrong endpoint: <url>"
```

**错误吞掉规则**：若流已产出内容且累计 `trim` 后长度 ≥ 8，**并且**错误上下文包含 `incorrectcontenttype(` / `incorrect content-type:` / `unacceptable content-type:` 之一，**并且**上下文里出现 `text/plain` 或 `application/json`，则该错误只记日志、当作正常结束（<8 字符永不吞）。其余流中错误分类为 `QueryError(.contentTypeMismatch, …)`：

| 上下文包含 | message | errorDataMessage |
|---|---|---|
| `text/html` | `Endpoint returned an HTML page` | `Please check that the endpoint URL is correct and includes the full API path.` |
| `application/json` | `Endpoint returned JSON instead of a stream` | `Try disabling "Enable Streaming" in the service settings.` |
| 其它 | `Unexpected response format` | `The endpoint may not be fully OpenAI-compatible. Check the endpoint URL and API documentation.` |

移植把这些逐条搬进 `service/llm/mod.rs`：`should_ignore_stream_error`、`classified_stream_error`、`content_type_mismatch_error`、`api_error_message`、`error_context`、`handle_stream_error`，常量 `MAX_ERROR_BODY_SIZE = 64 * 1024`、`DONE_FLAG = "[DONE]"`、`MODEL_LIST_TIMEOUT_SECS = 15`。

### 1.6 节流、think 过滤与最终文本

- **节流**：`throttleUpdateResultText(interval: 0.3)`，对**整个累积文本**做 0.3 秒节流。移植常量 `STREAM_THROTTLE_INTERVAL = 300 ms`；**差异**：移植的 `ServiceUpdate::Chunk` 是**追加**语义，因此节流器只发送尚未交付的**后缀**，而不是上游那样重发整段快照（`ChunkThrottle`，头注释在案）。
- **think 标签过滤**：`hideThinkTagContent`（默认 on）时用正则 `^<tag>(?:(?!</tag>).)*?</?tag>` 把开头的 `<think>…</think>` 整段替换为空。**只在字符串开头**匹配：`"<think>hello"` → `""`；`"<think>hello</think>world"` → `"world"`；`"hello<think>world</think>"` → **不变**。移植为 `service/stream.rs::filter_think_tag_content`。
- **最终文本** `getFinalResultText`：`trim()` → 若查询文本本身不以 `</s>` 结尾而结果以 `</s>` 结尾则去掉该后缀再 `trim()`（Groq mixtral 的历史修复）→ `tryToRemoveQuotes()`（成对引号 `" ' \` “” ‘’ «» ‹› 「」 『』 《》 〈〉 ﹁﹂ ﹃﹄`，仅当首尾引号等长且成对时剥离）→ 再 `trim()`。
- 结果为空且无错误 → `QueryError(.noResult)`；词典类型的结果置 `showBigWord = true` 与 `translateResultsTopInset = 6`（出错时归 0）。

### 1.7 远端模型列表

```
GET <modelsURL>
Authorization: Bearer <key>；api-key: <key>；Accept: application/json
超时 15 秒
```

`modelsURL` 推导：优先用 `remoteModelsEndpoint`；否则取 `endpoint` 的路径段，去掉结尾的 `chat/completions`（2 段）或 `completions` 或 `models`，否则默认 `["v1"]`，再拼 `models`。响应形如 `{"data":[{"id":"…"}]}`，逐条 `trim`、去空、按「去空格小写」去重。错误消息提取顺序：`error`(字符串) → `error.message` → `message` → 原始体截断到 **300 字符**。

---

## 2. 提示词模板

上游文件：`Swift/Service/OpenAI/StreamService+Prompt.swift`（`swiftlint:disable all`），移植逐字复制到 `service/llm/prompts.rs`（头注释要求与 Swift 字面量 diff 为空，包含上游拼写错误 `Eglish`）。

### 2.1 系统提示词（逐字引用）

**翻译（`translationSystemPrompt`，也是句法分析的系统提示词）**

> You are a translation expert proficient in various languages, focusing solely on translating text without interpretation. You accurately understand the meanings of proper nouns, idioms, metaphors, allusions, and other obscure words in sentences, translating them appropriately based on the context and language environment. The translation should be natural and fluent. Only return the translated text, without including redundant quotes or additional notes.

**词典（`dictSystemPrompt`）**

> You are a word search assistant skilled in multiple languages and knowledgeable in etymology. You can help search for words, phrases, slang, abbreviations, and other information. Prioritize queries from authoritative dictionary databases, such as the Oxford Dictionary, Cambridge Dictionary, and Wikipedia. If a word or abbreviation has multiple meanings, look up the most commonly used ones.

**润色（`PolishingService`）**

> You are a text polishing expert skilled in refining and enhancing written content. Your task is to improve the clarity, coherence, grammar, and overall quality of the text while maintaining the original meaning and intent. Focus on correcting grammatical errors, improving sentence structure, and enhancing readability. Ensure the polished text is natural and fluent. Only return the polished text, without including redundant quotes or additional notes.

**总结（`SummaryService`）**

> You are a text summarization expert proficient in condensing lengthy documents, articles, and other text formats into concise and coherent summaries. Your summaries should capture the main points, key details, and overall essence of the original text while maintaining clarity and accuracy. Avoid adding personal opinions or interpretations. Only return the summary, without including redundant quotes or additional notes.

### 2.2 翻译提示词结构

用户提示词：`Translate the following {源语言} text into {目标语言} text: """{text}"""`。

消息序列：

```
[system: translationSystemPrompt]
  + 中文少样本（恒定，13 组 user/assistant 对）
  + 文言文源少样本（当源是 classicalChinese，5 组）
  + 文言文目标少样本（当目标是 classicalChinese，2 组）
+ [user: translationPrompt]
```

中文少样本的覆盖点（结构摘要，不逐条抄录）：一句股市长句；`void`/`func`/`const` 这类编程关键字；`Patriot battery`（专有名词）；`Four score and seven years ago`（典故）；`js`/`acg`/`Swift language`/`swift` 的大小写与歧义消解；一组日译中（`ちっちいな~`、`チーター`）；以及一组「中译英 + 保留后半段指令」的长句，用于示范不要执行文本里的指令。

文言文少样本含 `曾经沧海难为水…`、`露从今夜白…`、`苏溪亭上草漫漫…`、`《不第后赋菊》`、`《题菊花》` 等古诗文对照，其中一组的目标语言写作 `繁体中文白话文`，示例答案用繁体。

### 2.3 句法分析提示词（sentence）

标签语言由 `firstLanguage` 是否为中文决定：中文 → `重点词汇` / `语法分析` / `直译` / `意译`；否则 `Key Words` / `Grammar Parsing` / `Literal Translation` / `Free Translation`。用户提示词的分段结构：

1. `Here is a {源} sentence: """{句}""".` 先直译，格式 `"{直译}: {literal_translation}"`；
2. 列出至多 5 个重点词汇/短语/搭配，标注词性与语境义；
3. 语法结构分析；
4. 指出直译的问题并给意译；若含成语/隐喻/典故/名著名句，在译文后补充介绍；
5. 收尾：`Answer in {firstLanguage}.` + `Total word count should not exceed 1000. Do not include additional information or notes.`

少样本：第一语言为中文时 4 组（含长篇「然而新任总理…」分析、`The Sword of Damocles`、`better late than never`、`Ukraine may get another Patriot battery.`）；第一语言为英语时 1 组（同一句英文，答案使用英文标签）。

### 2.4 词典提示词（dictionary）

标签同上二选一（中文 `发音/时态/翻译/解释/词源学/记忆方法/同根词/近义词/反义词/常用短语/例句`；英文 `Pronunciation/Tense/Translation/Explanation/Etymology/How to remember/Cognate/Synonym/Antonym/common Phrases/Example sentence`，注意 `common Phrases` 里的**小写 c** 是上游原样）。拼装顺序与条件：

```
Using {firstLanguage}: Here is a {源} word: """{word}""",
  Look up its pronunciation … "{发音}: /{音标}/"
若 isEnglishWord（^[a-zA-Z]+$ 且长度 ≤ 20）：
  列全部词性与释义（每行一条）+ 全部时态（名词只给复数；动词给三单/现在分词/过去式/过去分词）
否则：
  {translationPrompt(word, 源, 目标)}，格式 "{翻译}: {译文}"
恒定：简要解释（约 50 词）+ 词源（100–400 词）
若 isEnglishWord：记忆方法 + 同根词（≤4，不含短语）
若 isWord 或 isEnglishPhrase：近义词（≤3）/ 反义词（≤3）/ 常用短语（≤3）
恒定：例句与译文（≤2，译文里用 * 标出对应义项）+ 字数上限 1000 + 不要额外内容
```

类型判定：`isEnglishWord` = 去引号后长度 ≤ 20 且匹配 `^[a-zA-Z]+$`；`isEnglishPhrase` = 去空格后长度 ≤ 40 且词数恰为 2；`isChineseWord` = 去引号后长度 ≤ 4 且匹配 `^[一-龥]+$`。少样本：中文答案语言 3 组（`album`、`raven` 完整词条 + `js` 短词条），英文答案语言 2 组（`raven`、`acg`）。

### 2.5 润色与总结提示词

- 润色用户提示词：`Polish the following {源语言} text to improve its clarity, coherence, grammar, and overall quality while maintaining the original meaning and intent: """{text}"""`，附 3 组英文英式润色少样本（时态错误、主谓一致、连接词优化）。
- 总结用户提示词：`Using {firstLanguage 原始值} to summarize the following {源语言} text: """{text}""".`，附 2 组少样本（英文短故事、中文联合国新闻长文）。
- **两者都整体覆写 `chatMessageDicts`**，因此 `enableCustomPrompt` 对它们**无效**（移植 `service/llm/ai_tool.rs` 头注释记录了这一偏离，并说明设置里的 `customPrompt` 被忽略的原因）。

---

## 3. 各服务规格

下表是「端点 / 默认模型 / 鉴权 / 关键怪癖」的汇总，逐服务说明见 §3.1–§3.13。

| 服务 | ServiceType | 端点 | 默认模型 | 鉴权头 | 怪癖 |
|---|---|---|---|---|---|
| OpenAI | `OpenAI` | `https://api.openai.com/v1/chat/completions` | `gpt-5-mini` | Bearer + api-key | 不监听 endpoint 变化 |
| CustomOpenAI | `CustomOpenAI[#uuid]` | 用户填写（默认空） | 用户填写 | Bearer + api-key | 唯一暴露「启用流式」开关、唯一在校验失败时自动关流 |
| Gemini | `Gemini` | `https://generativelanguage.googleapis.com/v1beta/openai/chat/completions` | `gemini-flash-lite-latest` | Bearer + api-key | 走 OpenAI 兼容层；模型 id 去掉 `models/` 前缀 |
| Claude | `Claude` | `https://api.anthropic.com/v1/messages` | `claude-sonnet-4-6` | `x-api-key` + `anthropic-version: 2023-06-01` | 自有 SSE 解析；system 提到顶层；`max_tokens: 4096`；温度截到 0–1 |
| DeepSeek | `DeepSeek` | `https://api.deepseek.com/v1/chat/completions` | `deepseek-v4-flash` | 仅 Bearer | 自研解析；`thinking.type` 恒发；`reasoning_effort` 仅 high/max；无内容类型检查 |
| Groq | `Groq` | `https://api.groq.com/openai/v1/chat/completions` | `openai/gpt-oss-120b` | Bearer + api-key | 结果末尾 `</s>` 会被剥离（历史修复） |
| Zhipu | `Zhipu` | `https://open.bigmodel.cn/api/paas/v4/chat/completions` | `glm-4-flash-250414` | Bearer + api-key | 无 |
| MiniMax | `MiniMax` | `https://api.minimaxi.com/v1/chat/completions` | `MiniMax-M2.5` | Bearer + api-key | 无 |
| GitHub Models | `GitHub` | `https://models.github.ai/inference/chat/completions` | `gpt-4.1` | Bearer + api-key | 模型目录 `https://models.github.ai/catalog/models`，按 text 模态过滤 |
| Ollama | `Ollama` | `http://localhost:11434/v1/chat/completions` | 首个本地模型 | 无 | `apiKeyRequirement = .none`；模型列表来自 `http://localhost:11434/api/tags` |
| Built-In AI | `BuiltInAI` | 上游取自加密 plist | `glm-4-flash-250414` | Bearer + api-key | 上游内置端点与 key；移植不随包分发 |
| Polishing | `Polishing` | 同上 | 同上 | 同上 | 默认关闭（`alwaysOff`）；整体覆写消息 |
| Summary | `Summary` | 同上 | 同上 | 同上 | 同上 |

### 3.1 OpenAI

`serviceType` `OpenAI`；`name()` = `openai_translate`（中「OpenAI 翻译」）；`link()` = `https://chatgpt.com`；`observeKeys = [apiKey, supportedModels]`（**不监听 endpoint**）；配置项 `gpt-5`、`gpt-5-mini`、`gpt-5-nano`、`gpt-4.1`，默认 `gpt-5-mini`；远端模型列表继承 → `https://api.openai.com/v1/models`。

### 3.2 CustomOpenAI

`serviceType` `CustomOpenAI`；标识符 `"CustomOpenAI#<uuid>"`（uuid 空时为 `"CustomOpenAI"`）；`defaultEndpoint` 为**空串**（因此设置界面的占位文案是「完整请求 URL，例如 https://api.openai.com/v1/chat/completions」），`defaultModels = [""]`；`name()` 取用户填写的名字（设置里限制 20 字符），否则本地化 `custom_openai`；**唯一** `supportsStreamingToggle = true` 的服务。`validate()` 在「流式失败且错误类型是 `contentTypeMismatch`」时自动重试一次非流式，成功后把 `enableStreaming` 落盘为 `false` 并给出提示 `Streaming is not supported by this endpoint and has been auto-disabled.`（移植只在自定义 OpenAI 上保留该行为）。

### 3.3 Gemini

继承 `OpenAIService`，用 Gemini 的 **OpenAI 兼容端点**（不调用 `generateContent`，也不用 `?key=` 查询参数）。模型：`gemini-flash-lite-latest`（默认）、`gemini-flash-latest`、`gemini-pro-latest`。`unsupportedLanguages` = 波斯语、菲律宾语、高棉语、老挝语、马来语、蒙古语、缅甸语、泰卢固语、泰米尔语、乌尔都语（这 10 种从语言选择器里移除）。远端模型 id 会剥掉前导 `models/`。

### 3.4 Claude（完全自定义）

- 端点 `https://api.anthropic.com/v1/messages`；远端模型 `https://api.anthropic.com/v1/models?limit=1000[&after_id=…]`（分页直到 `has_more` 为假）。
- 请求头：`Content-Type: application/json`、`x-api-key: <key>`、`anthropic-version: 2023-06-01`。**没有** Bearer、没有 `api-key`。**不设**超时（用 URLSession 默认值）。
- 请求体：`{model, max_tokens: 4096, temperature: clamp(0…1), stream: true, messages:[{role:user|assistant, content}], system?}`；所有 `.system` 消息被抽到顶层 `system` 字段（用 `\n\n` 连接）。
- SSE 解析（`ClaudeSSEParser`）：攒够 1024 字节或遇到 `0x0A` 才解码；CRLF→LF 时**保留一个末尾 `\r`** 与下一块拼对；按 `"\n\n"` 切事件；`event: content_block_delta` 取 `delta.text`；`event: error` 直接抛 `QueryError(.api, errorDataMessage: <error.message>)`。
- 非 2xx 时最多读 4096 字节，尝试解 `{"error":{"message":…}}`。

### 3.5 DeepSeek（自定义传输）

- 端点 `https://api.deepseek.com/v1/chat/completions`，模型列表 `https://api.deepseek.com/models`（`remoteModelFetchRequiresEndpoint = false`）；模型 `deepseek-v4-flash`（默认）、`deepseek-v4-pro`。
- `supportsReasoningEffort = true`：请求体在通用字段之外**恒带** `thinking: {"type": "enabled"|"disabled"}`，并在 high/max 时带 `reasoning_effort`（`.off` 时整键省略）。
- 自己的 SSE 解析：按 `0x0A` 累积、`\r\n`→`\n`、按 `"\n\n"` 切、丢掉最后一段；同一事件的多条 `data:` 行**不用分隔符**直接拼接；`[DONE]` 返回 nil；解码失败只记日志、不抛错；**完全没有内容类型检查**。
- 移植 `service/llm/deepseek.rs` 保留上述语义，并同样只发 `Authorization: Bearer`。

### 3.6 Groq

端点 `https://api.groq.com/openai/v1/chat/completions`；模型 `openai/gpt-oss-120b`（默认）、`openai/gpt-oss-20b`、`llama-3.1-8b-instant`、`moonshotai/kimi-k2-instruct-0905`、`meta-llama/llama-4-maverick-17b-128e-instruct`、`qwen/qwen3-32b`。无额外请求头。

### 3.7 Zhipu 智谱

端点 `https://open.bigmodel.cn/api/paas/v4/chat/completions`；默认 `glm-4-flash-250414`；共 16 个可选模型（`glm-5`、`glm-4.7`、`glm-4.7-flash`、`glm-4.7-flashx`、`glm-4.6`、`glm-4.5-air`、`glm-4.5-airx`、`glm-4.5-flash`、`glm-4-flash-250414`、`glm-4-flashx-250414`、`glm-4.6v`、`glm-4.6v-flash`、`glm-4.6v-flashx`、`glm-4v-flash`、`glm-4.1v-thinking-flashx`、`glm-4.1v-thinking-flash`）。

### 3.8 MiniMax

端点 `https://api.minimaxi.com/v1/chat/completions`；默认 `MiniMax-M2.5`；模型 `MiniMax-M2.5`、`MiniMax-M2.5-highspeed`、`MiniMax-M2.1`、`MiniMax-M2.1-highspeed`、`MiniMax-M2`。

### 3.9 GitHub Models

端点 `https://models.github.ai/inference/chat/completions`；默认 `gpt-4.1`；模型 `gpt-4.1`、`gpt-4.1-mini`、`gpt-4.1-nano`、`gpt-4o`、`gpt-4o-mini`、`deepseek-v3-0324`。目录端点 `https://models.github.ai/catalog/models`，请求头额外带 `Accept: application/vnd.github+json` 与 `X-GitHub-Api-Version: 2026-03-10`；目录条目按输入/输出模态含 `text` 过滤（数组缺失视为通过）。模型 id 归一化：去掉首个 `/` 之前的部分（`openai/gpt-4.1` → `gpt-4.1`），并按前缀分组。**没有任何 OAuth/设备码流程**——用户直接粘贴 GitHub PAT。空 token 报 `GitHub Models token is empty`。

### 3.10 Ollama

端点 `http://localhost:11434/v1/chat/completions`；`apiKeyRequirement = .none`（不发鉴权头）；模型列表来自本地 `http://localhost:11434/api/tags`（只取 `models[].name`），界面按钮文案为「获取本地模型」。移植 `service/llm/openai.rs` 的 `local_models_url` 复刻该地址。

### 3.11 Built-In AI（上游内置凭据）

`apiKeyRequirement = .builtIn`；`apiKey` 与 `endpoint` 都来自 `Swift/Feature/DefaultAPIKeys/EncryptedSecretKeys.plist`（AES 解密后读取 `builtInAIAPIKey` / `builtInAIEndpoint`），**本报告不复述其值**，见 `01-overview.md` §4。默认模型 `glm-4-flash-250414`，可选 `glm-4.5-flash`、`glm-4-flash-250414`、`llama-3.1-8b-instant`；`supportedModels` 在 init 时固定、界面不可编辑；`canFetchRemoteModels = false`。

**移植差异（重要）**：Windows 版**不随包分发**该凭据。`service/llm/openai.rs` 的 `BuiltInAiService` 是一个占位实现：默认端点写成智谱的公开端点，任何请求先报 `MissingSecretKey`，提示文案为「Windows 版不内置 Built-In AI 的 API key，请在 设置 → 服务 → Built-In AI 中填写自己的 key，并把端点改成你要用的 OpenAI 兼容服务」。这与 `01-overview.md` §4 的合规结论一致。

### 3.12 Polishing 润色

`serviceType` `Polishing`；`apiKeyRequirement = .builtIn`（移植改为要求用户 key）；`serviceUsageStatus` 默认 **`.alwaysOff`**（默认不出现在结果列表）；继承 `BuiltInAIService` 的端点/模型，但**整体覆写**消息构建：`[system: polishingSystemPrompt] + 3 组英文少样本 + [user: Polish the following …]`。`enableCustomPrompt` 对它无效。

### 3.13 Summary 总结

`serviceType` `Summary`；其余同上；消息为 `[system: summarySystemPrompt] + 2 组少样本 + [user: Using {firstLanguage} to summarize …]`。**与润色的差别只有系统提示词、用户提示词模板与少样本**，传输层完全一致。

**移植差异**：`service/llm/ai_tool.rs` 让润色/总结**各自读取自己的服务配置项**（而不是复用 BuiltInAI 的槽位），缺 key 时报 `MissingSecretKey` 并点名设置路径；同时记录「因为 `chatMessageDicts` 被整体覆写，`customPrompt` 设置被忽略」这一与上游一致的行为。

---

## 4. 汇总

### 4.1 `queryType` 与工具类服务

- 13 个服务都支持 translation；dictionary / sentence 由各自开关决定（默认关闭）。
- 只有 `.dictionary` 走 `dictMessages`，`.sentence` 走 `sentenceMessages`，其余一律 `translationMessages`。

### 4.2 与上游的移植差异一览

| # | 差异 | 位置 | 原因 |
|---|---|---|---|
| 1 | Built-In AI 不内置端点与 key | `service/llm/openai.rs`（`MISSING_KEY_MESSAGE`） | 合规：不消耗他人配额 |
| 2 | Polishing / Summary 各自读自己的配置并报缺 key | `service/llm/ai_tool.rs` | 同上 |
| 3 | 节流只发送增量后缀，而非整段快照 | `service/llm/mod.rs`（`ChunkThrottle`） | 移植的 `ServiceUpdate::Chunk` 是追加语义 |
| 4 | 保留 SSE 的**后缀**分隔符语义（不改成扫描式） | `service/llm/openai.rs` 头注释 + `SseReader` | 行为等价优先 |
| 5 | DeepSeek 同样不做内容类型检查 | `service/llm/deepseek.rs` | 与上游一致 |
| 6 | Claude 自带解析器，不共用 `LlmTransport` | `service/llm/claude.rs` | 消息结构不同（顶层 `system`、`max_tokens`） |
| 7 | Agent CLI 服务（`ClaudeCode` / `CodexCLI` / `GitHubCopilot`）未实现 | `service/registry.rs::is_implemented` | 依赖本机 CLI 登录态，Windows 版不提供；详见 `05-architecture.md` 的「已知限制与未实现项」 |
| 8 | 提示词逐字复制（含上游拼写错误 `Eglish`） | `service/llm/prompts.rs` | 行为等价；头注释要求与 Swift 字面量 diff 为空 |

### 4.3 未确定项

- `BuiltInAIService` 的端点与 key 在 `01-overview.md` §4 已记录，本卷不复述。
- `StreamService+Prompt.swift` 的少样本全文未逐条抄录（体量原因），仅描述结构与覆盖点；移植侧 `service/llm/prompts.rs` 是唯一的逐字副本。
- 上游 `validate()` 的探针文本是 `曾经沧海难为水`（zh→en，长度 > 5 以避开词典分支），移植在同名路径保留。
