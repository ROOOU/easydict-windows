# 会话压缩交接：Windows Easydict 移植

> 用途：上下文接近上限时，本文件是**压缩后的状态快照**。新会话或压缩后的会话先读本文件，
> 再按需读 `docs/windows-validation.md`（逐轮修改与证据，权威）和 `docs/mac-parity.md`（macOS 对照）。
> 只写已验证事实；未验证项一律标注。

- 更新时间：2026-10-19（Asia/Shanghai，第十七轮接续）
- 工作目录：`C:\Users\12064\Desktop\win翻译应用`（非 Git 仓库，改动直接落盘）
- 当前会话：本会话（Codex `01a14ead-025e-72b1-89b1-fa1f2b46f475` 的接续）。
  本轮 `read_thread` 可用，确认此前优化回合因远端 `invalid_request_error` 失败，
  随后的「继续」被中断；当前文件另有第十五轮修改，已保留并完成回归。
  → 压缩规则：本会话 ≥1.0 MiB 开始压缩（只读日志/JSON 摘要，回答变短）；≥1.5 MiB 时写完本文件后
  结束回合，请用户新开会话说「继续」。

## 目标（不变）

继续优化修复直到实现移植；做完后与 macOS 原版比对，发现不足则修复；保持 Windows 端轻量占用。

## 已完成到哪一步

- **第十七轮（当前，2026-10-19）**：窗口位置保存不再走通用偏好更新。
  `AppState::save_window_frames` 只合并几何、加密写盘成功后更新内存，复用设置事务锁；
  不重建服务/语言检测器、不刷新托盘、不广播整份设置，相同矩形不重复写盘。
  原生测试证明旧版单次重建 24 服务+1事件，新版两项均为0；三次重复记录加密文件
  字节不变，模拟保存失败不发布未落盘矩形，恢复后可保存。
  `window_frame::WRITING` 串行化出队到提交，`flush` 等待已在途批次再保存新队列；
  不再提前清 `SAVING`。去掉 flush 锁的变异在1.43秒失败，修复后连续3次约1.9秒通过。
  全量通过：前端53、Rust393单元+9传输+8原生，1项剪贴板忽略、6项既有警告。
  Release/NSIS于20:20:57构建成功（310.7秒），EXE14,804,480字节（`A7D7EE33…`）、
  安装器5,079,867字节（`CFFBC528…`）；无新增依赖或常驻线程。
  新Release可见验收：从托盘首次Alt+D读取记事本整句，三家在线服务返回译文；
  拖动/隐藏后改选`easier.`再Alt+D，同窗口正确更新原文与译文。临时记事本和样例
  已关闭/清理，保留新结果窗。未改服务/快捷键/隐私设置，样例可能新增历史。
  物理鼠标图标仍未验：自动划词钩子忽略注入事件，不能用Computer Use代替，
  未绕过保护。浏览器/WPS/微信、密码框、剪贴板回退、托盘实际退出、跨重启恢复、
  混合DPI、安装升级和长期资源占用仍待专项验收；已安装旧版未更新。
- **第十六轮（2026-10-19）**：修复 `window_frame::saver` 在空队列分支持锁后
  再次获取 `PENDING` 导致的自锁，后续主线程移动/缩放也会被阻塞。删除重复加锁，
  在原锁内清除 `SAVING`；纠正注释：500 ms 分批保存，并非完整拖动只写一次。
  新增真实事件回调+隔离设置库的三轮空闲/再移动回归：旧实现 20.04 秒 watchdog
  失败，新实现 5.63 秒通过。原生测试 watchdog 不再依赖已经死锁的事件循环退出。
  全量通过：前端 53、Rust 393 单元 + 9 传输 + 6 原生集成，1 项剪贴板仍忽略。
  Release/NSIS 于 19:53:30 构建完成（305.3 秒），EXE 14,810,624 字节（`98BBF6A2…`），
  安装器 5,080,528 字节（`072C5959…`），6 项既有警告。无新增依赖或常驻线程。
  可见桌面已验证：空闲后拖动、输入、Enter 查询，DeepL/Google/Bing 显示中文译文、
  结果窗自动增高；设置页切标签/进出详情回顶部；关闭隐藏后再显示仍保留译文。
  未修改服务或快捷键设置；窗口矩形正常更新，样例查询可能新增历史。结束保留本轮
  Release 主窗口，已安装旧版未更新（未运行安装器）。启动工具曾误选已安装版本，
  已核验并结束该测试进程，实际验收改用工作区完整路径启动。
  仍待：外部应用划词、跨进程重启恢复、混合 DPI、多屏、安装升级及长时间资源验收。
