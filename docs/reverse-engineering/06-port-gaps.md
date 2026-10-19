# 移植缺口登记

> **2026-10-18 优化复核**：下文包含上一轮审计的历史代码位置与结论，不能全部视作当前状态。
> 当前实现与测试边界以 [`../windows-validation.md`](../windows-validation.md) 为准。
> 已补：查询窗口隔离、请求 ID、取消语义、监听清理、即时快捷键注册、粘贴翻译、自动朗读、
> OCR 快捷键路由、设置导航、首次显示、位置入口统一。第八轮接入后台鼠标划词和原生图标，
> 更正点击/悬停及自动查询的设置语义，源码对照见 [`../mac-parity.md`](../mac-parity.md)。
> HTTP、有道 OCR 和远程语言校正仍标注为未接入；鼠标图标实机及其余历史条目仍需逐项验收。

> 本文件记录 Windows 移植**已经实现但与上游不一致**、或**声明了却没有生效**的地方。
> 与 `01-overview.md` §6「已知未确定项」的分工：那里记的是**上游行为无法确定**的条目；
> 这里记的是**上游行为清楚、移植没做到**的条目。
>
> 核对基准：`%TEMP%\ezsrc\Easydict-dev`（上游 Swift 源码）。每条都给出上游与移植两侧的
> `文件:行`，便于复核。状态：✅ 已修 · ⬜ 待修 · ⏸ 有意不做（并说明理由）。

---

## 1. 凭证与配置：十个服务无法在 UI 里配置 ✅

最严重的一类 —— 用户在界面上填了密钥，服务仍然用不了。

`settings/ServiceDetail.tsx:273-281` 只渲染**一个**通用 `API Key` 输入框，写入
`services.<identifier>.apiKey`；而下列服务读的是**顶层扁平设置字段**，从不读 `config.apiKey`。
前端对这些字段名的引用数为 0（只出现在 `types.ts` 的类型声明里）：

| 服务 | 后端实际读取（`src/service/*`） | 上游对应位置 |
|---|---|---|
| Baidu | `baidu_app_id` / `baidu_secret_key` | `BaiduService.swift:24-26` |
| Tencent | `tencent_secret_id` / `tencent_secret_key` | `TencentService.swift:48` |
| Alibaba | `ali_access_key_id` / `ali_access_key_secret` | `AliService.swift` |
| Volcano | `volcano_access_key_id` / `volcano_secret_access_key` | `VolcanoService.swift:165-167` |
| Doubao | `doubao_api_key` / `doubao_model` | `DoubaoService.swift:61-69` |
| Caiyun | `caiyun_token` | `CaiyunService.swift:42` |
| NiuTrans | `niu_trans_api_key` | `NiuTransService.swift:26` |
| Youdao | `youdao_web_api_key` | `YoudaoService.swift` |
| DeepL | `deep_l_auth` | `DeepLService.swift` |
| Bing | `bing_cookie` | `BingService.swift` |

后果按严重度：

- Baidu / Tencent / Alibaba / Volcano / Doubao 是 `UserProvided`，UI **会**显示密钥框；用户填完，
  服务仍然报 `Please enter your … API key in Settings → Services.`（`service/mod.rs:214-222`）
  —— 错误信息指向用户刚刚填过的那个框。Tencent 走 `has_private_api_key`（`tencent.rs:129-131`），
  只读扁平字段，所以这个报错是必然的。
- Caiyun / NiuTrans 是 `BuiltIn`，UI 显示「无需填写」，但服务会因缺 key 失败。
- `youdao_web_api_key` 最糟：它不在 `easydict://writeKeyValue` 映射表里（`lib.rs:356-374`），
  而服务自己的报错写着「Set … in Settings → Advanced」，`AdvancedTab.tsx` 里并没有这个字段。
  Youdao 默认启用（`config/mod.rs:386-391`），所以全新安装的 Youdao 卡片必定报错。
  同时深链 `EZYoudaoAPIKey` 映射到 `services.Youdao.apiKey`（`lib.rs:360`），而 `youdao.rs` 从不读它 —— 该深链是空操作。
