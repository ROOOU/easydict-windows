# Easydict 逆向分析报告 — 非 AI 翻译服务

> 目标：把 `01-overview.md` §3 服务清单第 1–14 项（14 个非 AI 服务）的**请求格式、签名算法、语言代码表、长度限制、查询类型分流**逐条固定下来，作为 Windows 版重写的规格。
>
> 上游源码：`Easydict/Swift/Service/`（下称 `Swift/Service/…`）与 `Easydict/objc/Service/…`。
> 移植实现：`src-tauri/src/service/`。
>
> 分析日期：2026-10-16

---

## 0. 公共约定

### 0.1 内置密钥处理

上游 Easydict 把一批**内置凭据**打包进应用：`Swift/Feature/DefaultAPIKeys/EncryptedSecretKeys.plist` 由 `APIKey.swift` 在运行时 AES 解密（算法见 `01-overview.md` §4），另有若干**直接硬编码在源码字面量里**的签名用 key。本卷记录它们的**存在与用途**，但**不复制任何密钥、token 或 secret 的值**；`01-overview.md` §4 已单独列出去混淆后的清单，需要时查阅那里。

规则如下，逐服务适用：

| 服务 | 上游是否内置密钥 | Windows 移植的处理 |
|---|---|---|
| Youdao | **是**（`+Translate` 的 key-fetch key、`+Dict` 的签名 key） | 不随包分发；用户需在设置里填 `youdaoWebApiKey`，否则报 `MissingSecretKey` |
| DeepL | 否（网页模式匿名） | 直接可用 |
| Google | 否（`tk` 由公开 JS 算法生成） | 直接可用 |
| Bing | 否（appid 为公开常量） | 直接可用 |
| Baidu | 否 | 用户自备 AppId/SecretKey |
| Tencent | 否 | 用户自备 SecretId/SecretKey |
| Alibaba | 否 | 用户自备 AccessKeyId/Secret |
| Volcano | 否 | 用户自备 AK/SK |
| NiuTrans | **是**（源码内置回落 key） | 不随包分发；空 key 即报缺密钥 |
| Caiyun | **是**（源码内置回落 token） | 不随包分发；空 token 即报缺密钥 |
| Doubao | 否 | 用户自备 API key |
| Apple / AppleDictionary / MDict | 无（系统/本地能力） | Windows 无对应实现，见 `05-architecture.md` |

### 0.2 截断语义（务必区分）

Swift 侧存在两套互不相同的“取前 N 个字符”：

| 上游写法 | 计数单位 | 出现位置 |
|---|---|---|
| `(text as NSString).trimmingToMaxLength(n)` / `String.truncated(n)`（`substring(to:)`） | **UTF-16 码元** | Google 的 5000 上限、Google 检测的 73 |
| `String(text.prefix(n))` / `trimToMaxLength(n)` | **字素簇（grapheme cluster）** | Bing 的 1000、Ali 的 5000、NiuTrans 的 5000、Bing 网页链接的 450、Bing TTS 的 7000/2000 |

Windows 移植在 `util` 里提供了两个对应函数（`ns_trim_to_max_length` = UTF-16 单位；`prefix_graphemes` = 字素簇），并在每处上游对应位置分别使用。**混用会改变被截断的位置**，尤其是 emoji 与组合字符。

### 0.3 查询类型与中文变体

`EZQueryTextType` 位掩码决定走词典还是翻译（见 `01-overview.md` §2.1）。服务层再叠加两条规则：

- **中文变体转换**：`autoConvertTraditionalChinese()` 为真、且源/目标**都**是中文（`onlyContainsChineseLanguages`）时，`prehandleQueryText` **不发请求**，直接用 `String.applyingTransform("Hans-Hant" / "Hant-Hans")` 在本地转换并返回。上游返回 `true` 的是 **Apple** 与 **DeepL**。Windows 移植用 `service/languages.rs` 的 `S2T` 字表 + 一对多特例表实现同等转换。
- **缺密钥短路**：`apiKeyRequirement() == .userProvided` 且 `hasPrivateAPIKey()` 为假时立刻 `QueryError(.missingSecretKey)`；移植在 `QueryService::prehandle` 中集中实现，并把提示文案改成“请在 设置 → 服务 中填写 … 的 API Key”。

### 0.4 服务基本契约表

| # | 服务 | ServiceType | 上游目录 | 移植文件 | 密钥 | 流式 | 词典 | 繁简自转 |
|---|---|---|---|---|---|---|---|---|
| 1 | Youdao | `Youdao` | `Swift/Service/Youdao/` | `service/youdao.rs` | 上游内置 → 用户 | 否 | **是** | 否 |
| 2 | DeepL | `DeepL` | `Swift/Service/DeepL/` | `service/deepl.rs` | 无（网页）/ 用户（官方 API） | 否 | 否 | **是** |
| 3 | Google | `Google` | `Swift/Service/Google/` | `service/google.rs` | 无 | 否 | **是** | 否 |
| 4 | Bing | `Bing` | `Swift/Service/Bing/` | `service/bing.rs` | 无 | 否 | **是** | 否 |
| 5 | Baidu | `Baidu` | `Swift/Service/Baidu/` | `service/baidu.rs` | 用户 | 否 | 否 | 否 |
| 6 | Tencent | `Tencent` | `Swift/Service/Tencent/` | `service/tencent.rs` | 用户 | 否 | 否 | 否 |
| 7 | Alibaba | `Alibaba` | `Swift/Service/Ali/` | `service/alibaba.rs` | 用户 | 否 | 否 | 否 |
| 8 | Volcano | `Volcano` | `Swift/Service/Volcano/` | `service/volcano.rs` | 用户 | 否 | 否 | 否 |
| 9 | NiuTrans | `NiuTrans` | `Swift/Service/NiuTrans/` | `service/niutrans.rs` | 上游内置 → 用户 | 否 | 否 | 否 |
| 10 | Caiyun | `Caiyun` | `Swift/Service/Caiyun/` | `service/caiyun.rs` | 上游内置 → 用户 | 否 | 否 | 否 |
| 11 | Doubao | `Doubao` | `Swift/Service/Doubao/` | `service/doubao.rs` | 用户 | **是** | 否 | 否 |
| 12 | Apple 系统翻译 | `Apple` | `Swift/Service/Apple/` | **无** | 无（系统） | 否 | 否 | **是** |
| 13 | Apple Dictionary | `AppleDictionary` | `Swift/Service/Dictionary/AppleDictionary/` | **无** | 无（系统） | 否 | **是** | 否 |
| 14 | MDict | `MDict` | `Swift/Service/Dictionary/MDict/` | **无** | 无（本地） | 否 | **是** | 否 |