- **第十五轮（2026-10-19）**：修复两处真实缺陷，均有真机或变异证据。
  1. **`easydict://` 在真实系统路径下全部失效**：日志实测为 `deep link: query/` 且无后续查询。
     根因是 `ShellExecute` 调用处理程序前会规范化 URL，主机名后补 `/`
     （`easydict://query?text=…` → `easydict://query/?text=…`，`easydict://ocr` → `easydict://ocr/`），
     而 `handle_deep_link` 用裸 `match host` 匹配，`query/`、`ocr/`、`writeKeyValue/` 全不命中，
     链接被静默丢弃。用临时 scheme `ezprobe://` 经 `Start-Process` 实测确认该规则。
     修复：新增 `split_link`（路由末尾 `/` 归路由、查询串原样保留）+ 路由按主机语义小写比较。
     回归：Rust 单元 1 项；原生集成深链测试的输入改为 `ShellExecute` 真实交付形式
     （此前传干净写法，所以「测试通过、真实链接全废」）；变异检查去掉 `trim_end_matches('/')`
     后该原生测试以 `the query window stayed hidden after its deep link` 失败（25.06 秒）。
     `app/tools/dev/protocol-check.ps1` 改为**先普通启动注册、再经 `ShellExecute` 打开链接**
     （原来是 `Start-Process -ArgumentList $link`，绕过了规范化，正是漏检原因）。
     真机 `docs/protocol-check-20261019.json`：`status=completed`，6.9 秒，
     `launchedThroughShellExecute=true`，窗口出现、`deep link: query`、`query: window=fixed`
     （16 字符）、Google/Bing/DeepL 各 5 字符、注销/写回注册均通过。
  2. **设置页沿用旧滚动位置**：`.ezy-content` 是五标签共用滚动容器，切标签/进出服务详情不重置
     `scrollTop`。修复：新增 `app/src/settings/scroll.ts`，`SettingsApp` 按标签重置、
     `ServiceTab` 在列表↔详情切换时重置（都用 `useLayoutEffect`，绘制前重置，避免先闪一帧）。
     回归：`app/tools/settings-navigation.test.mjs` 2 项，
     变异检查（删 `ServiceTab` 的重置 effect）会失败。
  - 全量：前端 typecheck + 53 项测试通过；Rust 393 单元（1 项私有剪贴板仍忽略）+ 9 传输
    + 5 原生集成通过。Release/NSIS 于 18:34:49 构建成功，EXE 14,811,136 字节
    （`B7BCBE9E…`）、安装器 5,081,728 字节（`782BD24A…`）；协议检查脚本对新 EXE 复跑
    `status=completed`。无新增依赖、无新增常驻线程。
  - 未验收：浏览器/资源管理器**人工点击**链接、安装/升级/卸载后的注册状态、其余 URL 命令；
    设置页滚动修复的真实视觉效果；混合 DPI、多屏、可见窗口资源测量、与 macOS 并排比对。
- **第十四轮**：已实际读取指定 zstd 会话；其最后一次 `state.rs` 修改后触发
  `max-tokens`，代码有 4 个编译错误而非可用产物。现已补齐逐窗口智能查询：
  `intelligentQueryMode` 三窗口开关 + 独立的服务 `intelligentQueryTextType` 参与掩码，
  不混用输出模式；默认关闭，旧配置缺字段兼容，OCR/替换直接调用不受窗口筛选影响。
  修复 `start_query` 接线，保留 request ID/窗口隔离；URL 的窗口键转为布尔值。
  新增前端 2 项、Rust 4 项、隔离原生派发 1 项回归。全量为前端 51、Rust 392 单元
  + 9 传输 + 5 原生集成通过，1 项私有剪贴板仍忽略。没有新增依赖或常驻线程。
  Release/NSIS 于 19:14:45 构建成功（305.6 秒），EXE 14,810,624 字节（`9A1C1A6E…`）、
  安装器 5,081,132 字节（`0B5FDEC8…`）。原生桌面通道本轮可用：主窗口模式独立切换、
  Google 单词参与与输出模式独立切换均已观察；两个测试开关均恢复原值，未在线查询、
  未操作剪贴板或安装器。完整哈希及边界见 `windows-validation.md`。
