# Easydict 逆向分析报告 — Windows 移植架构

> 目标：说明 Windows 版**实际实现**的分层、查询编排、存储设计与平台替换，并列出与上游的差距。
>
> 代码根：`C:\Users\12064\Desktop\win翻译应用\app\`（Rust crate `app` / lib `app_lib`，Tauri v2，产品名 `Easydict`，标识符 `com.easydict.windows`）。
>
> 分析日期：2026-10-16

---

## 1. 分层

```
┌──────────────────────────────────────────────────────────────────────┐
│ React 前端（app/src）                                                  │
│   App.tsx ── 按 window label 分流：QueryWindow / SettingsApp / OcrApp   │
│   hooks/: useSettings(全局设置缓存+订阅) useQuery(结果表)               │
│           useLanguages useAppearance                                  │
│   api.ts ── 每个 Tauri 命令一个包装函数（invoke）                       │
└──────────────────────────▲───────────────────────────────────────────┘
            命令调用（invoke）│  事件（emit）：query-update / query-finished /
                              │  action / ocr-result
┌──────────────────────────┴───────────────────────────────────────────┐
│ Tauri 命令面（commands.rs，48 个 #[tauri::command]）                    │
│   薄适配层：JSON ⇄ 领域类型（model/config），其余交给 AppState          │
└──────────────────────────▲───────────────────────────────────────────┘
                           │
┌──────────────────────────┴───────────────────────────────────────────┐
│ AppState（state.rs）                                                   │
│   settings: RwLock<Settings>    handles: RwLock<Vec<Arc<ServiceHandle>>>│
│   run: Mutex<Option<QueryRun>>  store / tts / ocr / detector           │
└──┬────────────┬──────────────┬───────────────┬──────────────┬────────┘
   │            │              │               │              │
 query/       service/       store/          ocr/ tts/      config/ detect/
 编排引擎      服务实现        持久化           平台能力        配置 / 语言检测
```

**移植原则**：上游的 Objective-C 运行时对象（`EZQueryService` 实例、`Defaults` 观察者）换成 Rust 的所有权结构；服务通过 trait 对象（`Arc<dyn QueryService>`）保存，注册表由 `match` 构造而非类注册。

---

## 2. 模块表（`app/src-tauri/src/lib.rs`）

`lib.rs` 头部注释给出与上游目录的对应关系：

| 本仓库 | 上游 |
|---|---|
| `model` | `Model/`、`Service/Model/` |
| `config` | `Feature/Configuration/` |
| `service` | `Service/` |
| `detect` | `Service/Detect/` + `NLLanguageRecognizer` |
| `query` | `objc/Service/Model/EZQueryService.*` |
| `store` | `Model/Storage/` |
| `ocr` | Vision（`VNRecognizeTextRequest`） |
| `tts` | `NSSpeechSynthesizer` + `AVPlayer` |
| `platform` | `Accessibility` + `CGWindowListCreateImage` |
| `window` | `EZWindowManager` |

模块声明（16 个）：

```rust
pub mod commands;  pub mod config;   pub mod detect;  pub mod model;
pub mod net;       pub mod ocr;      pub mod platform; pub mod query;
pub mod service;   pub mod state;    pub mod store;   pub mod system;
pub mod tray;      pub mod tts;      pub mod util;    pub mod window;
```

其中 `ocr`、`tts`、`platform` 是 `#![cfg(windows)]`；`system.rs` 为非 Windows 平台留了桩实现（`send_paste_keystroke` 返回错误、`system_languages()` 读 `$LANG`），其余模块跨平台可编译，便于测试。

**Tauri 插件**（`lib.rs` 注册顺序）：`single-instance`（最先，桌面端）→ `log` → `opener` → `shell` → `dialog` → `fs` → `os` → `process` → `notification` → `autostart`（`MacosLauncher::LaunchAgent`）→ `clipboard-manager` → `store` → `global-shortcut`。`tauri-plugin-store` 虽注册，但**应用的持久化不使用它**（见 §5）。

---

## 3. 命令面（48 个）

