# 服务端点 / 请求 / 限制 / 签名摘录

对应卷：`02-services.md`（非 AI 服务）、`03-ai-services.md`（LLM 服务）。
下表所有端点、参数名与长度上限均按上游源码字符串常量转录；**不含任何凭据值**——凡上游内置的 key / token / cookie / appid 一律只注明"由上游内置，值不在此记录"。

## 1. 端点总表

| 服务 | 方法 | URL | 编码 | 关键参数 |
|---|---|---|---|---|
| Youdao 取密钥 | GET | `https://dict.youdao.com/webtranslate/key` | query | `keyid=webfanyi-key-getter&mysticTime&sign` |
| Youdao 翻译 | POST | `https://dict.youdao.com/webtranslate` | form | `i` `from` `to` `dictResult=false` `keyid=webfanyi` `sign` `mysticTime` |
| Youdao 词典 | POST | `https://dict.youdao.com/jsonapi_s?doctype=json&jsonversion=4` | form | `q` `le` `client=web` `t` `sign` `keyfrom=webdict` |
| Youdao 发音 | GET | `https://dict.youdao.com/dictvoice` | query | `audio` `le` `type`（英式 1 / 美式 2） |
| DeepL 网页 | POST | `https://oneshot-free.www.deepl.com/v1/translate` | JSON | `text[]` `target_lang` `source_lang?` `usage_type=translate` `app_information{…}` |
| DeepL 官方 | POST | `https://api.deepl.com/v2/translate`（key 以 `:fx` 结尾改用 `api-free.deepl.com`） | form | `text` `source_lang` `target_lang` |
| DeepL 版本查询 | GET | `https://itunes.apple.com/lookup?id=1552407475&country=us` | query | 校验 `trackId`/`bundleId`/`artistName`/版本号为 2–4 段数字 |
| Google 词典优先 | GET | `https://translate.google.com/translate_a/single`（`client=webapp`，`dt=at&bd&ex&ld&md&qca&rw&rm&ss&t`，`tk=…`） | query | `sl` `tl` `hl=en` `otf=2` `ssel=3` `tsel=0` `kc=6` `q` |
| Google 兜底 | GET | 同上（`client=gtx&dt=t&dj=1&ie=UTF-8`） | query | `q` `sl` `tl` |
| Google TTS（非英文） | GET | `https://translate.google.com/translate_tts` | query | `ie=UTF-8&q&tl&total=1&idx=0&textlen&tk&client=webapp&prev=input` |
| Google TTS（英文 RPC） | POST | `https://translate.google.co.uk` 或 `translate.google.as` 的 `/_/TranslateWebserverUi/data/batchexecute` | form | `f.req=[[["jQ1olc", <JSON>, null, "generic"]]]`，头 `X-Same-Domain: 1` |
| Google 检测 | GET | 同 `translate_a/single` | query | `sl=auto&tl=auto`，读下标 `[2]` |
| Bing 主机发现 | GET | `http://cn.bing.com` | — | 取重定向后 host |
| Bing 配置抓取 | GET | `https://{host}/translator` | — | 正则 `IG:`、`data-iid=`、`params_AbusePreventionHelper=` |
| Bing 翻译 | POST | `https://{host}/ttranslatev3?isVertical=1&IG=<IG>&IID=<IID>` | form | `text` `to` `token` `key` `fromLang` `tryFetchingGenderDebiasedTranslations=true` |
| Bing 查询释义 | POST | `https://{host}/tlookupv3?isVertical=1&IG=&IID=` | form | `text` `to` `token` `key` **`from`**（不是 `fromLang`） |
| Bing 英中词典 | GET | `https://{host}/api/v7/dictionarywords/search` | query | `appid`（公开常量，值不记录）`mkt=zh-cn` `pname=bingdict` `q` |
| Bing TTS | POST | `https://{host}/tfettts?isVertical=1&IG=&IID=` | form | `ssml` `token` `key` |
| Baidu 翻译 | POST | `https://fanyi-api.baidu.com/api/trans/vip/translate` | form | `q` `from` `to` `appid` `salt` `sign` |
| Baidu 检测 | POST | `https://fanyi.baidu.com/langdetect` | query | 文本截到 73 |
| Baidu TTS | GET | `https://fanyi.baidu.com/gettts` | query | `text` `lan` `spd`（中文 5 / 其它 3），上限 1000 |
| Tencent | POST | `https://tmt.tencentcloudapi.com` | JSON | 头 `X-TC-Action: TextTranslate` `X-TC-Timestamp` `X-TC-Version: 2018-03-21` `X-TC-Region: ap-guangzhou`；体 `SourceText` `Source` `Target` `ProjectId: 0` |
| Alibaba | POST | `https://mt.aliyuncs.com` | query（RPC） | `Action=TranslateGeneral` `Version=2018-10-12` `Format=JSON` `FormatType=text` `Scene=general` `SourceLanguage` `TargetLanguage` `SourceText` `SignatureMethod=HMAC-SHA1` `SignatureVersion=1.0` `SignatureNonce` `Timestamp` `AccessKeyId` |
| Volcano | POST | `https://translate.volcengineapi.com/?Action=TranslateText&Version=2020-06-01` | JSON | 头 `X-Date`；体 `SourceLanguage` `TargetLanguage` `TextList` |
| NiuTrans | POST | `https://api.niutrans.com/NiuTransServer/translation` | form | `apikey` `src_text` `from` `to` **`source=Easydict`** |
| Caiyun | POST | `https://api.interpreter.caiyunai.com/v1/translator` | JSON | 头 `x-authorization: token <token>`；体 `source[]` `trans_type=<from>2<to>` `media=text` `request_id=Easydict` `detect` |
| Doubao | POST | `https://ark.cn-beijing.volces.com/api/v3/responses` | JSON(SSE) | 头 `Authorization: Bearer`；体 `model` `stream:true` `input[0].content[0].{type:"input_text",text,translation_options{source_language,target_language}}` |