- Doubao 的模型选择器写 `services.Doubao.model`，而 `doubao.rs:41-59` 只读扁平的 `doubao_model`，
  于是「选了模型」实际不生效，静默沿用默认模型。

**已修**，分三部分：

1. **UI 侧**（`ServiceDetail.tsx`）：按上游做法增加逐服务密钥区，字段名、掩码方式与上游的
   `SecureInputCell` / `InputCell` 一一对应，写入顶层设置字段。Doubao 的模型改为写 `doubaoModel`；
   `deep_l_translate_end_point` / `deep_l_translation` 也一并暴露（上游 `DeepLService`
   同时渲染这两项）。凡是自带密钥区的服务都不再显示通用 API Key / API 端点行 ——
   那两项它们从不读取。另外 `BuiltInAI` / `Polishing` / `Summary` 在本移植没有内置密钥，
   而 Rust 侧要求 `services.<id>.apiKey`，其 `ApiKeyRequirement` 却是 `BuiltIn`（`requiresApiKey` 为 false），
   面板原本显示「使用内置密钥，无需填写。」—— 现改为显示通用密钥框。
2. **深链**（`lib.rs`）：`EZYoudaoAPIKey` 改指 `youdaoWebApiKey`；测试断言每个映射目标
   都是真实存在的设置字段，避免再次出现「写进无人读取的路径」。
3. **后端访问门**（`baidu.rs` / `alibaba.rs` / `volcano.rs` / `doubao.rs`）：这是补齐第 1 步后
   **暴露出来的第二个缺陷**。`prehandle`（`service/mod.rs:213`）用 `needs_user_provided_key() &&
   !has_private_api_key()` 拦截缺密钥的请求，而 `has_private_api_key` 的默认实现读的是
   `config.api_key`。Baidu / Alibaba / Volcano / Doubao 读的是扁平字段，因此**永远无法通过这道门**：
   用户按新面板填好 App ID 与 Secret Key，仍然收到「Please enter your Baidu API key in Settings →
   Services.」——指向面板上根本不存在的框。四个服务各按其上游覆写
   （`BaiduService.swift:53`、`AliService.swift:31`、`VolcanoService.swift:47`、`DoubaoService.swift:50`；
   Tencent 早已覆写）。`registry.rs` 的服务列表「未填写密钥」角标同理改为
   `has_private_api_key()`，否则填对了字段角标也不灭。

**仍未做**：`baidu_service_api_type` / `ali_service_api_type` 保持不可配置 —— 上游 `BaiduApiTranslate.swift:28`
据此在签名 API 与免费 Web API 之间分支，本移植只实现了签名 API 路径，放出来就是个死控件（见 §3）。

---

## 2. 声明了但从不生效的设置

这些设置项在 `src/config/mod.rs` 里有字段、多数还有 UI 开关，但后端没有任何读取点。