| 分组 | 命令 |
|---|---|
| 设置（5） | `get_settings` `update_settings`(JSON 深合并) `reset_settings` `export_settings` `import_settings` |
| 服务（6） | `list_services` `get_service_config` `update_service_config` `validate_service` `fetch_service_models` `service_languages` |
| 查询（5） | `query` `stop_query` `retry_query` `query_single` `translate_text` |
| 剪贴板 / 文本（5） | `clipboard_text` `set_clipboard_text` `selected_text` `replace_selected_text` `translate_text`（见上） |
| OCR（3） | `ocr_image`（base64）`screenshot_ocr` `pasteboard_ocr` |
| 语音（2） | `play_tts` `stop_tts` |
| 语言（4） | `detect_language` `supported_languages` `system_languages` `toggle_languages` |
| 历史 / 收藏（9） | `history` `search_history` `delete_history` `clear_history` `history_count` `favorites` `add_favorite` `remove_favorite` `is_favorite` |
| 窗口 / 应用（10） | `show_window` `hide_window` `pin_window` `set_window_size` `fit_window_to_content` `open_link` `app_info` `service_order` `set_service_order` `show_service_result` |

事件常量（`commands.rs::events`）：

| 常量 | 名称 | 载荷 |
|---|---|---|
| `QUERY_UPDATE` | `query-update` | `QueryEventPayload { identifier, serviceType, result }`（camelCase） |
| `QUERY_FINISHED` | `query-finished` | `()` |
| `ACTION` | `action` | `ActionPayload { action, text? }` |
| `OCR_RESULT` | `ocr-result` | `OcrOutput`（**已声明但当前没有任何发送方**） |

---

## 4. 查询编排

### 4.1 每个服务一份状态

`query/mod.rs`：

```rust
pub struct ServiceState {
    generation: Mutex<u64>,                    // 单调递增的「结果代」
    result: Mutex<Option<QueryResult>>,
    stop: Mutex<Option<CancellationSource>>,   // queryModel.setStop(_:serviceType:)
}
pub struct ServiceHandle { service: Arc<dyn QueryService>, state: Arc<ServiceState> }
```

- `begin_new_result(query)`：`generation = generation.wrapping_add(1)`，生成新 `QueryResult`，按 `ActionType`（`AutoSelectQuery` / `ShortcutQuery` / `InvokeQuery`）决定是否显示「替换」按钮，并立即写入 `state.result`。
- **陈旧判定**：流式循环内与结束后都检查 `state.generation() != generation`，不等则**静默丢弃**（对应上游的 `resultGeneration` + `updateResultLock`）。
- **停止块**：每次请求注册一个新的 `CancellationSource`（`watch::channel(false)` + `send(true)`），`stop_all()` 会逐个取消。`Cancellation` 可克隆，`cancelled()` 会等真正的 `true`（`watch::changed()` 只在**新值**时返回，所以用循环而非单次等待）。

### 4.2 扇出与事件

```rust
QueryRun::start(handles, model, tx)
  └─ 每个 handle 一个 tokio::spawn(run_one(..))
       ├─ 先发一次 Update{ is_showing:true, is_loading:true }
       ├─ tokio::select!{ biased; 外部取消 | 自身取消 | update_rx.recv() | translate 完成 }
       ├─ 循环内把 ServiceUpdate::Chunk 追加进结果并**立即发 Update**
       └─ 结束后折叠最终结果（保留已流出的文本、繁简转换、置 Finished）
  └─ 全部结束后发 QueryEvent::AllFinished
```

```rust
pub enum ServiceUpdate { Chunk(String), Validation(String) }   // 服务 → 编排
pub enum QueryEvent { Update{ identifier, service_type, result }, AllFinished }  // 编排 → 命令层
```

`commands::query` 建 `mpsc::unbounded_channel::<QueryEvent>()`，交给 `AppState::start_query`，随后 spawn 一个循环把每个 `QueryEvent` 翻成 `app.emit("query-update", …)`；只有收到 `AllFinished` 才写历史（`state.record_history`），避免中途取消也入库。