## 2. 长度上限与计数单位

| 服务 | 上限 | 单位 | 上游写法 |
|---|---|---|---|
| Youdao | 无显式上限（词典 `time` 字段按字素取模） | 字素（模运算） | `ww.count % 10` |
| Google 正文（中文） | 1800 | 字素簇 | `text.prefix(1800)` |
| Google 正文（非中文） | 5000 | UTF-16 | `trimmingToMaxLength(5000)` |
| Google 检测 | 73 | UTF-16 | `trimmingToMaxLength(73)` |
| Google TTS 非英文 | 200 | UTF-16 | `trimmingToMaxLength(200)` |
| Bing 正文 | 1000 | 字素簇 | `text.prefix(1000)` |
| Bing 中文网页链接 | 450 | 字素簇 | `trimToMaxLength(450)` |
| Bing 词典触发 | 单个英文单词 ≤ 20 字符 | 字符 | 单词判定 |
| Bing TTS | 7000（拉丁语系）/ 2000（CJK 等） | 字素簇 | `prefix(…)` 后 XML 转义 |
| Baidu | 5000 | 移植常量 `TEXT_LIMIT = 5000` | — |
| Baidu 检测 | 73 | 字符 | 源码注释 |
| Baidu TTS | 1000 | 字符 | — |
| Alibaba | 5000 | 字素簇 | `text.prefix(limit)` |
| NiuTrans | 5000 | 字素簇 | `text.prefix(5000)` |
| DeepL | 上游无限制；**移植**加 128 KiB 安全网 | 字节 | 移植自有常量 |
| 其余（Tencent/Volcano/Caiyun/Doubao/Bing TTS 除外） | 无显式上限 | — | — |

## 3. 签名方案（算法层，无凭据）