- **第十 / 十一轮（前一会话）**：按需创建 WebView + 有界动作队列（每窗 32 条 / 单条 256 KiB）；
  隐藏窗口时 `TrySuspend` + 内存级别 LOW，显示前 `Resume` + 恢复可见；隐藏动作先唤醒、
  送达后 10 秒仍隐藏再挂起。修复两处实测缺陷（接口归属：必须经
  `ICoreWebView2Controller::CoreWebView2()` 再强转 `ICoreWebView2_3/_19`；异步派发竞态：
  `with_webview` 在工作线程只投递事件，返回即读必然过期）。
- **第十二轮（本会话）：`easydict://` 协议注册与冷启动**
  - 新增 `app/src-tauri/src/platform/protocol.rs`：`HKCU\Software\Classes\easydict` 写 URL Protocol
    （默认值、`URL Protocol` 标记、`shell\open\command` = `"<exe>" "%1"`），启动时（Release）
    每次重写，便携移动后自愈；`unregister` 可逆；不需要管理员权限、无新依赖
    （只加了 `Win32_System_Registry` 特性）。
  - `lib.rs`：`dispatch_deep_links`（跳过 `argv[0]`，大小写不敏感、容忍引号）供**冷启动**
    （`setup` 里解析自身命令行，之前完全没实现，链接被丢弃）与**单实例转发**共用；
    `handle_deep_link` 只把路由写进日志（不记查询串，`writeKeyValue` 带密钥）。
  - 修复：`easydict://query?text=` 过去只入队不显示窗口 → 现在先入队再 `request_show("fixed")`。
  - 清理：`app.exe --unregister-url-scheme`（退出码 0/1/2，事件循环前处理，避免被在跑的实例吞掉）；
    `app/src-tauri/installer-hooks.nsh` 的 `NSIS_HOOK_PREUNINSTALL` 只删指向 `$INSTDIR\app.exe`
    的那条注册。
  - 证据：3 项单元测试（含真实注册表临时 scheme 往返）；1 项冷启动原生集成测试
    （断言固定窗口可见 + 队列文本百分号解码正确 + `ocr` 链接开 OCR 窗口）；
    变异检查（删 `request_show` → 测试以 `the query window stayed hidden after its deep link`
    失败 25.05 秒）；真机脚本 `app/tools/dev/protocol-check.ps1` →
    `docs/protocol-check-20261018.json`（1.7 KB，`status=completed`，6.8 秒，报告 SHA-256
    `D5BCC706…`）：冷启动后注册命令
    指向 release exe、可见 Easydict 窗口出现、日志 `deep link: query`、日志
    `query: window=fixed`（文本真的进入查询，Bing/Google 已发起请求）；`--unregister-url-scheme`
    删除成功、再执行退出码 1；最后不带参数的普通启动把注册写回（结束状态：链接可用）。
  - 未验收：浏览器/`ShellExecute` 点击唤起、安装/升级/卸载清理（hooks 只验证被 NSIS 包含并
    通过 makensis）、`readValueOfKey` / `resetUserDefaultsData` /
    `saveUserDefaultsDataToDownloadFolder`、协议注册目前无设置开关（同 macOS 默认总注册）。
- **第十一轮测量（仍然有效）**：隔离 Release 探针，2 轮 × 45 秒，12 轮全部 `stableProcessTree=true`，
  `status=completed`。空闲 0/1/5 窗口私有内存 3.8 / 188.9 / 350.2 MiB、工作集合计
  16.0 / 375.4 / 682.5 MiB、进程 1 / 7 / 11；挂起 1/5 窗口 142.3 / 306.7 MiB、
  工作集合计 51.1 / 53.6 MiB、进程 7 / 11（不销毁窗口）。证据
  `docs/resource-probe-20261018-hidden.json`（基线 `…-idle.json`）。不含托盘/钩子/快捷键/可见交互。
