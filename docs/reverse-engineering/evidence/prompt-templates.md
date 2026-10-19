# 提示词模板摘录

对应卷：`03-ai-services.md` §2。
来源：上游 `Swift/Service/OpenAI/StreamService+Prompt.swift`（文件头 `swiftlint:disable all`）、`Swift/Service/AITool/PolishingService.swift`、`Swift/Service/AITool/SummaryService.swift`；移植的逐字副本在 `src-tauri/src/service/llm/prompts.rs`（头注释要求与 Swift 字面量 diff 为空，含上游拼写错误 `Eglish`）。以下引文与原文字节一致。

## 1. 系统提示词（全文）

**`translationSystemPrompt`**（翻译；句法分析也复用此条）

> You are a translation expert proficient in various languages, focusing solely on translating text without interpretation. You accurately understand the meanings of proper nouns, idioms, metaphors, allusions, and other obscure words in sentences, translating them appropriately based on the context and language environment. The translation should be natural and fluent. Only return the translated text, without including redundant quotes or additional notes.

**`dictSystemPrompt`**（词典）

> You are a word search assistant skilled in multiple languages and knowledgeable in etymology. You can help search for words, phrases, slang, abbreviations, and other information. Prioritize queries from authoritative dictionary databases, such as the Oxford Dictionary, Cambridge Dictionary, and Wikipedia. If a word or abbreviation has multiple meanings, look up the most commonly used ones.

**`polishingSystemPrompt`**（润色）

> You are a text polishing expert skilled in refining and enhancing written content. Your task is to improve the clarity, coherence, grammar, and overall quality of the text while maintaining the original meaning and intent. Focus on correcting grammatical errors, improving sentence structure, and enhancing readability. Ensure the polished text is natural and fluent. Only return the polished text, without including redundant quotes or additional notes.

**`summarySystemPrompt`**（总结）

> You are a text summarization expert proficient in condensing lengthy documents, articles, and other text formats into concise and coherent summaries. Your summaries should capture the main points, key details, and overall essence of the original text while maintaining clarity and accuracy. Avoid adding personal opinions or interpretations. Only return the summary, without including redundant quotes or additional notes.

`systemMessage(queryType:)` 的分派：`dictionary` → `dictSystemPrompt`，其余（翻译 / 句法）→ `translationSystemPrompt`。

## 2. 用户提示词模板（全文）

| 场景 | 模板 |
|---|---|
| 翻译 | `Translate the following {源} text into {目标} text: """{text}"""` |
| 润色 | `Polish the following {源} text to improve its clarity, coherence, grammar, and overall quality while maintaining the original meaning and intent: """{text}"""` |
| 总结 | `Using {answerLanguage.rawValue} to summarize the following {源} text: """{text}""".` |

句法分析模板（`sentenceMessages`，按顺序拼接，`{重点词汇}`/`{语法分析}`/`{直译}`/`{意译}` 为中文答案语言下的标签，否则 `Key Words`/`Grammar Parsing`/`Literal Translation`/`Free Translation`）：

1. `Here is a {源.rawValue} sentence: """{sentence}""".` + `First, translate the sentence into {目标.rawValue} literally, keeping the original format and including all information. Use the format: "{直译}:\n{literal_translation}".`
2. `1. List up to 5 key words, phrases, or collocations from the sentence. For each, include all parts of speech and meanings, and specify its meaning in this context if it differs from the common meaning. Use the format: "{重点词汇}:\n{key_words_pos}".`
3. `2. Analyze the grammatical structure of the sentence. Use the format: "{语法分析}:\n{grammatical_analysis}".`
4. `3. Identify any issues in the literal translation, such as non-standard {目标.rawValue} expressions, awkwardness, or ambiguity. Provide a free translation that maintains the original meaning but is clearer and more natural in {目标.rawValue}. If the sentence includes idioms, metaphors, historical references, or famous works, provide a detailed introduction after the translation. Use the format: "{意译}:\n{free_translation}".`
5. `Answer in {answerLanguage.rawValue}.` + `Total word count should not exceed 1000. Do not include additional information or notes.`

词典模板（`dictMessages`，条件拼装；标签在中文答案语言下换成 `发音/时态/翻译/解释/词源学/记忆方法/同根词/近义词/反义词/常用短语/例句`，英文侧原文含小写 `common Phrases`）：