| 服务 | 算法 |
|---|---|
| Youdao | `mysticTime` = ms 时间戳；`sign = md5("client=fanyideskweb&mysticTime=<ms>&product=webfanyi&key=<内置 KEY>")` 换取 `secretKey/aesKey/aesIv`；翻译 `sign` 换用 `secretKey`；响应 = URL-safe base64 → AES-128-CBC/PKCS7，`key = MD5(aesKey)`、`iv = MD5(aesIv)`；词典 `salt = md5(text+"webdict")`、`time = (text+"webdict").count % 10`、`sign = md5("web"+text+time+<内置 DICT_KEY>+salt)` |
| DeepL | 网页通道 `Authorization: None`；官方通道 `DeepL-Auth-Key <authKey>` |
| Google | `tk` = `google-translate-sign.js` 的 `sM` 算法；TKK 因 JS 缓存冻结在首次读取，实际使用文件内初始常量（值不在此记录） |
| Bing | 从 `/translator` 页面抓 IG / IID / `params_AbusePreventionHelper`（`key`、`token`、`expirationInterval`）；`now - key > interval/2` 即重抓 |
| Baidu | `sign = md5(appId + utf8(text) + salt + secretKey)`（十六进制小写）；`salt` = 大写带连字符 UUID |
| Tencent | TC3-HMAC-SHA256：规范请求（`POST\n/\n\ncontent-type:application/json; charset=utf-8\nhost:tmt.tencentcloudapi.com\nx-tc-action:texttranslate\n\ncontent-type;host;x-tc-action\n<sha256(body)>`）→ 待签串 `TC3-HMAC-SHA256\n<ts>\n<scope>\n<sha256(canonical)>` → 四段派生 HMAC（首段密钥 = `"TC3"+secretKey`）→ `Authorization: TC3-HMAC-SHA256 Credential=<secretId>/<scope>, SignedHeaders=…, Signature=<hex>` |
| Alibaba | 参数按 key 字典序 → RFC-3986 编码 → `POST&%2F&<编码串>` → **`.nonLossyASCII` 转义非 ASCII** → `HMAC-SHA1(key = secret + "&")` → base64 |
| Volcano | Volcengine 变种 HMAC-SHA256（算法标识为裸的 `HMAC-SHA256`，无 `AWS4-` 前缀）；`credentialScope = "<shortDate>/cn-north-1/translate/request"`；`Authorization` 头逗号后是**两个空格** |
| NiuTrans | 无签名，仅 `apikey` 参数 |
| Caiyun | `x-authorization: token <token>` |
| Doubao | `Authorization: Bearer <key>` |
| OpenAI 兼容（13 个 LLM 服务） | `Authorization: Bearer` **与** `api-key` 同时发送（Azure 兼容）；Claude 例外：`x-api-key` + `anthropic-version: 2023-06-01` |

## 4. 查询类型分流

- 位掩码 `EZQueryTextType`：translation 1 / dictionary 2 / sentence 4。
- **非 AI 服务**：词典分支要求 `shouldQueryDictionary(withLanguage:maxWordCount: 1)`；Youdao 词典只在 en/ja/fr/ko ↔ 中文时触发；Bing 词典只在 English→简体中文且单词 ≤ 20 字符时触发；Google 走 `client=webapp` 的主分支，否则 `gtx`。
- **LLM 服务**：`maxWordCount: 2`；dictionary → `dictMessages`，sentence（且源语言为英语）→ `sentenceMessages`，其余 → `translationMessages`。
- 显式指定的 `queryType` 优先于自动判定（两侧一致）。

## 5. 上游内置凭据清单（只记位置）

| 位置 | 内容 | 处理 |
|---|---|---|
| `Swift/Feature/DefaultAPIKeys/EncryptedSecretKeys.plist` + `APIKey.swift` | 内置服务 key / endpoint（含 BuiltInAI 的 `builtInAIAPIKey`、`builtInAIEndpoint`） | 值见 `01-overview.md` §4；本目录不复述 |
| `YoudaoService+Translate.swift` key-fetch 用 key | 源码字面量 | 不记录 |
| `YoudaoService+Dict.swift` 词典签名 key | 源码字面量 | 不记录 |
| Youdao 请求的 `OUTFOX_SEARCH_USER_ID` cookie | 源码字面量 | 不记录 |
| `BingService.swift` 词典 appid | 源码字面量（公开常量） | 不记录 |
| `NiuTransService.swift` 回落 key | 源码字面量 | 不记录 |
| `CaiyunService.swift` 回落 token | 源码字面量 | 不记录 |
| Google 初始 TKK（`google-translate-sign.js`） | 脚本常量 | 不记录 |

> Windows 移植**不随包分发**以上任何凭据；缺失时统一报 `MissingSecretKey` 并提示在「设置 → 服务」中填写。