---

## 1. Youdao 有道

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Youdao/YoudaoService.swift`、`YoudaoService+Translate.swift`、`YoudaoService+Dict.swift`、`Youdao+Language.swift` |
| 移植文件 | `src-tauri/src/service/youdao.rs` |
| `apiKeyRequirement` | 上游 `ServiceAPIKeyRequirement` 为内置；移植改为用户提供 |
| 支持查询类型 | translation + dictionary（**只**在 en/ja/fr/ko ↔ 中文时走词典） |
| 流式 | 否 |

### 1.1 三个端点

| 用途 | 方法 | URL |
|---|---|---|
| 取会话密钥 | GET | `https://dict.youdao.com/webtranslate/key` |
| 网页翻译 | POST（form） | `https://dict.youdao.com/webtranslate` |
| V4 词典 | POST（form） | `https://dict.youdao.com/jsonapi_s?doctype=json&jsonversion=4` |
| 发音 | GET | `https://dict.youdao.com/dictvoice?audio=&le=&type=` |

常量（`YoudaoService+Translate.swift`）：`client=fanyideskweb`、`product=webfanyi`、`appVersion=1.0.0`、`vendor=web`、`pointParam=client,mysticTime,product`、`keyfrom=fanyi.web`；`YoudaoService.swift` 里 `kYoudaoTranslateURL = "https://fanyi.youdao.com"`、`kYoudaoDictURL = "https://dict.youdao.com"`。

请求头：`User-Agent` 为项目统一 UA（`AppConstants.userAgent`），`Referer` 指向 `fanyi.youdao.com`，并带一个**固定写死的 `OUTFOX_SEARCH_USER_ID` cookie**（值见 `01-overview.md` §4）。

### 1.2 签名算法（算法描述，无凭据）

1. `mysticTime` = 当前毫秒时间戳。
2. 取密钥：`sign = md5("client=fanyideskweb&mysticTime=<ms>&product=webfanyi&key=<KEY>")`，KEY 为**上游内置字面量**；`GET /webtranslate/key?keyid=webfanyi-key-getter&mysticTime=<ms>&sign=<sign>` 返回 `{"code":0,"data":{"secretKey":…,"aesKey":…,"aesIv":…}}`。
3. 翻译请求体（form）：`i`、`from`、`to`、`dictResult=false`、`keyid=webfanyi`、`sign`、`mysticTime`，其中 `sign = md5("client=fanyideskweb&mysticTime=<ms>&product=webfanyi&key=<secretKey>")`。
4. 响应是 **URL-safe base64 → AES-128-CBC/PKCS7**，`key = MD5(aesKey)`、`iv = MD5(aesIv)`。

词典签名（`YoudaoService+Dict.swift`）：`ww = text + "webdict"`、`time = ww.count % 10`、`salt = md5(ww)`、`sign = md5("web" + text + time + <DICT_KEY> + salt)`；参数 `q/le/client=web/t/sign/keyfrom=webdict`。`<DICT_KEY>` 在上游是源码字面量，**归档副本中该位置为脱敏占位符，本报告不记录其值**。

### 1.3 语言代码（与 BCP-47 不同处加粗）

`zh-CHS`、`zh-CHT`、`en`、`ja`、`ko`、`fr`、`es`、`pt`、`it`、`de`、`ru`、`ar`、`th`、`nl`、`id`、`vi`。

- 繁体用 **`zh-CHT`**（不是 `zh-Hant`），简体 `zh-CHS`（不是 `zh-CN`）。
- 葡萄牙语只有 `pt`，没有 `pt-PT`/`pt-BR` 之分。
- 无 `auto`：源语言自动识别由本地检测器完成后填入。

### 1.4 长度限制与解析怪癖

- 词典 `time` 字段按 **字素簇** 取模（`ww.count % 10`）；移植显式使用字素计数（`util::grapheme_count`）。
- `/webtranslate` 的 `translateResult` 是嵌套数组，上游先对**每一行**做 `joined("")`，再整体按 `"\n"` split 且**保留空串**（`components(separatedBy:)` 语义）。
- 词典端点仅在 `from`/`to` 属于 {en, ja, fr, ko} × 中文时调用；翻译与词典**并发**发起，任一失败不影响另一个（移植同）。
- 发音：`dictvoice?audio=&le=&type=`，英式 `type=1`、美式 `type=2`；TTS 语言码里中文统一用 `zh`。

### 1.5 移植差异

| 上游 | 移植 |
|---|---|
| key-fetch 与词典签名使用源码内置 key | `default_key()` 读取 `Settings::youdao_web_api_key`，为空即 `MissingSecretKey` 并点名该设置项 |
| `time = ww.count % 10`（Swift `String.count` 即字素） | `util::grapheme_count(&ww) % 10`，语义等价且显式 |
| AES 解密用 CommonCrypto/CryptoSwift | 用 `aes` + `md-5` crate 复现 AES-128-CBC/PKCS7 与 MD5 key/IV |
| 词典结果装配散落在 `+Dict` 扩展 | `apply_dict_v4` 集中映射 `ec`/`ce`/`web_trans`，读音取 `美`/`英`，`exchange` 按 `或` 拆分，`tags` 来自 `exam_type` |

---