1. `Using {answerLanguage.rawValue}: \n` + `Here is a {源.rawValue} word: """{word}""", `
2. `Look up its pronunciation, use the format: "{发音}: /{pronunciation}/" `
3. **是英文单词时**：`Look up all parts of speech and meanings. Use the format: "{pos}. {meaning}", with one part of speech and meaning per line.` + 时态（名词只给复数；动词给三单/现在分词/过去式/过去分词）。**否则**：`{translationPrompt(word, 源, 目标)}, use the format: "{翻译}: {translation}" `
4. `Look up its brief explanation in {answerLanguage.rawValue} in a clear and understandable way. Use the format: "{解释}: {brief_explanation}"`
5. `Look up its detailed etymology, including the original origin, changes in meaning, and current common meaning. Use the format: "{词源学}: {detailed_etymology}".`
6. **是英文单词时**：记忆方法 + 同根词（≤4，不含短语，没有则不显示）
7. **是单词或英文短语时**：近义词 ≤3 / 反义词 ≤3 / 常用短语 ≤3（均"没有则不显示"）
8. `Look up main {源.rawValue} example sentences and their translations, no more than 2. Mark the specific meaning in the translated sentence with *. Use the format: "{例句}:\n{example_sentences}".`
9. `Ensure the explanation is around 50 words and the etymology is between 100 and 400 words. Total word count should not exceed 1000. Word count does not need to be displayed.` + `Do not display additional information or notes.`

类型判定（`src-tauri/src/util/analysis.rs`，与上游 `String+Extension` 一致）：
- `is_english_word`：去引号后长度 ≤ 20 且 `^[a-zA-Z]+$`
- `is_english_phrase`：去空格后长度 ≤ 40 且词数恰为 2
- `is_chinese_word`：去引号后长度 ≤ 4 且 `^[一-龥]+$`

## 3. 少样本清单（条数与首行；全文见 `prompts.rs`）

| 组 | 条数 | 语言条件 | 首条 user 首行 |
|---|---|---|---|
| `chineseFewShot`（翻译） | 13 | 恒定 | `Translate the following English text into Simplified-Chinese text: """The stock market has now reached a plateau."""` |
| `fromClassicalChineseFewShot` | 5 | 源 = 文言文 | `Translate the following 简体中文文言文 text into 简体中文白话文 text:\n"""曾经沧海难为水，除却巫山不是云。"""` |
| `toClassicalChineseFewShot` | 2 | 目标 = 文言文 | `Translate the following 简体中文白话文 text into 简体中文文言文 text:\n"""不要忽视梦想。不要工作过久。说出想法。交朋友。要开心。"""` |
| `sentenceChineseFewShot` | 4 | 答案语言为中文 | `Here is an English sentence: """But whether the incoming chancellor will offer dynamic leadership, rather than more of Germany's recent drift, is hard to say."""` |
| `sentenceEnglishFewShot` | 1 | 答案语言非中文 | 同一句英文，答案使用英文标签（`Literal Translation:` 等） |
| `dictChineseFewShot` | 3 | 答案语言为中文 | `Using Simplified-Chinese:\nHere is a English word: """album"",`（完整词条）/ `raven`（完整词条）/ `js`（短词条） |
| `dictEnglishFewShot` | 2 | 答案语言非中文 | `Using English:\nHere is a English word: "raven",` / `acg` |
| `polishingFewShot` | 3 | 恒定 | 英文英式润色（时态错误 / 主谓一致 / 连接词优化） |
| `summaryFewShot` | 2 | 恒定 | 英文短故事；中文联合国新闻长文 |

消息序列（翻译）：`[system] * 1 + chineseFewShot * 13 + (源为文言文时) 5 + (目标为文言文时) 2 + [user: 翻译提示词]`。
句法分析：`[system: translationSystemPrompt] + 对应语言少样本 + [user]`；词典同理。

## 4. 自定义提示词与变量

`enableCustomPrompt = true` 时：系统消息 = `replaceCustomPromptWithVariable(systemPrompt)`（空则不发），用户消息 = `replaceCustomPromptWithVariable(userPrompt)`（空则回落 `queryModel.queryText`）。

| 变量 | 替换为 |
|---|---|
| `$queryFromLanguage` | 源语言 `queryLanguageName` |
| `$queryTargetLanguage` | 目标语言 `queryLanguageName` |
| `$queryText` | 查询文本 |
| `$firstLanguage` | 第一语言 `rawValue` |

**不生效之处**：`Polishing` / `Summary` **整体覆写** `chatMessageDicts`，因此 `enableCustomPrompt` 对它们无效（与上游一致，移植在 `ai_tool.rs` 头注释记录）。

## 5. 相关常量

| 项 | 值 |
|---|---|
| 验证探针文本 | `曾经沧海难为水`（zh→en，长度 > 5 以避开词典分支） |
| 请求温度默认 | 0.3 |
| Claude `max_tokens` | 4096 |
| 流式节流 | 0.3 s |
| 隐藏 think 标签 | 正则 `^<tag>(?:(?!</tag>).)*?</?tag>`，仅在字符串**开头**匹配 |
| 最终文本清理 | trim → 去尾部 `</s>`（仅当查询文本本身不以 `</s>` 结尾）→ 成对引号剥离 → trim |