- **第十二轮验证**：`cargo test` 退出码 0（383 单元 + 9 传输 + 3 隐藏原生窗口集成；1 项私有剪贴板
  显式忽略），前端 `npm run typecheck` 与 44 项测试通过；`dist` 在打包前重建。
- **第十三轮（本会话）：逐窗口显示配置 + 窗口几何记忆**
  - 逐窗口显示：新增 `app/src/hooks/windowSettings.ts`（`windowConfigurationValue` /
    `showsInputField` / `showsLanguageBar`），按窗口类型读
    `inputFieldCellVisible{Main,Mini,Fixed}`、`selectLanguageCellVisible{Main,Mini,Fixed}`；
    `QueryWindow` 按值条件渲染输入框与语言栏，两者都关时头部加 `ez-query-head-empty`
    去掉内边距与第二条分隔线；高级页新增「逐窗口显示」一节（三窗口 × 两开关）。
    缺失/非布尔值一律按上游默认「显示」。`types.ts` 补六个字段。
  - 窗口几何：新增 `app/src-tauri/src/window_frame.rs`。`Settings.windowFrames`
    （`BTreeMap<WindowType, WindowFrame>`，JSON 键为 `main`/`mini`/`fixed`，字段 x/y/width/height，
    物理坐标可负）保存每个查询窗口的原点 + **客户区尺寸**（Windows `set_size` 作用于客户区；
    按外框存会每次恢复都长一条标题栏，已由原生测试实测确认）。`Moved`/`Resized` 事件只入内存、
    静置 `SETTLE = 500 ms` 后由工作线程合并落盘（主线程不写盘）；「上次位置」显示时用
    `window::restore` 先定位再定尺寸，随后仍按工作区收拢；只记 main/mini/fixed。
    `WindowType` 补 `PartialOrd/Ord`；`main_window_width/height` 随主窗口同步更新。
  - 测试：前端 5 项（`app/tools/window-settings.test.mjs`）、Rust 5 项
    （`config::tests::per_window_switches_keep_their_upstream_wire_names_and_defaults` +
    `window_frame::tests` 4 项）、原生集成 1 项
    （`window_geometry.rs::a_remembered_frame_is_restored_and_a_zero_sized_one_is_ignored`，
    先按外框实现实测失败于 `… 452x278, expected … 436x239`，改按客户区后通过）。
    `selection-actions.test.mjs` 为 `../hooks/windowSettings` 补桩（否则 15 项选区回归因缺模块报错）。
  - 全量：`cargo test` 退出码 0（388 单元 + 9 传输 + 4 原生集成，1 项私有剪贴板忽略）；
    前端 typecheck 与 49 项测试通过。
  - 未验收：六种显示组合的真实外观、跨重启位置恢复、拖拽期间写盘次数、混合 DPI 跨屏恢复；
    上一行「智能查询模式」(`IntelligentQueryMode`) 仍未移植。
- **旧产物记录**（2026-10-18 18:06:07，第十三轮；已被第十四轮覆盖）：
  `app.exe` 14,789,632 字节（14.10 MiB），PE `0x8664`，SHA-256
  `9242A52F56F10921919D77002F4CA4B901B34EAD4BA180C8FAEFD61811343BDC`；
  NSIS 安装器 `Easydict_0.1.0_x64-setup.exe` 5,075,366 字节（4.84 MiB），SHA-256
  `64D42365E2583135E13AC10B73FE62697725DE8571C926670B8183CB1DA34F34`（完整值见
  `docs/windows-validation.md` 的「运行产物」）；整轮
  `tauri build --bundles nsis` 耗时 304 秒，返回 0；构建后未再改业务源码。
  注：上一轮（第十二轮）的 exe 为 14,743,040 字节 / `578476DD…`，协议检查脚本
  `docs/protocol-check-20261018.json` 测的是那个哈希，本轮未重跑。

## 关键文件