## 2. DeepL

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/DeepL/DeepLService.swift`、`DeepLService+Translate.swift`、`DeepLTranslateResponse.swift`、`DeepLWebAppVersionProvider.swift` |
| 移植文件 | `service/deepl.rs` |
| 支持查询类型 | translation + sentence（基类默认） |
| 繁简自转 | **是**（`autoConvertTraditionalChinese() == true`） |
| 流式 | 否 |

### 2.1 两条通道与优先级

`DeepLAPIUsagePriority`（Defaults 存字符串 `"0"|"1"|"2"`）：

| 值 | 枚举 | 行为 |
|---|---|---|
| `0` | `webFirst`（默认） | 先网页 oneshot；失败且有 auth key 时回落到官方 API |
| `1` | `authKeyFirst` | 先官方 API；失败时回落到网页 |
| `2` | `authKeyOnly` | 只走官方 API |

### 2.2 网页（oneshot）通道

```
POST https://oneshot-free.www.deepl.com/v1/translate
Content-Type: application/json
Authorization: None
User-Agent: DeepL/<appVersion> CFNetwork/3826.600.41 Darwin/25.0.0
x-app-os-version: 26.0
x-app-instance-id: <进程级随机 UUID>
x-app-session-id: <进程级随机 UUID>
```

请求体（键序即声明序）：`text: [ ... ]`、`target_lang`、`source_lang`（源为 `auto` 时**整键省略**）、`usage_type: "translate"`、`app_information: {os:"iOS", os_version:"26.0", app_version, app_build:"5443737", instance_id}`。

`appVersion` 由 `DeepLWebAppVersionProvider` 从 App Store 查询：

```
GET https://itunes.apple.com/lookup?id=1552407475&country=us
```

校验 `resultCount == 1 && results.count == 1 && trackId == 1552407475 && bundleId == "com.linguee.DeepLMobileTranslator" && artistName == "DeepL SE" && version 为 2–4 段纯数字`。缓存 24 h，失败回退 15 min，超时 3 s，兜底版本 `"26.52"`。缓存写进 `EZDeepLWebAppVersionCacheKey`。

语言码：`zh-hans`/`zh-hant`/`pt-PT`/`pt-BR`，oneshot 目标码把 `en` 提升为 `en-US`、`zh-hans`→`zh-Hans` 等大小写敏感形式；源码调用 `removeLanguageVariant()` 去掉 `-` 后缀。

### 2.3 官方 API 通道

```
POST https://api.deepl.com/v2/translate          # authKey 以 ":fx" 结尾时改用 https://api-free.deepl.com
Authorization: DeepL-Auth-Key <authKey>
form: text=<text>&source_lang=<src>&target_lang=<tgt>
```

`EZDeepLTranslateEndPointKey` 非空时**完全覆盖**整条 URL。源语言未知时发送字面量 `auto`（与 oneshot 的“省略”相反）。响应 `translations[0].text` 再按 `"\n"` 切段。

错误消息提取顺序：`message` → `title` → `error`（字符串）→ `error.message`。

### 2.4 语言代码

`auto`、`zh-hans`、`zh-hant`、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt-PT`、`pt-BR`、`it`、`de`、`ru`、`sv`、`ro`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`sl`、`et`、`nb`、`ar`、`vi`、`he`、`th`。繁体/简体的**小写** `zh-hans`/`zh-hant` 是 DeepL 侧的实际取值。

### 2.5 移植差异

- 上游的 `DeepLWebTranslationCancellation`（`queryModel.setStop`）**没有对应物**：移植由查询引擎直接丢弃 future 来取消（`service/deepl.rs` 头注释已声明）。
- 移植额外加了 `TEXT_LIMIT = 128 KiB` 的**安全网**截断；上游请求体**不做**长度限制。这是刻意偏离，避免一次查询把整个窗口卡死。
- 版本提供者的缓存与回退时序（24 h / 15 min / 3 s）逐条复刻；`OnceLock` 代替单例。

---

## 3. Google

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Google/GoogleService.swift`、`GoogleService+Translate.swift`、`GoogleService+Language.swift`、`google-translate-sign.js` |
| 移植文件 | `service/google.rs`（TTS URL 另在 `tts/mod.rs` 复刻一份） |
| 支持查询类型 | **dictionary + sentence + translation** |
| 流式 | 否 |

### 3.1 `tk` 签名与冻结的 TKK

`tk` 由上游内嵌的 `google-translate-sign.js`（JavaScriptCore 执行）生成，等价算法：

1. 取 `window.TKK`，按 `.` 切两段，前段记 `b`、后段记 `d1`。
2. 把待签字符串逐字符转 UTF-8 字节序列 `e`。
3. 累加器初值 `a = b`，对每个字节 `e[f]` 执行 `a += e[f]`，再 `a = xr(a, "+-a^+6")`。
4. 之后 `a = xr(a, "+-3^+b+-f")`，`a ^= d1`；负数转 uint32；`a %= 1e6`。
5. 结果字符串 `"<a>.<a ^ b>"`。

`xr(a, s)`：按 3 字符步长扫描 `s`，第 3 个字符为位移量（`'a' <= c` 时取 `c-87`，否则取十进制），第 2 个字符为 `+` 则 `a >>> d`、否则 `a << d`；第 1 个字符为 `+` 则 `(a + d) & 0xFFFFFFFF`，否则 `a ^ d`。

**关键怪癖**：JS 里 `yr` 会缓存第一次读到的 `window.TKK`；上游的 `updateWebAppTKK` 在首次 `sign` 调用**之后**才写 `window.TKK`，因此实际参与签名的永远是 `google-translate-sign.js` 里的**初始常量**（值见 `01-overview.md` §4）。移植把该常量硬编码进 `service/google.rs`（`const TKK`）并在 `tts/mod.rs` 复刻一份，注释说明“`yr` 缓存冻结首次读取”的理由。

TKK 刷新流程（仅 `textToAudio` 的非英文路径触发）：`GET https://translate.google.com`，正则 `tkk:'\d+\.\d+',`（忽略大小写）取第一个匹配，去掉前后 5/2 个字符得到值；若该值的小时部分等于当前 epoch 小时则跳过刷新。

### 3.2 两个翻译端点

| 路径 | 请求 |
|---|---|
| webapp（词典优先） | `GET /translate_a/single?dt=at&dt=bd&dt=ex&dt=ld&dt=md&dt=qca&dt=rw&dt=rm&dt=ss&dt=t&client=webapp&sl=<sl>&tl=<tl>&hl=en&otf=2&ssel=3&tsel=0&kc=6&tk=<sign>&q=<text>` |
| gtx（翻译兜底） | `GET /translate_a/single?q=<text>&sl=<sl>&tl=<tl>&dt=t&dj=1&ie=UTF-8&client=gtx` |

分流：`shouldQueryDictionary(withLanguage:maxWordCount: 1)` 为真走 webapp，否则走 gtx。UA 为固定 `kGoogleUserAgent`（Chrome 77 的 macOS UA）。

