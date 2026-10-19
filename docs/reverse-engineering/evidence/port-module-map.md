# 移植侧模块 / 命令 / 事件 / 替换点清单

对应卷：`05-architecture.md`。来源：`app/src-tauri/src/**` 与 `app/src/**` 源码字面量（截至 2026-10-16）。

## 1. 模块（`app/src-tauri/src/lib.rs`，16 个）

```rust
pub mod commands;  pub mod config;   pub mod detect;  pub mod model;
pub mod net;       pub mod ocr;      pub mod platform; pub mod query;
pub mod service;   pub mod state;    pub mod store;   pub mod system;
pub mod tray;      pub mod tts;      pub mod util;    pub mod window;
```

| 本仓库模块 | 上游对应 |
|---|---|
| `model` | `Model/`、`Service/Model/` |
| `config` | `Feature/Configuration/` |
| `service` | `Service/` |
| `detect` | `Service/Detect/` + `NLLanguageRecognizer` |
| `query` | `objc/Service/Model/EZQueryService.*` |
| `store` | `Model/Storage/` |
| `ocr`（`#![cfg(windows)]`） | Vision `VNRecognizeTextRequest` |
| `tts`（`#![cfg(windows)]`） | `NSSpeechSynthesizer` + `AVPlayer` |
| `platform`（`#![cfg(windows)]`） | Accessibility + `CGWindowListCreateImage` |
| `window` | `EZWindowManager` |
| `system` | 非 Windows 桩：`send_paste_keystroke` 返回错误、`system_languages()` 读 `$LANG` |

Tauri 插件注册顺序：`single-instance` → `log` → `opener` → `shell` → `dialog` → `fs` → `os` → `process` → `notification` → `autostart`（`MacosLauncher::LaunchAgent`）→ `clipboard-manager` → `store` → `global-shortcut`。`tauri-plugin-store` 已注册但持久化不使用它（`store/mod.rs` 自管 `settings.json` + SQLite）。

## 2. 命令面（`commands.rs`，48 个 `#[tauri::command]`）

| 分组 | 命令 |
|---|---|
| 设置（5） | `get_settings` `update_settings` `reset_settings` `export_settings` `import_settings` |
| 服务（6） | `list_services` `get_service_config` `update_service_config` `validate_service` `fetch_service_models` `service_languages` |
| 查询（5） | `query` `stop_query` `retry_query` `query_single` `translate_text` |
| 剪贴板 / 文本（4） | `clipboard_text` `set_clipboard_text` `selected_text` `replace_selected_text` |
| OCR（3） | `ocr_image` `screenshot_ocr` `pasteboard_ocr` |
| 语音（2） | `play_tts` `stop_tts` |
| 语言（4） | `detect_language` `supported_languages` `system_languages` `toggle_languages` |
| 历史 / 收藏（9） | `history` `search_history` `delete_history` `clear_history` `history_count` `favorites` `add_favorite` `remove_favorite` `is_favorite` |
| 窗口 / 应用（10） | `show_window` `hide_window` `pin_window` `set_window_size` `fit_window_to_content` `open_link` `app_info` `service_order` `set_service_order` `show_service_result` |

计数命令：`grep -c "^#\[tauri::command\]" app/src-tauri/src/**/*.rs` → **48**。

## 3. 事件（`commands.rs::events`）

| 常量 | 名称 | 载荷 |
|---|---|---|
| `QUERY_UPDATE` | `query-update` | `QueryEventPayload { identifier, serviceType, result }`（camelCase） |
| `QUERY_FINISHED` | `query-finished` | `()` |
| `ACTION` | `action` | `ActionPayload { action, text? }` |
| `OCR_RESULT` | `ocr-result` | `OcrOutput`（**已声明但无发送方**） |

## 4. 查询编排（`query/mod.rs` + `state.rs`）

- `ServiceState { generation: Mutex<u64>, result: Mutex<Option<QueryResult>>, stop: Mutex<Option<CancellationSource>> }`；`ServiceHandle { service: Arc<dyn QueryService>, state: Arc<ServiceState> }`。
- `Cancellation` 基于 `tokio::sync::watch`（`cancelled()` 循环等待 `changed()`）；`stop_all()` 逐个取消。
- `QueryRun::start(handles, model, tx)`：每服务一个 `tokio::spawn(run_one)`，先发 `Update{is_showing:true,is_loading:true}`，`select!{biased}` 处理取消与 `ServiceUpdate::{Chunk,Validation}`，全部结束后发 `QueryEvent::AllFinished`。
- `commands::query` 把 `QueryEvent` 翻成 `app.emit("query-update", …)`；仅 `AllFinished` 时写历史。
- `AppState::start_query` 先 `stop_query()`；`run_direct(handle, model)` 供 `validate_service` / `query_single` / `translate_text` 直连。
- `service/mod.rs::prehandle` 顺序：不支持语言 → 繁简本地转换 → 缺密钥（与上游顺序不同，见 `05-architecture.md` §4.3）。

