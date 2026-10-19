# 语言代码表（含与 BCP-47 的差异）

对应卷：`02-services.md`、`03-ai-services.md`。
来源：上游 `supportLanguagesDictionary`（Swift）/ `EZLanguageModel`（ObjC）/ 移植 `src-tauri/src/service/languages.rs` 的对应常量。**粗体 = 与 BCP-47 或直觉写法不一致者。**

## 1. Youdao

`zh-CHS`、`zh-CHT`、`en`、`ja`、`ko`、`fr`、`es`、`pt`、`it`、`de`、`ru`、`ar`、`th`、`nl`、`id`、`vi`

- **`zh-CHS` / `zh-CHT`**：既不是 `zh-CN`/`zh-TW`，也不是 `zh-Hans`/`zh-Hant`。
- 只有 `pt`，无 `pt-PT`/`pt-BR`。
- 无 `auto`：源语言自动识别在本地完成后才发请求。

## 2. DeepL

`auto`、**`zh-hans`**、**`zh-hant`**、`en`、`ja`、`ko`、`fr`、`es`、`ca`、**`pt-PT`**、**`pt-BR`**、`it`、`de`、`ru`、`sv`、`ro`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`sl`、`et`、**`nb`**、`ar`、`vi`、`he`、`th`

- 中文变体是**全小写** `zh-hans`/`zh-hant`；oneshot 通道发送前会做大小写归一（`zh-hans`→`zh-Hans`，`en`→`en-US`），官方通道则原样发送。
- 源语言未知时：oneshot **整键省略** `source_lang`，官方 API 发送字面量 `auto`。
- 移植另存 `autoConvertTraditionalChinese() == true`：双方都是中文时本地转换。

## 3. Google

`auto`、**`zh-CN`**、**`zh-TW`**、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt-PT`、**`pt`（巴葡）**、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、**`tl`（菲律宾语）**、`km`、`lo`、`bn`、`my`、**`no`（挪威语）**、`sr`、`hr`、`mn`、**`iw`（希伯来语）**、`ka`、`ug`

应用内 → 线上映射：**`fil`→`tl`**、**`he`→`iw`**、**`nb`→`no`**。

## 4. Bing

**`auto-detect`**、`zh-Hans`、`zh-Hant`、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt-PT`、`pt`、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、`fil`、`km`、`lo`、`bn`、`my`、**`nb`（书面挪威语）**、**`sr-Cyrl`**、`hr`、**`mn-Mong`**、`he`、`ka`、`ug`

- 源语言自动识别写作 **`auto-detect`**（不是 `auto`）。
- 塞尔维亚语带**脚本后缀** `sr-Cyrl`；蒙古语 `mn-Mong`。

## 5. Baidu

第一批：`auto`、`zh`、**`wyw`（文言文）**、**`cht`（繁体）**、`en`、**`jp`（日语）**、**`kor`（韩语）**、`fra`、`spa`、`pt`、`it`、`de`、`ru`、**`ara`**、`swe`、**`rom`**、`th`、**`slo`**、`nl`、`hu`、`el`、**`dan`**、`fin`、`pl`、`cs`、**`bul`**、`est`、**`vie`**

第二批（"不常见语种"，普通账号返回 **58001**）：**`cat`**、**`pot`（巴葡）**、`tr`、**`lit`**、`lav`、`ukr`、`id`、**`msa`**、**`slv`**、**`per`（波斯语）**、**`hin`**、`tel`、`tam`、`urd`、`fil`、`khm`、`lo`、`ben`、**`bur`**、**`nor`**、**`srp`**、`hrv`、`mon`、**`heb`**、**`geo`**

## 6. Tencent

`auto`、`zh`、**`zh-TW`**、`en`、`ja`、`ko`、`fr`、`es`、`it`、`de`、`tr`、`ru`、`pt`、`vi`、`id`、`th`、`ms`、`ar`、`hi`

`transType` 方向矩阵（`supportedTypes`）：命中矩阵、源=目标、或双方都是中文 → 允许。