| 设置 | 上游行为 | 移植现状 | 状态 |
|---|---|---|---|
| `enable_streaming` | `usesStreamingTransport` 决定流式/非流式传输（`StreamService.swift:224-226`、`BaseOpenAIService.swift:37-40,247-250`） | 原先只被 `ServiceContext::enable_streaming` 读取，而该函数无调用者；四条 LLM 路径恒发 `stream: true`。现已按上游 `nonStreamingTranslate`（`BaseOpenAIService.swift:326-391`）+ `makeNonStreamingChatRequest`（`:399-419`）实现非流式分支：同一请求体发 `stream: false`、**无 Content-Type 门**、仅由状态码判定失败、`{"error":{…}}` 信封优先于原始正文、`choices.first?.message.content` 为空按 `.noResult` 处理，整段答案作为一个 chunk 下发。集成测试断言的是**服务端收到的字节**（开关开→`"stream":true`+SSE 回包，关→`"stream":false`+JSON 回包），并对四条路径分别做过变异验证 | ✅ |
| Doubao 的「温度 / 推理强度 / 自定义提示词」 | `DoubaoService.buildRequestBody`（上游）只发 `model`/`stream`/`input`；该服务的配置面板也只渲染密钥与模型两项 | 三个控件写 `services.Doubao.*`，而 `doubao.rs` 只读 `hide_think_tag` 与扁平 `doubao_model`。现已在 UI 中隐藏（「流式输出」保留 —— `doubao.rs:133` 确实据此分支，属本移植的有意扩展） | ✅ |
| `auto_play_audio` | 查询完成后自动朗读（`EZBaseQueryViewController.m:1913`） | 仅 `commands.rs`/`store` 的测试引用，`tts/mod.rs` 与前端都不读 | ⬜ |
| `language_detect_optimize` | 本地识别不理想时改用百度/谷歌识别（`DetectManager.swift:118-145`） | 零读取；且 `BaiduService::detect_text`（`baidu.rs:40`，上游 `detectText`）已实现却无调用者 | ⬜ |
| `auto_show_query_icon` | 对应上游 `autoSelectText` 的后台监听开关 | 第八轮接入原生钩子与图标，关闭后卸载钩子；托盘和快捷键修改相同设置 | 代码已接入，桌面待验 |
| `click_query` | 隐藏主窗口模式下，开启为点击、关闭为悬停；未隐藏主窗口时也为悬停（`EZWindowManager.m:169/176`），并非弹菜单 | 第八轮接入原生图标激活条件和设置推导回归 | 代码已接入，桌面待验 |
| `auto_show_query_icon_excluded_language` / `auto_show_query_icon_min_text_length` | `EventMonitor.swift:560/566` | 第八轮按字素长度及本地识别语言过滤，最小长度限制 0..50 | 代码已接入，桌面待验 |
| `auto_query_selected_text` | `EZWindowManager.m:303/321` 决定结果窗打开后是否查询 | 第八轮鼠标及快捷键均遵守；关闭只填原文，文本变化自动查询不绕过预览；前端回归通过 | 代码已接入，桌面待验 |
| `auto_query_pasted_text` | 粘贴即查询（`EZBaseQueryViewController.m:1494`） | 只有开关，没有任何 paste 处理 | ⬜ |
| `enable_http_server` / `http_port` | 真实 Vapor 服务器（`VaporServer.swift:52-53,90`） | 完全没有 HTTP 服务器 | ⏸ 见 §5 |
| `enable_youdao_ocr` | 切换 OCR 引擎（`DetectManager.swift`） | `ocr/mod.rs` 只有 `Windows.Media.Ocr`，没有可切换的对象 | ⏸ 见 §5 |
| `show_ocr_menu_items` | `MenuItemView.m` 控制 OCR 菜单项 | `tray.rs:62-100` 永远显示 OCR 项 | ⬜ |
| `enable_beta_feature` | 古汉语检测（`AppleLanguageDetector.swift:453`） | 已接到 `LanguageDetector`，但 `detect/mod.rs:27` 的 `beta` 字段在模块内无读取点 —— 最后一跳断了 | ⬜ |
| `allow_analytics` / `allow_crash_log` | 遥测与崩溃上报 | 移植没有任何遥测代码，两个开关纯装饰 | ⏸ 见 §5 |
| `mouse_select_translate_window_type` | `EZWindowManager.m:199` 的图标激活目标 | 第八轮向指定窗口发送捕获选区，通用页可配置 | 代码已接入，桌面待验 |
| `fixed_window_position` / `mini_window_position` | `EZWindowManager.m:456/460`、`EZLayoutManager.m:227/239` | 原先零读取；浮动窗位置实际由前端传 `position`，而前端三处调用**都不传**，于是恒为 `Right` —— Mini 窗的 `Mouse` 默认值永远不会生效。现已在 `show_window`（`commands.rs:868`）中按窗口 label 回退到这两个设置。**且这两个设置原先没有 UI**（只出现在 `types.ts` 的类型声明里），等于「后端会读、但没人能设」；现补到 设置 → 高级 → 窗口（对应上游 `AdvancedTab.swift:321/335`，选项文案取自上游 zh-Hans） | ✅ |
| `shortcut_select_translate_window_type` | `FavoritesTab.swift:320` | 零读取 | ⬜ |
| `auto_select_all_text_field_text` | `ActionManager.swift:58` | 零读取（连 UI 都没有） | ⬜ |
| `enable_remove_books_excerpt_info` / `enable_ocr_text_normalization` | `String+HandleInputText.swift:123`、`OCRSectionMerger.swift:166` | 零读取 | ⬜（Books 那条属 macOS 专有，可不做） |
| `enable_force_get_selected_text` | `SelectionWorkflow.swift:182/327` 的「强制读取选区」 | 第六轮接入复制回退，默认开启，设置 → 通用可关闭。第七轮仅在 UIA 安全检查通过、焦点稳定且剪贴板所有格式可完整备份时模拟 `Ctrl+C`；恢复前校验版本。真实多格式/焦点回归仍待验收 | ✅ |
| `hide_menu_bar_icon` / `selected_menu_bar_icon` / `show_*_quick_link` / `show_quick_action_button` | `EasydictApp.swift`、`EZTitlebar.m:108/322/327/332` | 零读取；`TitleBar.tsx` 根本不渲染这些按钮，同名的三个功能只作为应用内快捷键存在（`QueryWindow.tsx:913-915`），不受这些开关约束 | ⬜ |
| `is_screenshot_tip_layer_hidden` | `ScreenshotState.swift` | 零读取，没有截图浮层 | ⏸ |
| `first_launch` | `ShortcutManager.swift:22-23` | 零读取；移植以 `default_shortcuts()` 作默认值，所以无实际影响 | ⏸ |
| `query_history` / `favorites`（两个 `Vec<QueryRecord>`） | 上游 `QueryRecordManager` | 从未读写；历史/收藏实际在 SQLite（`store/mod.rs:37-57`），这两个字段只是永久序列化进 `settings.json` 的死重量 | ⬜ |
| `main_window_width` / `main_window_height` / `font_size_ratio_index` | — | 零读取；`font_size_ratio_index` 与真正生效的 `font_size_option_index` 重复 | ⬜ |