| 位置 | 作用 |
|---|---|
| `app/src-tauri/src/platform/protocol.rs` | 第十二轮核心：URL 协议注册/注销/回读 + 命令行解析 |
| `app/src-tauri/src/lib.rs` | `split_link`（第十五轮：`ShellExecute` 规范化后的路由匹配）、`dispatch_deep_links` / `handle_deep_link`、冷启动路由、`--unregister-url-scheme` |
| `app/src-tauri/installer-hooks.nsh` + `tauri.conf.json` | NSIS 卸载清理 `HKCU\Software\Classes\easydict` |
| `app/tools/dev/protocol-check.ps1` | 真机协议/冷启动检查（第十五轮改为经 `ShellExecute` 打开链接；输出 `docs/protocol-check-*.json`） |
| `app/src/settings/scroll.ts` | 第十五轮：设置页共用滚动容器的重置点 |
| `app/src-tauri/src/platform/webview_memory.rs` | 第十一轮：隐藏窗口挂起/唤醒 |
| `app/src-tauri/src/window_frame.rs` | 第十三轮：窗口几何记忆（`windowFrames` 读写、静置合并落盘） |
| `app/src/hooks/windowSettings.ts` | 第十三轮：逐窗口显示开关的唯一读取点 |
| `app/src-tauri/tests/window_geometry.rs` | 原生窗口集成测试（定位、挂起/唤醒、深链冷启动、记忆矩形恢复） |
| `app/src-tauri/examples/resource-probe.rs`、`app/tools/dev/resource-probe.ps1` | 资源探针（`-Scenarios idle,suspend`） |
| `docs/windows-validation.md` | 逐轮修改、证据、`仍未完成` 清单（权威记录） |
| `docs/mac-parity.md` | macOS 行为对照基线 |

## 下一步（按优先级）

1. 真实桌面验收剩余项：鼠标划词图标交互（注入事件被保护性忽略，需物理鼠标）、
   浏览器/WPS/微信取词、密码框/剪贴板专项；记事本快捷键划词已在第十七轮实测。
   另有多屏/混合 DPI、安装/升级/卸载、
   从浏览器/资源管理器人工点击 `easydict://` 链接（本机注册已指向
   `target\release\app.exe`，脚本已用同一 `ShellExecute` 路径验证）、可见窗口资源测量、
   与 macOS 并排比对。
2. 设置页滚动的三条导航路径已在第十六轮实测；下一步检查服务详情输出模式控件
   （不要把当前已启用类型误当成服务的能力上限）、六种逐窗口显示组合与跨重启位置恢复。
3. 功能缺口：HTTP 服务、有道 OCR 引擎、服务端语言识别校正、MDict、本地 CLI 翻译服务。
4. 可选的协议补全：`readValueOfKey` 等其余 URL 命令。

## 压缩与交接规则（本会话约定）

- 触发检查：每轮结束时看 `$env:DSH_HOME\sessions\--C-Users-12064-Desktop-win~7FFB~8BD1~5E94~7528--\<session-id>\session.v4.jsonl.zstd` 的大小。
- ≥ 1.0 MiB：开始压缩——更新本文件、回答变短、不再整段读取大 JSON/长日志（改用脚本只取摘要）。
- ≥ 1.5 MiB：写完本文件后结束回合，请用户新开会话说「继续」；新会话第一步读本文件。
- 压缩必须保留：目标、已验证数字、未完成清单、下一步、验证命令；不得把未验证项写成已完成。

## 常用命令

```powershell
cd C:\Users\12064\Desktop\win翻译应用\app
npm run typecheck; npm test; npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test window_geometry
cargo build --manifest-path src-tauri/Cargo.toml --release
& ./tools/dev/protocol-check.ps1 -OutputPath ../docs/protocol-check-<date>.json
cargo build --manifest-path src-tauri/Cargo.toml --release --example resource-probe --features tauri/custom-protocol
& ./tools/dev/resource-probe.ps1 -Windows 0,1,5 -Scenarios idle,suspend -Seconds 45 -Repetitions 2 -OutputPath ../docs/resource-probe-<date>.json
.\node_modules\.bin\tauri.cmd build --bundles nsis
```

注意：本机没有 PowerShell 7，`.ps1` 用当前 Windows PowerShell 5.1 直接执行
（`& ./tools/dev/resource-probe.ps1 ...`），不要写 `pwsh -File ...`。
`cargo test` 在 PowerShell 里会因 stderr 输出显示 `[exit code: 1]`，以 `$LASTEXITCODE` 为准。
已知历史编译警告（别声称零警告）：`niutrans.rs` 未用 `Language`、doubao 的
`EVENT_PREFIX`/`DATA_PREFIX`、volcano/youdao 若干未读字段。
