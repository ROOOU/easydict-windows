# macOS 行为对照与轻量化验收

日期：2026-10-19。这里区分源码对照、自动化检查和实机验收，不把构建成功视作完整移植。

## 对照基线

对照基线：上游 [tisfeng/Easydict](https://github.com/tisfeng/Easydict) 源码。
本地分析副本未确认 release/tag/commit，不能声称对齐某个确定发布版本。
本轮重新读取并核对以下文件的 SHA-256：

| 上游相对路径 | SHA-256 |
|---|---|
| `Easydict/Swift/Utility/EventMonitor/Core/EventMonitor.swift` | `DFCBE2BA5D2F07AB9D8034484090F41D114BBE18F87073E27F6EDDF715F1FB5E` |
| `Easydict/Swift/Utility/EventMonitor/Workflow/SelectionWorkflow.swift` | `B970F2473050A587672554AAF5F27A527FC80577FCCA068A18385A538294837D` |
| `Easydict/objc/ViewController/Window/WindowManager/EZWindowManager.m` | `22C2A007D7699641A0AA973E8D19B808A18ED66EF0D3BDCE9704458BD9808D64` |
| `Easydict/objc/ViewController/Window/WindowManager/EZLayoutManager.m` | `51AD7426609046F9A04BB66507F42B8A491B768519CBDCEBE01945C164848295` |

窗口不激活与消失规则另参考 `EZPopButtonWindow.m`、`PopButtonVisibilityController.swift`。
Windows 主要实现为 `src-tauri/src/platform/auto_selection.rs`、`platform/selection.rs`、
`src/windows/QueryWindow.tsx` 和 `src/settings/GeneralTab.tsx`。

## 本轮对照

| 行为 | 上游证据 | Windows 当前实现/证据 | 边界 |
|---|---|---|---|
| 后台划词开关 | `EventMonitor.swift` 的 `autoSelectText` 控制事件监听 | `autoShowQueryIcon` 控制原生钩子；托盘、设置、快捷键共用持久化路径 | 关闭状态生命周期测试通过，实际开关交互未验收 |
| 自动查询不是监听开关 | `EZWindowManager.m:303/321` 用 `autoQuerySelectedText` 控制打开后的查询 | 鼠标与快捷键均支持仅填原文；前端回归覆盖文本变化自动查询不能绕过预览 | 真实窗口操作未验收 |
| 点击与悬停 | `EZWindowManager.m:169/176`：隐藏主窗口且 clickQuery 开才要求点击，否则悬停 | 配置推导测试覆盖两种设置组合；原生窗口接收鼠标消息 | 不是菜单开关；悬停焦点行为需实测 |
| 不激活图标 | `EZPopButtonWindow` 不成为 key/main window | `WS_EX_NOACTIVATE`、`WS_EX_TOOLWINDOW` 和 `MA_NOACTIVATE` | 仅源码检查；不得称焦点验收通过 |
| 最小长度及排除语言 | `EventMonitor.swift:558` 的 `shouldShowAutoQueryIcon` | 长度限制 0..50，按 Unicode 字素计数，本地识别语言后过滤 | 字素/配置过滤测试通过；识别器与 Apple 实现不同 |
| 使用捕获文本 | `EZWindowManager.m:195` 先收起图标，用 `selectedText` 打开鼠标配置窗口 | 只向目标窗口发送捕获文本，不再次读取系统选区 | 前端回归验证无重复读取；事件实际投递未验收 |
| 旧操作失效 | `autoSelectionGeneration`、选区工作流 | 原生 generation、单读取线程；前端共享查询/OCR 失效序号 | 旧文本/空值/错误/预览竞态有回归；系统 UIA 在途调用不能强制中断 |
| 触发与消失 | 事件监视器及 visibility controller | 拖选、双击/三击、Shift+点击、编辑框 Ctrl+A；键入、滚动、切换前台等收起 | 基础手势有测试，未完整移植应用/网站排除和特殊应用规则 |
| 结果窗位置 | `getFloatingWindowInRightSideOfScreenPoint` / `getFloatingWindowInCenterOfScreenPoint` 使用当前屏 `visibleFrame` | 第九轮改用目标显示器物理工作区；右侧为右上角，保留负坐标，调整尺寸后收回工作区 | 几何单元和本机隐藏窗口 API 通过，跨 DPI/多屏实机仍待验 |
| 客户区与边框 | 上游布局区分窗口 frame 和内容 | 第九轮修复 Windows 客户区重复加边框，返回实际宽高给前端；手动改宽和被限高的程序调整可区分 | 无焦点原生窗口覆盖实际最小尺寸约束，不代表完整交互通过 |
| 按需查询窗口 | `EZWindowManager.m:206/214/222` 的 getter 首次访问时初始化，关闭后保留 | 第十轮托盘启动不创建 WebView，目标窗口首次使用创建并复用；有界动作队列跨越首次加载 | 隐藏原生测试为 0 -> 1，首次动作只领取一次；未测生产启动的可见交互 |
| 隐藏窗口的内存 | 上游保留 `NSWindow`，由 macOS 自行处理不可见窗口的内存 | 第十一轮在隐藏时挂起 WebView 并下调内存目标级别，显示前唤醒；进程与页面状态都不销毁 | 隔离探针测量隐藏空闲窗口；可见交互、长时间占用仍未测，也不是与 macOS 的并排对比 |
| URL 协议 | `Info.plist` 的 `CFBundleURLTypes` + AppDelegate 的 `kAEGetURL` | 第十二轮启动时写 `HKCU\Software\Classes\easydict`（URL Protocol），冷启动与二实例都从命令行解析链接；`query`/`ocr`/`writeKeyValue` 三个路由 | 注册读写删、命令行解析、窗口可见与文本入队有单测/原生/真机脚本证据；浏览器真实唤起、安装卸载清理、`readValueOfKey` 等仍未验收 |
| 逐窗口显示 | `WindowConfigurationKey.swift` 生成 `EZConfiguration_InputFieldCellVisible_Window<raw>_Key` 与 `…SelectLanguageCellVisible…`，`EZBaseQueryViewController.m:231/233` 读取，`WindowConfigurationView.swift` 是设置界面 | 第十三轮 `hooks/windowSettings.ts` 统一读取六个字段，`QueryWindow` 按窗口类型渲染头部，高级页有六个开关 | 读取映射、默认值、条件渲染和设置项有前端与 Rust 回归；三个窗口的组合显示未做桌面验收 |
| 窗口几何记忆 | `EZLayoutManager.m:53/59/65` 初始化并回写 `windowFrameWithType:`，`:217` 在变化时保存；`EZWindowManager.m:619/676` 用「上次位置」恢复 | 第十三轮 `Settings.windowFrames` 按窗口类型保存矩形，`window_frame.rs` 在移动/缩小时静置合并落盘，「上次位置」恢复位置与尺寸 | 映射、零尺寸拒绝、覆盖与 JSON 往返有单测；跨重启真机恢复、混合 DPI 跨屏、拖拽期间的写入次数未验收 |

## 轻量化事实与待测项

第十七轮补充：窗口几何记录独立于用户偏好更新，仍保留上游逐窗口矩形语义及旧版主窗宽高。
原生回归测得旧实现一次记录重建 24 个服务对象并广播 1 次设置；新实现两项均为 0，
相同矩形不再加密重写设置，失败不发布未落盘矩形。退出补写与后台提交串行，
主线程回归覆盖等待在途批次和保留并发设置编辑。没有新增依赖、常驻线程或轮询器；
这是具体工作量的减少，本轮未测量整进程树 CPU/内存，不能据此声明资源占用比例下降。

第十六轮补充：重新核对 `EZLayoutManager.m:217` 的窗口矩形记录语义，保留逐窗口记忆。
修复 Windows 保存工作线程退出时重复加锁导致的主线程卡死，原生回归覆盖三轮真实
移动/缩放、磁盘保存、空闲后再记录；Release 已完成拖动后在线查询与隐藏再显示的可见验收。
保存策略实际是每 500 ms 合并一批，而非完整拖动结束后只写一次；没有新增常驻线程或依赖，
但本轮没有重新测量资源占用，也没有验证跨进程重启或混合 DPI 恢复。

第十四轮补充：逐窗口 `IntelligentQueryMode` 已接通。对照 `QueryService.swift:89/401`、
`Configuration+Extension.swift:27/51` 与 `Defaults.Keys+Extension.swift:250`，将自动参与类型
与翻译/词典/句子输出模式分开；默认窗口模式关闭，可配置服务的参与掩码默认 7，
其他普通翻译服务使用普通文本/句子的默认组合。自动筛选采用一个汉字的词典阈值，
不修改已有输出路由阈值；OCR/替换直接调用不受窗口模式影响。
前端回调、配置迁移/合并、类型筛选、实例隔离及原生查询派发回归通过。
尚非完整在线服务或与 macOS 实机并排验证；未增加依赖、后台线程或轮询。

- 本轮图标是一个 Win32 窗口，没有新增 WebView、前端框架或依赖。
- `easydict://` 注册是每次启动写入当前用户注册表的三个值（约 200 字节），不常驻、不轮询、
  不新增进程或 WebView；解析命令行只在启动时做一次。多出的 7 KiB 二进制来自协议模块。
- 全局钩子不执行 UIA；选区读取至多一个在途工作线程。移动处理仅在图标可见时限频到 10 Hz。
- 关闭监听卸载钩子，保留一条等待消息的原生线程；不轮询剪贴板。未把此结构推断为具体 CPU/内存数字。
- 第十轮删除启动创建循环；创建在后台工作线程串行进行，托盘启动可以没有 WebView。
  首次运行或托盘失败仍显示主窗口。打开过的窗口关闭后保留，第十一轮起隐藏期间挂起 WebView、
  显示前唤醒，仍未做销毁回收或前端代码拆包。
- 第十一轮测量（隔离 Release 探针，隐藏空闲窗口，2 轮 × 45 秒，全部稳定进程树）：
  空闲 0/1/5 个 WebView 的私有内存为 3.8/188.9/350.2 MiB、工作集合计 16.0/375.4/682.5 MiB、
  进程数 1/7/11；挂起后 1/5 个窗口为 142.3/306.7 MiB、工作集合计 51.1/53.6 MiB，进程数不变。
  挂起只释放渲染进程可回收的内存，不销毁窗口；这是 Windows 端的实现选择，不是 macOS 实测值。
- `dispatch`、URL query、自动划词和前端设置导航统一进入目标队列，先监听后领取；
  原生每窗最多 32 条、单条文本 256 KiB，前端最多暂存一条，没有空闲轮询。
  第十二轮起，命令行里的 `easydict://` 链接也走这条队列：冷启动与二实例都先入队再显示窗口。
  根组件等待真实设置；原有结果监听就绪机制保留。StrictMode 和并发通知回归通过。
- 快捷键保存触发时的前台/焦点控件标识，实际读取前后变化则取消；这不是文本快照，
  同一控件内的选区变化及完整替换安全仍需完善。隐藏原生测试不运行实际业务页面，
  不能代替首次设置导航、普通/静默 OCR、真实应用划词与托盘交互验收。
- 待记录同一机器、同一配置下的冷/热启动、托盘空闲 60 秒、连续划词 5 分钟、逐一开关窗口后的
  完整进程树 RSS/私有内存、CPU 增量、进程数和句柄数；包括 WebView2 子进程，不能只测主 EXE。
  隐藏空闲窗口已按此口径测量（见上），可见窗口与真实操作负载仍未测量。
- 比较 macOS 时另记硬件、系统、运行时版本和启用服务，不用不同硬件的单次快照声称平台资源优劣。

## 尚未达到的验收门槛

1. 原生 UI 通道可用后，在真实应用验证鼠标图标、密码框、剪贴板、焦点、快速连续选择和混合 DPI。
   第九轮已修复结果窗按主屏/非负坐标定位的问题；本机隐藏原生测试仅覆盖缩放 1 的工作区，
   不能把坐标单测和单屏 API 测试当作混合 DPI 实机证据。
2. Windows 安装、升级、托盘退出，以及从浏览器点击链接的协议唤起，仍需端到端验收。
   第十二轮已用真机脚本证明冷启动链路（注册表命令 → 启动程序 → 窗口可见 → 文本进入查询），
   但那是脚本自己启动 exe，不是浏览器/`ShellExecute` 的唤起路径，也没有真的安装/卸载过。
3. 与固定 macOS 版本并排比较查询、OCR、快捷键、浮窗、输入法与设置；当前只是局部源码对照。
4. 逐窗口显示开关与窗口几何记忆已在第十三轮接通（六个字段 + `windowFrames`），但窗口动画语义、
   应用/网站规则、HTTP、MDict、CLI 服务、有道 OCR、远程语言校正及替换语义等缺口继续登记；
   跨重启窗口位置只有单元级覆盖，未做真机重启验收。
5. 补齐资源测量及持续使用稳定性，才可判断轻量化要求是否满足。

本轮测试和安装器记录见 [`windows-validation.md`](windows-validation.md)。目前总体目标仍未完成。