---

## 3. 后端读取、但 UI 无法设置

| 字段 | 情况 | 状态 |
|---|---|---|
| `doubao_model`（`config/mod.rs:357` → `doubao.rs:41-59`） | 模型选择器原先写 `services.Doubao.model`，Doubao 不读，静默沿用默认模型。现改为直接编辑该字段 | ✅ |
| `deep_l_translate_end_point` / `deep_l_translation`（`deepl.rs:81/263`） | 有读取方，原先只能手改 `settings.json`。上游 `DeepLService` 同时渲染这两项（文本输入 + 优先级选择器），现已在认证区暴露 | ✅ |
| `deep_l_web_app_version_cache`（`config/mod.rs:323`） | `deepl.rs:200-202` 会*读*以恢复缓存，但没有任何地方*写*（上游在 `DeepLWebAppVersionProvider.swift:129` 持久化），所以缓存永远是空的，只在进程内有效 —— 每次启动都要重新查一次 App Store 版本 | ⬜ |
| `baidu_service_api_type` / `ali_service_api_type` | 零读取。上游 `BaiduApiTranslate.swift:28` 据此在签名 API 与免费 Web API 之间分支 —— 从 macOS 导出的设置文件可以选一个移植会静默忽略的 API 类型。本移植只实现了签名 API 路径，放出来是死控件 | ⏸ 有意不做（见 §1 末） |

---

## 4. 死代码 / 不可达路径

- **10 个已注册但前端从不调用的命令**：`reset_settings`、`get_service_config`、`service_order`、
  `query_single`、`retry_query`、`ocr_image`、`detect_language`、`system_languages`、
  `search_history`、`set_window_size`。其中重试已在前端重实现（`useQuery.ts:106-110` 直接重跑
  `api.query`）；`query_single` 的文档注释说「用于单服务重试按钮」是**不实**的 ——
  校验走的是 `query::run_direct`（`commands.rs:389`）。