webapp 响应下标（0 基）：`[2]` 检测到的源语言；`[0]` 译文行（每行取第一个元素）；`[0][1][3]` 音标；`[1]` 词典（英→中给 `parts`，中→英给 `simpleWords`）。`wordResult` 仅在 `parts != nil || simpleWords != nil || text.count <= 4` 时挂载；两者皆空则落到 gtx。gtx 响应 `sentences[].trans` 用 `""` 拼接后按 `"\n"` 切段。

### 3.3 TTS

- 非英文：`GET /translate_tts?ie=UTF-8&q=<enc>&tl=<lang>&total=1&idx=0&textlen=<n>&tk=<sign>&client=webapp&prev=input`，文本先 `trimmingToMaxLength(200)`（**UTF-16**）。
- 英文：走 `batchexecute` RPC（`rpcids=jQ1olc`），host 由口音决定（uk → `translate.google.co.uk`，否则 `translate.google.as`），请求头带 `X-Same-Domain: 1`、`Origin`、`Referer`；body `"f.req=" + formEncoded(requestString) + "&"`，`requestString` 为 `[[["jQ1olc", <JSON of [text,"en",null,"undefined",[0]]>, null, "generic"]]]`；响应逐行找第一个 trim 后以 `[[` 开头的行，取 `fields[2]` 的 base64 音频。

### 3.4 检测与语言代码

`webAppDetect` 把输入截到 **73 个 UTF-16 单位**，`sl=auto&tl=auto`，读 `[2]`，必要时用 `[8].last.first` 修正 zh-CN/zh-TW。

代码表：`auto`、`zh-CN`、`zh-TW`、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt-PT`、`pt`（巴葡）、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、`tl`（菲律宾语）、`km`、`lo`、`bn`、`my`、`no`（挪威语）、`sr`、`hr`、`mn`、`iw`（希伯来语）、`ka`、`ug`。特殊：菲律宾语 `fil→tl`、希伯来语 `he→iw`、挪威语 `nb→no`。

### 3.5 长度限制与移植差异

| 项 | 上游 | 移植 |
|---|---|---|
| 中文文本上限 | `text.prefix(1800)`（字素簇） | `util::prefix_graphemes(text, 1800)` |
| 非中文文本上限 | `trimmingToMaxLength(5000)`（UTF-16） | `util::ns_trim_to_max_length(text, 5000)` |
| 检测 | `trimmingToMaxLength(73)` | 同（UTF-16） |
| `tk` | JS 引擎执行 + 冻结 TKK | Rust 复刻算法 + 冻结常量；`yr` 缓存语义注释在案 |
| TTS | 英文走 RPC，其它拼 URL | `tts/mod.rs` 复刻非英文 URL 与 200 上限；RPC 路径未实现，英文单词回落系统合成器 |

---

## 4. Bing

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Bing/BingService.swift`、`BingService+Translate.swift`、`BingRequest.swift`、`BingConfig.swift`、`BingTranslateResponse.swift`、`BingLookupResponse.swift`、`BingLanguageVoice.swift` |
| 移植文件 | `service/bing.rs` |
| 支持查询类型 | translation + sentence（基类默认；词典是**内部**分支，不体现在 `supportedQueryType`） |
| 流式 | 否 |

### 4.1 抓取式鉴权（唯一无官方 API 的服务）

1. **主机发现**：若未缓存 host，`GET http://cn.bing.com`（带 UA，可选 Cookie），取**重定向后的** `response.url.host`；传输失败则回落 `cn.bing.com`。
2. **配置抓取**：`GET https://{host}/translator`（只带 UA，不带 Cookie），三条正则取字段：
   - `IG:\s*"([^"]+)"`
   - `data-iid\s*=\s*"([^"]+)"`
   - `params_AbusePreventionHelper\s*=\s*\[([^\]]+)\]`
     第三项先删掉所有 `"`，再按 `","` 切三段：`[0]`=毫秒时间戳 `key`、`[1]`=`token`、`[2]`=`expirationInterval`（通常 `3600000`）。
3. **过期判定**：`now - key > expirationInterval / 2`（默认 30 分钟后重抓）。缓存写在 `UserDefaults` 的 `kBingConfigKey`（JSON 编码 `BingConfig`）。

### 4.2 请求

```
POST https://{host}/ttranslatev3?isVertical=1&IG=<IG>&IID=<IID>
form: text=<t>&to=<to>&token=<token>&key=<key>&fromLang=<from>&tryFetchingGenderDebiasedTranslations=true

POST https://{host}/tlookupv3?isVertical=1&IG=<IG>&IID=<IID>
form: text=<t>&to=<to>&token=<token>&key=<key>&from=<from>        # 注意是 from，不是 fromLang

GET  https://{host}/api/v7/dictionarywords/search?appid=<公开 appid>&mkt=zh-cn&pname=bingdict&q=<t>
```

词典端点只在 `from == english && to == simplifiedChinese` 且文本是**单个英文单词（≤20 字符）**时调用；失败静默回落普通翻译。公开 appid 的值见 `01-overview.md` §4。

### 4.3 重试与错误

| 情形 | 行为 |
|---|---|
| 翻译响应是 JSON 对象且 `statusCode == 205` | token 失效：`reset()` + 重新抓取配置，**只重试一次**（`canRetry` 守卫），文案 "token invalid, please try again or restart the app." |
| 翻译与词典响应**都**为空 | `reset()`、清空 host，重新走一遍主机发现 + 抓取，只重试一次 |
| HTTP 429 | `QueryError(.api, "429 error, Bing translate too many requests")` |
| TTS 响应 MIME ≠ `audio/mpeg` | 若重定向到别的 host 则更新 host 并重试一次 |

### 4.4 词典解析（`processWordSimpleWordAndPart` + `parseBingDictTranslate`）

- `tlookupv3`：按 `posTag` 分组；中→英产出 `simpleWords`（`means` 取 `backTranslations[].displayText`），其余产出 `parts`。
- 英→中走字典端点：`value[0].meaningGroups[].partsOfSpeech[0].description` 决定分支（**中文描述字面量**）：`发音`（只认 `US`/`UK`，US 的 audio URL 原样，UK 的把 `tom` 替换成 `george`）、`快速释义`、`词组`（`examples.count == 2`）、`分类词典`（同/反义词）、`搭配`；另独立判定 `name == "变形"` 产出 `exchanges`（按全角冒号切两段）。
- 音标仅在 `isShortWordLength(text, language:)` 为真时产出。