`AppState::start_query` 会**先** `stop_query()`，因此新查询天然取消上一次（`QueryRun::stop()` + `query::stop_all()`）。

**直连路径** `run_direct(handle, model)`：不走事件通道，供 `validate_service`、`query_single`、`translate_text` 使用（`prehandle` → `translate` → 抽干已排队的 chunk）。

**语言与查询类型路由**：`commands::build_query_model` 用 `state.detect_language()` 补 Auto 源语言，用第一/第二语言决定默认目标语言，用 `util::analysis::query_type(text, from, 2)`（与上游同样的 maxWordCount = 2）决定 `QueryTextType`。

### 4.3 服务层契约

`service/mod.rs` 的 `QueryService` trait：`service_type()` / `name()` / `identifier()` / `ctx()` / `language_code()` / `supported_languages()` / `supported_query_type()` / `is_stream()` / `has_private_api_key()` / `word_link()` / `link()` / `auto_convert_traditional_chinese()` / `translate(query, updates)`。

`prehandle()` 的顺序（与上游 `prehandleQueryTextOutcome` 一致，但**顺序**与上游略有差别，见下）：

1. `language_code(from|to)` 为 `None` → `QueryError::unsupported_language`；
2. `auto_convert_traditional_chinese()` 且双方都是中文 → **本地转换并直接返回**（不发请求）；
3. `apiKeyRequirement` 需要用户密钥而 key 为空 → `MissingSecretKey`。

> **差异说明**：上游把「繁简转换」放在最前、把「不支持语言」放在其后，并把「免费额度」检查放在最后；移植的顺序是「不支持语言 → 繁简转换 → 缺密钥」，且**没有实现免费额度计数**（`totalFreeQueryCharacterCount` 未移植）。功能上等价于常见场景，但错误优先级不同。

`service/registry.rs` 是 `QueryServiceFactory` 的替代：`service_types()` = `ServiceType::all()` 过滤 `is_available_on_windows() && is_implemented()`（24 项，见 §9）；`build_all()` 按设置构造全部 `ServiceHandle`；`fetch_remote_models()` 对 Ollama / Claude / GitHub 走特例，其余按 `{base}/models` 推导。

---

## 5. 设置与历史存储

### 5.1 上游的做法

- **全部**配置都在 `UserDefaults`：`Defaults.Keys+Extension.swift` 定义键名（如 `EZConfiguration_kFromKey`、`EZConfiguration_kQueryHistory`），`MyConfiguration` 是门面。
- 查询历史与收藏**不是 Core Data**——它们也只是两个 `UserDefaults` 数组（`EZConfiguration_kQueryHistory` / `EZConfiguration_kFavorites`），由 `QueryRecordManager` 整体重写：收藏按 `query_text` 去重、策略 `.skipIfExists`；历史策略 `.moveToFront`、上限 **1000**；插入时 `insert(at: 0)`，超出用 `prefix(maxCount)` 截断。
- 导出/导入靠 `easydict://writeKeyValue?...` 等 URL Scheme 命令。

### 5.2 移植的做法（`store/mod.rs`）

| 项 | 值 |
|---|---|
| 设置文件 | `settings.json`（**美化 JSON**，原子写：临时文件 → `sync_all` → `rename`） |
| 数据库 | `Easydict.db`（SQLite，`rusqlite` bundled；`busy_timeout` 5 s；`user_version` = `SCHEMA_VERSION = 1`） |
| 数据目录 | `app.path().app_data_dir()`，取不到回落 `"."` |
| 表 | `history` 与 `favorites` 结构相同：`(id INTEGER PK AUTOINCREMENT, query_text, from_language, to_language, timestamp, translated_text)` |
| 索引 | `idx_history_order`、`idx_favorites_order`（`timestamp DESC, id DESC`）、`idx_favorites_text` |
| 历史上限 | `MAX_HISTORY = 1000`（对应 `QueryRecordManager.maxHistoryCount`），每次写入后 `trim_history` |
| 语言存储 | 上游 `Language` 原始字符串；未知值读回 `Language::Unsupported` |