## 5. 存储（`store/mod.rs`）

| 项 | 值 |
|---|---|
| 设置文件 | `settings.json`（美化 JSON；临时文件 → `sync_all` → `rename` 原子写） |
| 数据库 | `Easydict.db`（SQLite，`rusqlite` bundled；`busy_timeout` 5 s；`user_version = 1`） |
| 表 | `history` / `favorites`：`(id INTEGER PK AUTOINCREMENT, query_text, from_language, to_language, timestamp, translated_text)` |
| 索引 | `idx_history_order`、`idx_favorites_order`、`idx_favorites_text` |
| 历史上限 | `MAX_HISTORY = 1000`（对应 `QueryRecordManager.maxHistoryCount`） |
| 上游做法 | **UserDefaults 数组**（`EZConfiguration_kQueryHistory` / `EZConfiguration_kFavorites`），无 Core Data |
| URL Scheme 映射 | `lib.rs::handle_deep_link` + `upstream_key_to_field`（`EZYoudaoAPIKey → youdaoWebApiKey` 等 14 条，均有测试断言目标字段存在） |

## 6. macOS → Windows 替换点（文件级索引）

| 能力 | 上游符号 | 移植文件 |
|---|---|---|
| OCR | Vision `VNRecognizeTextRequest` | `src/ocr/mod.rs`（`Windows.Media.Ocr`） |
| 离线语音 | `NSSpeechSynthesizer` | `src/tts/mod.rs`（`Windows.Media.SpeechSynthesis`，线程 `easydict-tts`） |
| 音频播放 | `AVPlayer` | `src/tts/mod.rs`（`MediaPlayer`） |
| 截图 | `CGWindowListCreateImage` | `src/platform/*`（GDI `BitBlt`） |
| 选中文本 | Accessibility | `src/platform/*`（UI Automation，最多上溯 8 层） |
| 发送按键 | `CGEvent.post` | `src/platform/*`（`SendInput`） |
| 语言识别 | `NLLanguageRecognizer` | `src/detect/*`（Unicode 脚本分类器 + 上游纠正流程） |
| 菜单栏 | `MenuBarExtra` / `NSStatusItem` | `src/tray.rs` |
| 开机自启 | `SMLoginItemSetEnabled` | `tauri-plugin-autostart`（`commands::apply_launch_at_startup` / `sync_launch_at_startup`） |
| URL Scheme | `NSAppleEventManager` | `src/platform/protocol.rs`（HKCU URL Protocol 注册）+ `lib.rs::dispatch_deep_links`（冷启动与单实例 `argv`） |
| 全局快捷键 | 磁力 `HotKey` | `tauri-plugin-global-shortcut`（`lib.rs::register_shortcuts`） |
| 窗口定位 | `EZShowWindowPosition` | `src/window.rs::position` |
| 繁简转换 | `String.applyingTransform` | `service/languages.rs` 的 `S2T` 表 |
| 本地词典 / 系统翻译面板 / 划词图标 | DictionaryServices / `Translation` / `EZPopButtonWindow` | **未实现** |

## 7. 服务注册与过滤器（`service/registry.rs`）

- `service_types()` = `ServiceType::all()`（31 项）过滤 `is_available_on_windows() && is_implemented()` → **24 项**。
- 未实现/不可用 7 项：`Apple`、`AppleDictionary`、`MDict`、`ClaudeCode`、`CodexCLI`、`GitHubCopilot`、`SystemSpeech`（作为查询服务）。
- `descriptors()` 对全部 31 项输出并标 `available: false`，保证旧设置文件往返无损。

## 8. 前端（`app/src`）

```
App.tsx            按窗口 label 分流 QueryWindow / SettingsApp / OcrApp
windows/QueryWindow.tsx  查询窗口（ResizeObserver 自动适配，80 ms 防抖）
windows/ResultCard.tsx   结果卡
windows/TitleBar.tsx     标题栏（含动作按钮）
settings/SettingsApp.tsx 设置八标签页
ocr/OcrApp.tsx       OCR 窗口
hooks/useSettings useQuery useLanguages useAppearance
api.ts              每个命令一个 invoke 包装
```