### 4.5 语言代码

`auto-detect`、`zh-Hans`、`zh-Hant`、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt-PT`、`pt`、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`uk`、`bg`、`id`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、`fil`、`km`、`lo`、`bn`、`my`、`nb`（书面挪威语）、`sr-Cyrl`、`hr`、`mn-Mong`、`he`、`ka`、`ug`。特殊：塞尔维亚语带脚本后缀 `sr-Cyrl`，蒙古语 `mn-Mong`，源语言自动识别用 `auto-detect` 而非 `auto`。

### 4.6 TTS（`tfettts`）

```
POST https://{host}/tfettts?isVertical=1&IG=<IG>&IID=<IID>
form: ssml=<ssml>&token=<token>&key=<key>
```

SSML 模板：`<speak version="1.0" xml:lang='<lang>'><voice name='<voiceName>'><prosody rate='-10%'>…</prosody></voice></speak>`；文本上限 **7000**（拉丁语系）或 **2000**（中日韩泰老缅等不用空格分词的语言，判据 `isLanguageWordsNeedSpace`），均按字素截断并做 XML 转义（`& < > ' "` 五个实体）。英式发音把语音换为 `en-GB` / `en-GB-SoniaNeural`。语音表 `BingLanguageVoice.languageVoices` 覆盖 47 种语言（zh-CN-XiaoxiaoNeural、en-US-JennyNeural、ja-JP-NanamiNeural …），未命中回落 `en-US` / `en-US-JennyNeural`。`classicalChinese` 先归一为 `simplifiedChinese`。

### 4.7 长度限制与移植差异

| 项 | 上游 | 移植 |
|---|---|---|
| 正文 | `text.prefix(1000)`（字素） | `util::prefix_graphemes(text, 1000)` |
| 中文网页链接 | `trimToMaxLength(450)`（字素） | 同 |
| 配置缓存 | `UserDefaults` 持久化 | **仅内存**（进程生命周期）。头注释说明：`BingRequest` 只用它做请求，持久化没有收益 |
| 词典回落 | `bingTranslate(_:useDictQuery:…)` 内联处理 | 移植保留同一条回落路径 |

---

## 5. Baidu 百度

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Baidu/BaiduService.swift`、`BaiduApiTranslate.swift` |
| 移植文件 | `service/baidu.rs` |
| 密钥 | 用户（`EZBaiduAppId` + `EZBaiduSecretKey`，另有 `EZBaiduServiceApiTypeKey` 选择网页/密钥模式） |
| 流式 | 否 |

### 5.1 端点

```
POST https://fanyi-api.baidu.com/api/trans/vip/translate
form: q=<text>&from=<from>&to=<to>&appid=<appid>&salt=<salt>&sign=<sign>
sign = md5(appId + utf8(text) + salt + secretKey)          # 拼接后整体 MD5，十六进制小写
salt = 大写带连字符的 UUID 字符串（不是数字 nonce）
```

```
POST https://fanyi.baidu.com/langdetect      # 语言检测，文本截到 73 字符（查询参数）
GET  https://fanyi.baidu.com/gettts?text=<t>&lan=<lan>&spd=<spd>   # spd=5（中文）/ 3（其它），文本上限 1000
```

### 5.2 语言代码

`auto`、`zh`、`wyw`（**文言文**）、`cht`（繁体）、`en`、`jp`（**日语**）、`kor`（**韩语**）、`fra`、`spa`、`pt`、`it`、`de`、`ru`、`ara`、`swe`、`rom`、`th`、`slo`、`nl`、`hu`、`el`、`dan`、`fin`、`pl`、`cs`、`bul`、`est`、`vie`；第二批“不常见语种”：`cat`、`pot`、`tr`、`lit`、`lav`、`ukr`、`id`、`msa`、`slv`、`per`、`hin`、`tel`、`tam`、`urd`、`fil`、`khm`、`lo`、`ben`、`bur`、`nor`、`srp`、`hrv`、`mon`、`heb`、`geo`。

与 BCP-47 的差异：日语 `jp`、韩语 `kor`、文言文 `wyw`、繁体 `cht`、法语 `fra`、西语 `spa`、阿拉伯语 `ara`、罗马尼亚语 `rom`、斯洛伐克语 `slo`、丹麦语 `dan`、芬兰语 `fin`、保加利亚语 `bul`、爱沙尼亚语 `est`、越南语 `vie`、巴葡 `pot`、波斯语 `per`、印地语 `hin`、格鲁吉亚语 `geo` 等。

### 5.3 限制与错误

- 文本上限 **5000**（移植 `TEXT_LIMIT = 5000`）。
- 不常见语种对普通账号返回错误码 **58001**，上游专门加了提示文案；移植在错误分类里同样处理。
- 无词典、无流式、无繁简自转。

---

## 6. Tencent 腾讯云

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Tencent/TencentService.swift`、`TencentSigning.swift`、`TencentTranslateType.swift`、`TencentResponse.swift` |
| 移植文件 | `service/tencent.rs` |
| 密钥 | 用户提供 **SecretId + SecretKey 两个值**（`hasPrivateAPIKey()` 要求两者都非空） |
| `link()` | `https://fanyi.qq.com`；`wordLink` 未覆写 → 回落 `link()` |
| 免费额度常量 | `totalFreeQueryCharacterCount() = 500 * 10000` |

### 6.1 请求

```
POST https://tmt.tencentcloudapi.com
Content-Type: application/json; charset=utf-8
X-TC-Action: TextTranslate
X-TC-Timestamp: <unix 秒>
X-TC-Version: 2018-03-21
X-TC-Region: ap-guangzhou
body: {"SourceText":"<text>","Source":"<src>","Target":"<tgt>","ProjectId":0}
```

### 6.2 TC3 签名（逐步）

1. **规范请求**
   ```
   POST
   /
   <空行：canonicalQueryString 为空>
   content-type:application/json; charset=utf-8
   host:tmt.tencentcloudapi.com
   x-tc-action:texttranslate
   
   content-type;host;x-tc-action
   <sha256(请求体的 JSON 字节) 小写十六进制>
   ```