行为对照：

- 历史去重：与**最新一条**同文本 + 同语言对时只更新时间戳，不新增（等价 `.moveToFront`）；空白文本不入库。
- 搜索用 `instr(lower(query_text), lower(?1)) > 0`，因此 `%` / `_` 是字面量。
- 删除按「文本 + 两个语言 + 时间戳」匹配（`QueryRecord` 不带 id）。
- 收藏以 `query_text` 为键，重复添加是 no-op。
- 设置读取：文件缺失 / 不可读 / 无法解析时**返回默认值且不覆盖原文件**。
- 设置键名保留上游命名（`serde(rename_all = "camelCase")`，每个字段的文档注释引用上游 `Defaults` 键），因此**上游导出与移植导出可以互相读取**；`Settings` 里还保留 `query_history` / `favorites` / `main_window_width` / `main_window_height` / `font_size_ratio_index` 等**兼容字段**（真正的历史/收藏在 SQLite 表里）。
- 深链 `easydict://writeKeyValue?EZ...=` 由 `lib.rs::handle_deep_link` + `upstream_key_to_field` 映射到设置字段（`EZDeepLAuthKey → deepLAuth`、`EZYoudaoAPIKey → youdaoWebApiKey`、`EZBingCookieKey → bingCookie`、`EZNiuTransAPIKey`、`EZCaiyunToken`、`EZTencentSecretId/Key`、`EZAliAccessKeyId/Secret`、`EZBaiduAppId/SecretKey`、`EZVolcanoAccessKeyID/SecretAccessKey`、`EZDoubaoAPIKey`），再走 `merge_settings`；每个映射目标都由 `lib.rs` 的测试断言是真实设置字段。

**未使用的设置字段**（保留但当前不产生行为）：`pin_window_when_displayed`、`enable_http_server`、`hide_menu_bar_icon`、`allow_analytics`、`show_ocr_menu_items`、`enable_youdao_ocr`、`auto_copy_ocr_text`、`first_launch`、`show_google_quick_link` / `show_eudic_quick_link` / `show_apple_dictionary_quick_link` / `show_quick_action_button`（标题栏快捷链接按钮未实现；保留字段是为了让 macOS 导出的设置往返不丢键）。

---

## 6. 进程与窗口生命周期

- **单实例**（`tauri-plugin-single-instance`）：第二次启动时先解析 `argv` 里的 `easydict://` 链接交给 `dispatch_deep_links`（本身跳过 `argv[0]`），没有链接才显示并聚焦主窗口。支持 `query?text=`（入队 `inputText` 并显示 `fixed`）、`ocr`（显示 OCR 窗口）、`writeKeyValue`（写设置）；日志只记路由不记查询串。`percent_decode` 处理 `%XX` 与 `+`。
- **URL 协议**（第十二轮，`platform/protocol.rs`）：程序每次启动（Release）把 `HKCU\Software\Classes\easydict` 写成 URL Protocol，命令为 `"<exe>" "%1"`，对应 macOS 的 `CFBundleURLTypes`；程序未在运行时 Windows 直接以该命令启动它，链接作为 `argv[1]` 到达，`setup` 在设置加载后解析自身命令行（冷启动），已运行时则走上面的单实例转发。`app.exe --unregister-url-scheme`（退出码 0/1/2，在事件循环前处理）与安装器 `installer-hooks.nsh` 的 `NSIS_HOOK_PREUNINSTALL`（只删指向 `$INSTDIR\app.exe` 的那条）负责回收。
- **窗口**：关闭请求一律转成隐藏；`mini` / `fixed` 启动即置顶（`alwaysOnTop:true`）；`pin_window` 调用 `set_always_on_top`。
- **托盘**：唯一能让用户回到隐藏窗口的入口，因此安装失败只记日志，并在启动时**改为显示主窗口**，避免出现「无窗口、无托盘」的不可达进程。
- **全局快捷键**：`register_shortcuts` 先注销全部再用 `settings.shortcuts` 注册 `global: true` 的动作；注册失败仅告警。`on_global_shortcut` 把按键映射回动作 id：`inputTranslate`/`showMiniWindow`/`showOCRWindow` 直接显示窗口，其余派发 `action` 事件；`screenshotOCR` 与 `silentScreenshotOCR` 都派发 `"screenshotOCR"`；事件优先发给 `[fixed, mini, main, ocr]` 中**已聚焦或可见**的第一个窗口，否则广播。