- **`QueryRequest.window_type`**（`commands.rs:53`）反序列化后从不被读取，`build_query_model`
  从不赋值 `model.window_type`，因此它恒为 `Main`（`model/mod.rs:599`）；连带
  `impl From<WindowTypeDto> for WindowType`（`commands.rs:65-73`）是死代码，前端传的
  `windowType`（`QueryWindow.tsx:271/733`）无处可去。
- **`window::Placement::{Mouse, Center, Former}`**（`window.rs:13-18`）：`window::position`
  只被 `show_window`（`commands.rs:873`）调用，而没有任何前端调用方传 `position`，
  所以实际只有 `Right` 可达。
- **`set_app_language_is_chinese`**（`model/language.rs:899`）从未被调用，而该全局量在
  `language.rs:649`、`youdao.rs:444/453` 被读取 —— 也就是说「应用语言为中文」这条分支永久为 false。
- **`polishAndReplace` / `translateAndReplace`** 都跑普通翻译路径（`lib.rs:261-262`，代码注释已承认）；
  `screenshotOCR` / `silentScreenshotOCR` 派发同一个 action id（`lib.rs:264`），「静默」无法区分。
  而 `SHORTCUT_ACTIONS`（`config/mod.rs:599-629`）把两者都当作可用功能对外宣传。
- 另有约 30 个 `pub fn` 无生产调用点（仅测试引用），例如 `Settings::secret`、
  `util::collapse_whitespace`、`ensure_word_result`、`BingService::text_to_audio`、
  三个 LLM 服务里与 `registry.rs:229` 重复的 `fetch_remote_model_ids` 等。

---

## 5. 有意不做（平台能力差异）

- **HTTP 服务器**（`enable_http_server` / `http_port`）：上游依赖 Vapor，移植没有任何 HTTP
  服务端栈。要么补一个最小 HTTP 端点，要么把这两个开关从 UI 移除 —— 目前 UI 在说谎。
- **Youdao OCR**（`enable_youdao_ocr`）：没有第二个 OCR 引擎可切。
- **遥测**（`allow_analytics` / `allow_crash_log`）：移植不上报任何数据，开关保持关闭语义即可，
  但应在 UI 注明「本移植不上报数据」。
- **Apple Dictionary / Apple 系统翻译**：见 `01-overview.md` §6 第 6、7 条。
- **Codex/Claude Code/GitHub Copilot 三个 CLI 服务**：`registry.rs:43-77` 有意未实现，
  原生 `ServiceConfig.cli_effort` 因此无读取点，与范围一致，不算回归。
- **逐窗口配置体系（`windowType`）**：上游 `EZWindowType` 是一整套**按窗口分别存储**的配置 ——
  窗口位置（`Configuration+Extension.swift:15-20`、`Defaults.Keys+Extension.swift:260`）、
  智能查询模式（`:51-66`）、是否显示输入框（`:74-75`）、以及 `WindowConfigurationKey.swift:18`
  生成的键族。目前窗口查询隔离、请求中的 `windowType`、鼠标/快捷键目标窗口与浮窗位置已经接通，
  不能再称这些字段零读取；但完整的逐窗口独立设置键族仍未实现。
  `fixed_window_position` / `mini_window_position` 已在共享窗口显示入口读取（见 §2），
  不等于整套逐窗口配置移植完成。

---

## 6. 已核对无误（可勾掉）

`query_from_language`、`query_to_language`、`first_language`、`second_language`、
`appearance_type`、`font_size_option_index`、`pronunciation_accent`、`default_tts_service_type`、
`prefer_youdao_tts_for_english_word`、`hide_main_window`、`max_window_height_percentage`、
`launch_at_startup`、`auto_query_ocr_text`、`auto_copy_*`、`pin_window_when_displayed`、
`keep_prev_result_when_selected_text_is_empty`、`services`、`service_order`、`shortcuts`、
以及 §1 表格里全部扁平凭证字段（**后端侧**读取正常）——
共 84 个 `Settings` 字段 + 19 个 `ServiceConfig` 字段逐一核对，其余均属上表。