2. **待签字符串**：`TC3-HMAC-SHA256\n<timestamp>\n<credentialScope>\n<sha256(canonicalRequest)>`，其中 `credentialScope = "<YYYY-MM-DD UTC>/<service>/<终止符>"`。
3. **签名**：四段 HMAC-SHA256，密钥逐级派生——第一段的 HMAC 密钥是字符串 `"TC3" + secretKey`、待签数据是日期；之后依次用上一段的结果作密钥，对服务名、终止符、待签字符串求 HMAC；中间结果保持原始 32 字节，只有最终 signature 转小写十六进制。
4. **Authorization**：`TC3-<算法标识> Credential=<secretId>/<scope>, SignedHeaders=content-type;host;x-tc-action, Signature=<hex>`。

> **关于终止符**：归档副本的 `TencentSigning.swift` 在该位置显示的是脱敏占位符，本报告不记录其值，也不做还原。该字符串是腾讯云 TC3 协议的**公开协议常量**（不是密钥），可在腾讯云官方 TC3 文档中查得；移植实现按协议写入同一常量。

### 6.3 语言与方向矩阵

代码表：`auto`、`zh`、`zh-TW`、`en`、`ja`、`ko`、`fr`、`es`、`it`、`de`、`tr`、`ru`、`pt`、`vi`、`id`、`th`、`ms`、`ar`、`hi`。繁体用 **`zh-TW`**。

`supportedTypes` 方向矩阵（`transType` 判定：命中矩阵 **或** 源=目标 **或** 双方都是中文）：

| 源 | 允许目标 |
|---|---|
| 简体/繁体中文 | en, ja, ko, fr, es, it, de, tr, ru, pt, vi, id, th, ms |
| 英语 | zh, zh-TW, ja, ko, fr, es, it, de, tr, ru, pt, vi, id, th, ms, **ar, hi** |
| 日语 | zh, zh-TW, en, ko |
| 韩语 | zh, zh-TW, en, ja |
| 法/西/意/德/土/俄 | zh, zh-TW, en 及该组其余语言（各缺自己一项） |
| 葡语 | zh, zh-TW, en, fr, es, it, de, tr, ru |
| 越/印尼/泰/马来 | zh, zh-TW, en |
| 阿拉伯语 / 印地语 | **仅 en** |

不支持的组合直接抛 `QueryError(.unsupportedLanguage, "\(from) --> \(to)")`（用本地化显示名）。注意上游这里与 `URL` 层的差异：zh↔zh-TW 虽不在文档里，但被 `onlyContainsChineseLanguages` 放行。

### 6.4 移植差异（重要）

上游 `TencentSigning` 用 `JSONSerialization.data(withJSONObject:)` 计算 payload 哈希，却把**同一个字典**交给 Alamofire 重新编码发送——**两次编码、两种键序**，哈希只在巧合下与请求体一致。移植 `service/tencent.rs` **只序列化一次**，把同一份字节既做哈希又做请求体（头注释已声明）。这是行为**修正**，不变更对外协议。

---

## 7. Alibaba 阿里云

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Ali/AliService.swift`、`AliTranslateType.swift` |
| 移植文件 | `service/alibaba.rs` |
| 密钥 | 用户（`EZAliAccessKeyId` + `EZAliAccessKeySecret`） |
| 流式 | 否 |

### 7.1 请求与签名

`POST https://mt.aliyuncs.com`，参数（RPC 风格，全部当作查询参数）：

`Action=TranslateGeneral`、`Version=2018-10-12`、`Format=JSON`、`FormatType=text`、`Scene=general`、`SourceLanguage`、`TargetLanguage`、`SourceText`、`SignatureMethod=HMAC-SHA1`、`SignatureVersion=1.0`、`SignatureNonce=<UUID>`、`Timestamp=<ISO8601 UTC>`、`AccessKeyId`。

签名算法：

1. 参数按 key 字典序排列，每个都用 RFC-3986 编码后拼成 `k=v&k=v…` 的规范串。
2. `stringToSign = "POST" + "&" + encode("/") + "&" + encode(规范串)`（`encode("/")` 即 `%2F`）。
3. 该字符串先经过 **`.nonLossyASCII`**（把每个非 ASCII 字符转义为 `\uXXXX`），再用 `HMAC-SHA1(key = secret + "&", stringToSign)` 并 **base64**。
4. 每次请求都带新 `SignatureNonce` 与 `Timestamp`。

`nonLossyASCII` 那一步是签名能对上的关键——移植 `service/alibaba.rs` 头注释专门记了这一点。

### 7.2 语言代码

`auto`、`zh`（简繁同码）、`en`、`ja`、`ko`、`fr`、`es`、`ca`、`pt`、`it`、`de`、`ru`、`ar`、`sv`、`ro`、`th`、`sk`、`nl`、`hu`、`el`、`da`、`fi`、`pl`、`cs`、`tr`、`lt`、`lv`、`bg`、`ms`、`sl`、`et`、`vi`、`fa`、`hi`、`te`、`ta`、`ur`、`fil`、`km`、`lo`、`bn`、`my`、`no`、**`hbs`**（克罗地亚语，非 `hr`）、`mn`、`he`、`ka`。

- 简体与繁体在代码表里**都是 `zh`**，zh↔zh-TW 的区分靠目标语言判定（移植与上游一致）。
- 蒙古语仅在与中文互译时可用。

### 7.3 限制

`SourceText` 前缀截断 **5000**，按**字素簇**（上游 `text.prefix(limit)`）——移植使用 `util::prefix_graphemes`。

---