---

## 7. macOS → Windows 替换表

| # | 能力 | 上游符号 / 框架 | Windows 实现 | 保真度损失 |
|---|---|---|---|---|
| 1 | OCR | Vision `VNRecognizeTextRequest`（`AppleOCREngine`） | `Windows.Media.Ocr.OcrEngine`：自动路径 `TryCreateFromUserProfileLanguages`，指定语言 `TryCreateFromLanguage`，`RecognizeAsync`；PNG 解码用 `Graphics.Imaging.BitmapDecoder` + `Convert(Bgra8)` | **无置信度**（上游有 `VNConfidence`，移植在 `ocr/mod.rs` 显式说明）；要求系统装有对应 OCR 语言包，否则回落第一个已装识别器或报 `NoLanguagePack`；`MaxImageDimension()` 超限即报错；COM 需按任务 `CoInitializeEx(MTA)`，枚举语言另起线程以免污染 STA |
| 2 | 离线语音 | `NSSpeechSynthesizer`（`EZServiceTypeApple`） | `Windows.Media.SpeechSynthesis.SpeechSynthesizer`（`SynthesizeTextToStreamAsync` + `AllVoices`） | 语音集合、语速/音高控制不同；`stopSpeaking` 无对应（用 `MediaPlayer::Pause()`）；引擎跑在专用线程 `easydict-tts` 上并驱动 `SpeechCommand::{PlayUrl, Speak, Stop}` |
| 3 | 音频播放 | `AVPlayer`（`EZAudioPlayer`，含 `audioCacheDirectory`） | `Windows.Media.Playback.MediaPlayer`（`MediaSource::CreateFromUri`） | 无音频缓存目录；上游的播放/停止状态机被简化 |
| 4 | 屏幕截图 | `CGWindowListCreateImage` / `CGDisplayCreateImage` | GDI `BitBlt`（`SRCCOPY ｜ CAPTUREBLT`）+ `CreateCompatibleDC/Bitmap` + `GetDIBits`（负 `biHeight` 取自上而下），BGRA→RGBA 并把 alpha 置 255 | 需要处理虚拟屏（`SM_XVIRTUALSCREEN` 等）；无法直接抓单个窗口；`CAPTUREBLT` 对硬件叠加层（部分全屏视频）可能抓不到 |
| 5 | 取选中文本 | Accessibility API（`AXUIElement`、`AXIsProcessTrustedWithOptions`）+ 合成 `⌘C` 回退 | UI Automation：`CoCreateInstance(&CUIAutomation)` → `GetFocusedElement` → 沿 `ControlViewWalker` 向上**最多 8 层**，依次尝试 `IUIAutomationTextPattern`（拼接多个 selection）与 `IUIAutomationValuePattern`，超时 2 s | 少数自绘控件没有 Text/Value pattern（返回 `NoSelection`）；无 AppleScript / 菜单栏复制回退；需要目标进程允许 UIA |
| 6 | 发送按键 | `CGEvent.post(tap:)` / `KeySender.selectAll()` | `SendInput`（`INPUT_KEYBOARD`，Ctrl+V；含 `VK_CONTROL` 的按下/抬起） | 受 UIPI 限制：对管理员权限的前台窗口无效；无键盘布局级的事件重放 |
| 7 | 语言识别 | `NLLanguageRecognizer` + `AppleLanguageDetector` 纠正层 | `src/detect/`：**Unicode 脚本分类器**（Han/假名/谚文/西里尔/阿拉伯/希伯来/泰/希腊/天城文/拉丁 + 24 种拉丁语言词表、变音符判别）产出概率字典，其后的**全部上游纠正流程逐字保留** | 概率分布与 Apple 模型不同；拉丁语系短文本靠函数词与变音符启发式；移植把中文分词近似为 `chinese_word_count() -> 1` |
| 8 | 菜单栏 | `MenuBarExtra` + `NSStatusItem` | Tauri 托盘图标 + 菜单 | check item 不能原地更新（每次重建菜单）；无图标样式选项；托盘是唯一入口 |
| 9 | 开机自启 | `LaunchAtLogin` / `SMLoginItemSetEnabled` | `tauri-plugin-autostart` | 注册表 Run 项 vs 登录项；`launch_at_startup` 在设置变更时同步到插件，并在启动时与注册表实际状态核对后纠正（`commands::sync_launch_at_startup`），使设置项不会与系统状态不一致 |
| 10 | 应用更新 | Sparkle（`updater.checkForUpdates()`） | **未实现** | 需手动更新 |
| 11 | URL Scheme | `NSAppleEventManager` + `kAEGetURL` + JLRoutes | 第十二轮：启动时写 `HKCU\Software\Classes\easydict`（URL Protocol），冷启动解析自身命令行、已运行时由 `tauri-plugin-single-instance` 转发 `argv`，两者共用 `dispatch_deep_links` | `easydict://readValueOfKey`、`resetUserDefaultsData`、`saveUserDefaultsDataToDownloadFolder` 未实现；浏览器点击唤起与安装/卸载清理未验收 |
| 12 | 全局快捷键 | Magnet `HotKey`（Carbon `RegisterEventHotKey`） | `tauri-plugin-global-shortcut` | 不支持「双击修饰键」；无系统符号热键冲突检测；只有注册成功/失败两种反馈 |
| 13 | 窗口定位 | `EZWindowManager` + `EZShowWindowPosition`（按最后点击的屏幕） | `window.rs::position`（主屏 + 逻辑像素夹取） | `.former` 交给 Tauri；不做「跟着鼠标所在屏幕」的多屏跟随 |
| 14 | 繁简转换 | `String.applyingTransform("Hans-Hant"/"Hant-Hans")` | `service/languages.rs` 的 `S2T` 字表 + 一对多特例表 | 字表非穷尽，罕见异体字保持不变 |
| 15 | 划词取词弹窗图标 | `EZPopButtonWindow`（屏幕保护层级） | **无** | 未实现，见 `04-ui-spec.md` §10 |
| 16 | 系统翻译面板 | macOS 15 `Translation` 框架 + AppleScript | **无** | `Apple` 服务不可用 |
| 17 | 本地词典 | `DictionaryServices.framework`（AppleDictionary）、`.mdx/.mdd`（MDict） | **无** | 两项均未实现 |
| 18 | 崩溃与统计 | `AnalyticsService` | 未实现（设置项保留） | 无上报 |