| 源 | 允许目标 |
|---|---|
| 中文（简/繁） | en, ja, ko, fr, es, it, de, tr, ru, pt, vi, id, th, ms |
| 英语 | 上述全部 + **ar, hi** |
| 日语 | zh, zh-TW, en, ko |
| 韩语 | zh, zh-TW, en, ja |
| 法/西/意/德/土/俄 | zh, zh-TW, en + 该组其余（各缺自身） |
| 葡语 | zh, zh-TW, en, fr, es, it, de, tr, ru |
| 越/印尼/泰/马来 | zh, zh-TW, en |
| 阿拉伯语 / 印地语 | 仅 en |

不在矩阵且非中文对 → `QueryError(.unsupportedLanguage, "<from> --> <to>")`。

## 7. Alibaba

`auto`、`zh`（**简繁同码**）、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt`、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`bg`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、`fil`、`km`、`lo`、`bn`、`my`、`no`、**`hbs`（克罗地亚语，非 `hr`）**、`mn`、`he`、`ka`

蒙古语仅支持与中文互译；简繁区分靠目标语言判定。

## 8. Volcano

`zh`、**`zh-Hant`**、`vi`、`it`、`id`、`hi`、`en`、`he`、`es`、`uk`、`ur`、`tr`、`th`、`ta`、`te`、`sl`、`sk`、`sv`、`ja`、`pt`、`no`、`my`、`bn`、`mn`、`ms`、`ro`、`lt`、`lv`、`lo`、`hr`、`cs`、`nl`、`ko`、`km`、`fi`、`fr`、`ru`、`de`、`da`、`fa`、`pl`、`bg`、`ar`、`et`、**`lzh`（文言文）**、`sr`、`hu`、`ka`、`ug`

方向限制（`VolcanoTranslateType.transType`）：**斯洛伐克语不能作源**；**维吾尔语只能作源**且目标限中文。

## 9. NiuTrans

与 Google 表同构 46 项，差异：**`cht`（繁体）**、**`pt-BR`**、**`jy`（格鲁吉亚语，非 `ka`）**；其余含 `no`、`sr`、`hr`、`mn`、`he`、`ka` 等。

## 10. Caiyun

`auto`、`zh`、**`zh-Hant`**、`en`、`ja`、`ko`、`de`、`es`、`fr`、`it`、`pt`、`ru`、`tr`、`vi`（14 项，全部服务中最短）。源=目标视为不支持。

## 11. Doubao

`auto`、`zh`、**`zh-Hant`**、`en`、`ja`、`ko`、`de`、`fr`、`es`、`it`、`pt`、`ru`、`th`、`vi`、`ar`、`cs`、`da`、`fi`、`hr`、`hu`、`id`、`ms`、**`nb`**、`nl`、`pl`、`ro`、`sv`、`tr`、`uk`

## 12. LLM 服务（`03-ai-services.md`）

- 13 个服务**不做代码表映射**：直接使用内部 `Language` 的 `rawValue` 拼进提示词（如 `English`、`Simplified-Chinese`），或用 `queryLanguageName` 的三个特例：
  - `.classicalChinese` → `简体中文文言文`
  - `.simplifiedChinese` → `简体中文白话文`
  - `.traditionalChinese` → `繁体中文白话文`
- 唯一例外：**Gemini** 的 `unsupportedLanguages` 移除 10 种——波斯语、菲律宾语、高棉语、老挝语、马来语、蒙古语、缅甸语、泰卢固语、泰米尔语、乌尔都语。

## 13. 移植侧与上游的一致性

`src-tauri/src/service/languages.rs` 逐服务保存上述映射，并提供 `tencent_supports_pair` / `volcano_supports_pair` 两张方向表；`S2T` 字表实现繁简自转（替代 `String.applyingTransform`）。移植的每个语言常量都带注释指向对应的上游文件，可用 `grep -n "zh-CHT" src-tauri/src/service/languages.rs` 抽查。