## 8. Volcano 火山翻译

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Volcano/VolcanoService.swift`、`VolcanoSigning.swift`、`VolcanoTranslateType.swift` |
| 移植文件 | `service/volcano.rs` |
| 密钥 | 用户（`EZVolcanoAccessKeyID` + `EZVolcanoSecretAccessKey`） |
| 流式 | 否 |

### 8.1 请求

```
POST https://translate.volcengineapi.com/?Action=TranslateText&Version=2020-06-01
X-Date: yyyyMMdd'T'HHmmss'Z'
body: {"SourceLanguage":…,"TargetLanguage":…,"TextList":[…]}
```

### 8.2 签名（Volcengine 变种 HMAC-SHA256）

1. 算法标识是**裸的 `HMAC-SHA256`**（没有 `AWS4-` 前缀）。
2. 规范头：把 `host`、`x-date` 等头**小写化并去首尾空格**后按字母序拼接。
3. `credentialScope = "<shortDate>/<region>/<service>/request"`（区域 `cn-north-1`、服务 `translate`，终止符字面量是 `request`）。
4. `Authorization` 头构造为 `"{算法} Credential=…,  SignedHeaders=…, Signature=…"`——**逗号后是两个空格**，这是上游原样保留的怪癖。移植 `service/volcano.rs` 头注释明确写着“保留项目原始的双空格”。

### 8.3 语言代码与方向限制

`zh`、`zh-Hant`、`vi`、`it`、`id`、`hi`、`en`、`he`、`es`、`uk`、`ur`、`tr`、`th`、`ta`、`te`、`sl`、`sk`、`sv`、`ja`、`pt`、`no`、`my`、`bn`、`mn`、`ms`、`ro`、`lt`、`lv`、`lo`、`hr`、`cs`、`nl`、`ko`、`km`、`fi`、`fr`、`ru`、`de`、`da`、`fa`、`pl`、`bg`、`ar`、`et`、**`lzh`**（文言文）、`sr`、`hu`、`ka`、`ug`。

方向限制（`VolcanoTranslateType.transType`）：斯洛伐克语**不能作为源**；维吾尔语**只能作为源**且目标限中文；移植的 `volcano_supports_pair` 复刻同一张表。

---

## 9. NiuTrans 小牛

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/NiuTrans/NiuTransService.swift` |
| 移植文件 | `service/niutrans.rs` |
| 密钥 | 上游：用户 key 为空时回落到**源码内置 key**；移植：不回落到内置池 |

```
POST https://api.niutrans.com/NiuTransServer/translation
form: apikey=<key>&src_text=<truncated>&from=<from>&to=<to>&source=Easydict
```

- `src_text` 截断 **5000**（字素簇），移植 `TEXT_LIMIT = 5000`。
- `source=Easydict` 是上游固定的产品标识，移植原样保留。
- 语言代码差异：繁体 **`cht`**、巴葡 `pt-BR`、格鲁吉亚语 **`jy`**（非 `ka`）；其余 46 项与 Google 表同构（`no` 表示挪威语、`sr`、`hr`、`mn`、`he`、`ka`）。
- **移植差异**：`service/niutrans.rs` 头注释写明“本移植不内置 key 池，因此未填 key 与缺少用户 key 的报错路径一致”。

---

## 10. Caiyun 彩云小译

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Caiyun/CaiyunService.swift`、`CaiyunTranslateType.swift` |
| 移植文件 | `service/caiyun.rs` |
| 密钥 | 上游内置 token 回落；移植要求用户提供 |

```
POST https://api.interpreter.caiyunai.com/v1/translator
content-type: application/json
x-authorization: token <token>
body: {
  "source": ["<按 \n 切分的行，保留空行>"],
  "trans_type": "<from>2<to>",
  "media": "text",
  "request_id": "Easydict",
  "detect": <trans_type 是否以 "auto" 开头>
}
```

- `trans_type` 字面量形如 `zh2en`、`auto2zh`；源语言与目标语言**相同**视为不支持。
- 语言码：`auto`、`zh`、**`zh-Hant`**、`en`、`ja`、`ko`、`de`、`es`、`fr`、`it`、`pt`、`ru`、`tr`、`vi`（共 14 项，是全部服务里最短的一张表）。
- 请求体把整段文本按换行拆成数组（保留空行），响应按行合并。
- 移植 `service/caiyun.rs` 保留 `<from>2<to>` 与 `request_id` 字面量。

---

## 11. Doubao 豆包翻译

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Doubao/DoubaoService.swift`、`DoubaoTranslateType.swift` |
| 移植文件 | `service/doubao.rs` |
| 密钥 | 用户（`EZDoubaoAPIKey`，模型 `EZDoubaoModelKey`） |
| 流式 | **是**（14 个非 AI 服务里唯一流式的） |
| 支持查询类型 | **仅 translation** |

### 11.1 请求（Ark Responses API，不是 chat/completions）

```
POST https://ark.cn-beijing.volces.com/api/v3/responses
Authorization: Bearer <key>
body: {
  "model": "<model>",                       // 默认 doubao-seed-translation-250915
  "stream": true,
  "input": [{
    "role": "user",
    "content": [{
      "type": "input_text",
      "text": "<text>",
      "translation_options": {"source_language": "<src>", "target_language": "<tgt>"}
    }]
  }]
}
```

### 11.2 SSE 解析

- 事件以 `"\n\n"` 分块（上游自己按该分隔符切，不用通用 SSE 解析器）。
- 只消费 `event: response.output_text.delta`，且 delta 位于**顶层** `delta` 字段（不是 `choices[0].delta.content`）。
- 解码阈值 1 KB（攒够 1 KB 或遇到换行才尝试解码），`[DONE]` 直接忽略。

### 11.3 语言代码

`auto`、`zh`、`zh-Hant`、`en`、`ja`、`ko`、`de`、`fr`、`es`、`it`、`pt`、`ru`、`th`、`vi`、`ar`、`cs`、`da`、`fi`、`hr`、`hu`、`id`、`ms`、**`nb`**（书面挪威语）、`nl`、`pl`、`ro`、`sv`、`tr`、`uk`。

### 11.4 移植差异

默认模型常量 `DEFAULT_MODEL = "doubao-seed-translation-250915"` 与上游 `defaultModelIdentifier` 一致；移植在设置里暴露为 `doubaoModel`，可在 UI 中覆盖。

---