---

## 8. 服务注册表与实现清单

`registry::is_implemented()` 为真的 **24 项**：
`Youdao, DeepL, Google, Baidu, Bing, Volcano, NiuTrans, Caiyun, Tencent, Alibaba, Doubao, OpenAI, CustomOpenAI, BuiltInAI, Gemini, Claude, DeepSeek, Groq, Zhipu, MiniMax, GitHub, Ollama, Polishing, Summary`。

`ServiceType::all()` 仍返回 **31 项**（含不可用者），`descriptors()` 为每一项输出一行并标 `available: false`，保证旧设置文件往返无损；`ServiceType::is_available_on_windows()` 是唯一的裁决点。

---

## 9. 已知限制与未实现项

### 9.1 未移植的服务

| 服务 | `ServiceType` | 上游形态 | 未实现原因 |
|---|---|---|---|
| Claude Code CLI | `ClaudeCode` | 调用本机 `claude` 子进程，伪流式 | 依赖本机 CLI 的登录态与交互协议；Windows 无等价安装/凭证模型 |
| Codex CLI | `CodexCLI` | 调用本机 `codex`，托管模式从外部 catalog 读模型 | 同上；且托管模型清单本身在上游就是运行时加载（`01-overview.md` §6 第 4 项未确定） |
| GitHub Copilot CLI | `GitHubCopilot` | 调用本机 `gh copilot` | 同上 |
| Apple 系统翻译 | `Apple` | `Translation` 框架 / AppleScript | Windows 无系统翻译 API |
| Apple Dictionary | `AppleDictionary` | 私有 `DictionaryServices.framework` | Windows 无对应框架 |
| MDict | `MDict` | 本地 `.mdx/.mdd` 解析 + WebView 渲染 | 未移植（LZO 解压状态机在上游也未完全确定） |
| SystemSpeech（作为查询服务） | `SystemSpeech` | 上游以 `NSSpeechSynthesizer` 提供系统朗读 | 移植只在 TTS 合成路径使用系统语音，**不**作为查询/翻译服务出现 |

