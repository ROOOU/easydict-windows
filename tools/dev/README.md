# 桌面对话验收脚本

下表中的交互脚本只用于 Windows 真机验收，不是自动化测试，也不会在 CI 中运行。
它们会启动程序、抢占前台窗口并注入按键，**运行前请保存手头工作**。

| 脚本 | 用途 |
|---|---|
| `hotkey-harness.ps1` | 启动 Release 程序，用一个 WinForms 文本框当划词目标，发送 `Alt+D`，截图并打印窗口、剪贴板和程序日志。默认结束时会关掉它启动的程序。 |
| `target-hotkey.ps1` | 对**已经在运行**的程序做同样的事：创建划词目标、发送快捷键、截图。用于调试构建或 `npm run tauri dev`。 |
| `probe-focused.ps1` | 创建带选区的 WinForms 文本框（`-RichText` 换成 RichTextBox），调用 `selection-probe` 检查选区读取；同时验证剪贴板是否被恢复。 |
| `notepad-uia.ps1` | 在 Windows 记事本里输入并全选，调用 `selection-probe --uia`，检查 UI Automation 分支。 |
| `cdp-inspect.mjs` | 在设置了 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 的程序上执行一段 JS（默认打印页面文本），用于确认窗口 DOM 的真实内容。 |

示例：

```powershell
# Release 端到端
.\app\tools\dev\hotkey-harness.ps1 -WaitMs 8000

# 只测选区读取（先构建 selection-probe）
cargo build --manifest-path app\src-tauri\Cargo.toml --example selection-probe
.\app\tools\dev\probe-focused.ps1
.\app\tools\dev\probe-focused.ps1 -RichText -ProbeArgs '--uia'
```

`selection-probe` 的用法见 `app/src-tauri/examples/selection-probe.rs` 顶部注释。

## 隔离资源测量（不注入输入）

`resource-probe.ps1` 使用独立 Release example，不启动安装版，不加载生产设置，不创建托盘、
启用输入钩子、注册快捷键或触发翻译。它复用生产插件、IPC、AppState 和真实 React 页面，
只创建隐藏、无焦点的 0/1/5 个 WebView；设置、SQLite、WebView profile 和日志位于本次唯一临时目录。
探针使用与 Release 相同的 Windows GUI 子系统，避免控制台宿主污染基线；探针不进入安装包。

```powershell
cd app
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --release --example resource-probe --features tauri/custom-protocol
pwsh -NoProfile -File tools/dev/resource-probe.ps1
# 挂起场景与空闲场景对比（每种窗口数、每轮采样秒数可调）：
pwsh -NoProfile -File tools/dev/resource-probe.ps1 -Windows 0,1,5 -Scenarios idle,suspend -Seconds 45 -Repetitions 2
# 采样器纯计算回归，不启动探针：
pwsh -NoProfile -File tools/dev/resource-probe.ps1 -SelfTest
```

`-Scenarios` 接受 `idle`（只让隐藏窗口空闲）和 `suspend`（再让每个隐藏窗口挂起自己的
WebView，等价于程序关闭窗口后的状态）。`suspend` 会等待运行时对每个窗口的答复，并把
`scenario`、`suspended` 计数与每窗答案写进报告与逐点轨迹：运行时过旧或拒绝挂起时本轮失败，
不会把「请求被静默忽略」当成已挂起。探针与脚本都不依赖 PowerShell 7，Windows PowerShell 5.1
已实跑验证。

默认每种窗口数三轮，确定性轮换顺序。等待业务根组件（真实设置已加载）就绪后再静置 10 秒，
采样 60 秒。探针在采样期间阻塞等待 stdin，不运行测量定时器；采样器在外部进程记录整棵
活跃进程树的 CPU 时间、Private Bytes、Working Set 和句柄，并保存逐点原始 JSON 与环境/二进制哈希。
PID 与创建时间一起标识进程，拒绝 PID 重用；进程增减使 CPU 区间标为无效，不冒充零占用。
读取进程失败会使本轮失败而不是把部分内存当完整总量。

报告同时保存单核百分比和除以逻辑处理器数的整机百分比。Working Set 相加会重复计算共享页；
新建 profile 不等于操作系统磁盘缓存冷启动，短命进程也可能在两个采样点间被遗漏。
退出时检查已观测子进程结束，校验目录后只清理本次 profile，保留测量 JSON。`status=completed`
且每轮 `stableProcessTree=true` 才能使用完整 CPU 汇总；失败/中断报告不能当作全部完成。

这是受控的隐藏空闲窗口比较，不包括真实划词、截图、翻译服务负载或可见窗口交互，
不能用它声称整个生产应用在所有场景的资源占用，或已经完成 Windows/macOS 实机验收。

## `easydict://` 协议与冷启动（真机、会显示窗口）

`protocol-check.ps1` 用已构建的 Release 程序验证 URL 协议链路：先普通启动一次把
`HKCU\Software\Classes\easydict` 注册写下来（随后关掉该进程，否则第二次启动会被单实例插件
转发，测到的是热启动），然后用 **`ShellExecute` 打开** `easydict://query?text=…`——与资源管理器、
浏览器点击链接走同一条系统路径，包括 Windows 自己对 URL 的规范化。等待并断言五件事——注册表
`HKCU\Software\Classes\easydict\shell\open\command` 指向该 exe、程序日志出现
`deep link: query` 路由记录、日志出现 `query: window=fixed`（文本真的进了查询）、桌面真的出现
标题以 `Easydict` 开头的可见窗口、以及 `app.exe --unregister-url-scheme` 能删掉注册
（退出码 0，再执行一次为 1）。最后它**不带参数**启动一次程序，确认普通启动会把注册写回来，
因此检查结束时链接仍然可用。结果写入 JSON（含 exe 哈希与日志片段）。

> 这条路径本身就是缺陷来源：`ShellExecute` 会把 `easydict://query?text=…` 规范化成
> `easydict://query/?text=…`（主机名后多一个斜杠），而早期脚本是把链接当作**命令行参数**
> 直接启动程序的，绕过了规范化，于是「测试通过、真实链接全部失效」。现在脚本走
> `ShellExecute`，路由匹配也按规范化后的形式处理（`split_link`）。

```powershell
cd app
cargo build --manifest-path src-tauri/Cargo.toml --release
& ./tools/dev/protocol-check.ps1 -OutputPath ../docs/protocol-check-<date>.json
```

脚本会短暂显示翻译窗口、强杀自己启动的进程，并产生真实的翻译请求（用的是当前设置里的服务）；
它不能替代人工点击浏览器里的 `easydict://` 链接，也不覆盖安装/卸载。只想手工清理注册时执行
`app.exe --unregister-url-scheme`（0=已删除，1=本来就没有，2=失败）。