## 12. Apple 系统翻译（macOS 专有）

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Apple/AppleService.swift`、`AppleTranslation/AppleTranslation.swift`、`AppleLanguageDetector`、`AppleOCREngine`、`AppleSpeechService` |
| 移植 | **无实现**；`ServiceType::is_available_on_windows()` 返回 `false` |

上游实现要点（供对照）：

- 翻译走 macOS 15+ 的 **`Translation` 框架**；先用 `LanguageAvailability` 判断语言对可用性；框架不可用时退回隐藏的 `NSHostingController`/窗口方案，再退回 **AppleScript**（`runTranslateShortcut` 触发“翻译”快捷指令）。
- `autoConvertTraditionalChinese()` 返回 **true** → zh-Hans ↔ zh-Hant 的查询在**本地**完成，不触发系统面板。
- 语言检测用 `NLLanguageRecognizer`；OCR 用 Vision（`VNRecognizeTextRequest`）；朗读用 `NSSpeechSynthesizer`。

Windows 的等价替换见 `05-architecture.md` 的替换表：Vision → `Windows.Media.Ocr`，`NSSpeechSynthesizer` → `Windows.Media.SpeechSynthesis`，`NLLanguageRecognizer` → `src/detect/` 的 Unicode 脚本分类器。**系统翻译面板本身没有 Windows 对应物**，因此 `Apple` 服务在 Windows 版中不可用（设置文件仍保留该条目以便往返）。

---

## 13. Apple Dictionary（macOS 专有）

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Dictionary/AppleDictionary/AppleDictionary.swift` |
| 移植 | **无实现** |

- 包装 **DictionaryKit / `DictionaryServices.framework`** 的 `TTTDictionary`（私有 API）。
- `supportedQueryType()` = dictionary + sentence。
- 结果不是纯文本，而是用 `DictionaryHTMLRenderer` 生成的 **iframe HTML**（`bodyMargin: 10`），并把词典资源（音频/图片/字体）以 data URL 内嵌：单个资源 8 MiB 上限、字体 512 KiB、单次查询总量 16 MiB。
- 词条校验规则（词头合法性）与 `wordLink = "dict://<encoded>"`。
- Windows 无 `DictionaryServices`，`service/registry.rs` 头注释写明它“包装私有 `DictionaryServices.framework`”，因此不进入注册表；替代方案是 MDict 或自研词典格式（见 `01-overview.md` §6 第 6 项）。

---

## 14. MDict（本地词典）

| 项 | 值 |
|---|---|
| 上游文件 | `Swift/Service/Dictionary/MDict/MDictService.swift`、`MDictDictionary.swift` |
| 移植 | **未实现**（`ServiceType` 保留，`is_implemented()` 为假；前端文案“本地 MDict 词典在 Windows 版中尚未实现。”） |

上游要点：

- 直接解析本地 `.mdx` / `.mdd` 文件，渲染到专用 WebView 页面；页面注入 `darkreader.min`、`MDictStyleScript`、`MDictEntryScript`，并带 CSP meta。
- 资源限制：data URI 缓存总量 32 MiB、单个 8 MiB、单次查询 1536 KiB / 96 个、CSS 4 MiB、缺失资源 512 个、搜索索引 200 000 条、链接解析深度 8。
- 词条跳转支持 `@@@LINK=` 间接引用。
- 主要未确定项是 **LZO 解压状态机**（见 `01-overview.md` §6 第 3 项）。

---

## 15. 汇总

### 15.1 端点一览

| 服务 | 端点 | 方法 | 编码 |
|---|---|---|---|
| Youdao | `dict.youdao.com/webtranslate/key` → `/webtranslate` → `/jsonapi_s` → `/dictvoice` | GET / POST / POST / GET | form / form / form / query |
| DeepL | `oneshot-free.www.deepl.com/v1/translate`；`api.deepl.com/v2/translate` | POST | JSON / form |
| Google | `translate.google.com/translate_a/single`（`client=webapp` 或 `gtx`）；`/translate_tts`；`/_/TranslateWebserverUi/data/batchexecute` | GET / GET / POST | query |
| Bing | `{host}/translator`、`/ttranslatev3`、`/tlookupv3`、`/tfettts`、`/api/v7/dictionarywords/search` | GET / POST×2 / POST / GET | 抓取 / form |
| Baidu | `fanyi-api.baidu.com/api/trans/vip/translate`；`fanyi.baidu.com/langdetect`、`/gettts` | POST / POST / GET | form / query |
| Tencent | `tmt.tencentcloudapi.com` | POST | JSON |
| Alibaba | `mt.aliyuncs.com` | POST | query（RPC） |
| Volcano | `translate.volcengineapi.com/?Action=TranslateText&Version=2020-06-01` | POST | JSON |
| NiuTrans | `api.niutrans.com/NiuTransServer/translation` | POST | form |
| Caiyun | `api.interpreter.caiyunai.com/v1/translator` | POST | JSON |
| Doubao | `ark.cn-beijing.volces.com/api/v3/responses` | POST | JSON（SSE） |

### 15.2 签名方案一览

| 服务 | 方案 |
|---|---|
| Youdao | MD5 签名 + AES-128-CBC/PKCS7（key/IV 各取 MD5）；词典另有 MD5 组合签名 |
| DeepL | 网页版只带 `Authorization: None`；官方 API 用 `DeepL-Auth-Key` 头 |
| Google | `tk` = JS `sM` 算法（冻结 TKK 的变体） |
| Bing | 页面抓取得到的 IG/IID + 带时间戳的 key/token |
| Baidu | `md5(appid + text + salt + secret)`，salt 为 UUID |
| Tencent | TC3（规范请求 + 凭据范围 + 四段派生 HMAC-SHA256） |
| Alibaba | RPC 风格 HMAC-SHA1（`POST&%2F&<编码查询串>`，经 `.nonLossyASCII`） |
| Volcano | Volcengine HMAC-SHA256，scope 以 `request` 结尾，`Authorization` 逗号后双空格 |
| NiuTrans | 无签名（仅 apikey 参数） |
| Caiyun | `x-authorization: token <token>` |
| Doubao | `Authorization: Bearer` |

### 15.3 移植状态

- **已实现（11 个）**：Youdao、DeepL、Google、Bing、Baidu、Tencent、Alibaba、Volcano、NiuTrans、Caiyun、Doubao。
- **未实现（3 个，macOS 专有）**：Apple、AppleDictionary、MDict。
- 移植侧的注册表 `service/registry.rs::service_types()` 过滤条件是 `is_available_on_windows() && is_implemented()`；`ServiceType::all()` 仍包含全部类型，保证旧设置文件可无损往返（`descriptors()` 对这些条目输出 `available: false`）。
- 全部**内置密钥均未随包分发**；缺失时统一报 `MissingSecretKey` 并提示在“设置 → 服务”中填写（详见 §0.1）。