`ClaudeCode` / `CodexCLI` / `GitHubCopilot` 的 `apiKeyRequirement` 是 `.agentCLI`，在移植里仍是「声明但不构造」的状态；前端在「不可用服务」分组里给出中文说明（「Agent CLI 服务在 Windows 版中尚未实现。」）。

### 9.2 功能缺口（同一服务内的次要能力）

- **Youdao**：内置签名 key 不分发 → 未填写时不可用（见 `02-services.md` §0.1）。
- **DeepL**：`DeepLWebTranslationCancellation` 无对应，取消靠丢弃 future；额外加了 128 KiB 请求体安全网（上游没有）。
- **Bing**：抓取到的 host/IG/IID/token 只存在内存里，进程重启后重新抓取（上游持久化到 `UserDefaults`）。
- **Bing TTS 的英文 RPC 路径**：未实现，播放回落到系统合成器。
- **Google**：英文 TTS 的 `batchexecute` RPC 未实现（其余 TTS URL 已复刻）。
- **内置 AI / 润色 / 总结**：不分发上游内置凭据，需用户自备 key 与端点。
- **免费额度计数**：`totalFreeQueryCharacterCount` 未移植。
- **HTTP 服务**：设置项存在，服务端未实现。
- **Anki Connect、词汇笔记本、Eudic / Apple Dictionary 快捷链接、菜单栏图标样式、截图提示层、有道 OCR**：未实现。
- **自动更新**：未实现。
- **界面语言**：只有简体中文。

### 9.3 已知行为差异（有意为之）

| 差异 | 位置 | 理由 |
|---|---|---|
| Tencent 请求体只序列化一次（哈希与发送同一份字节） | `service/tencent.rs` | 上游两次编码可能导致哈希与请求体不一致，属修正 |
| 流式节流只发送增量后缀 | `service/llm/mod.rs` | 移植的 chunk 语义是追加 |
| 查询窗口自动适配为「测量 + 命令式缩放」，无 AppKit 动画 | `window.rs` + `QueryWindow.tsx` | 平台差异；`animate` 参数被忽略 |
| 全局快捷键只做注册结果检查，不做系统冲突检测 | `lib.rs::register_shortcuts` | Windows 无公开的符号热键枚举 |
| `prehandle` 的三步顺序与上游略有不同 | `service/mod.rs` | 见 §4.3 |

### 9.4 未确定项（承接 `01-overview.md` §6）

1. `dictionary-result.html` 的完整脚本（上游 17 KB）——影响 AppleDictionary 的渲染保真，移植未实现该服务。
2. `String.handlingInputText()` 的完整归一化规则——移植只移植了 `util/analysis.rs` 中被服务实际调用的判定。
3. MDict 的 LZO 解压器状态机——移植未实现。
4. Codex CLI 托管模式的内置模型清单——移植未实现。
5. `AppPathManager.audioCacheDirectory` 的确切路径——移植没有音频缓存。
6. Apple Dictionary / Apple 系统翻译在 Windows 上的替代方案（当前选择：不提供）。
7. 上游 `ServiceUsageStatus` 与「免费额度」的交互在移植中未被建模（只保留 `AlwaysOff` / `AlwaysOn` / `Default` 三态语义）。
