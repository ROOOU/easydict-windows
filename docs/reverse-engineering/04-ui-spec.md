# Easydict 逆向分析报告 — 界面规格（Windows 版）

> 目标：把 macOS 版的窗口体系、结果卡片、设置窗口、托盘菜单、快捷键与窗口定位规则固定下来，并给出 Windows 版逐项实现对照（含尺寸常量与按键映射）。
>
> 上游源码：`Easydict/objc/ViewController/**`、`Easydict/Swift/View/**`、`Easydict/App/EZConst.h`、`Easydict/Swift/Feature/Configuration/**`。
> 移植实现：`app/src-tauri/tauri.conf.json`、`app/src-tauri/src/{window,tray,config}.rs`、`app/src/{windows,settings,ocr}/**`。
>
> 分析日期：2026-10-16

---

## 0. 窗口清单

### 0.1 上游（macOS）

| 窗口 | `EZWindowType` | 类 | 样式 | 常驻 |
|---|---|---|---|---|
| 主窗口 | `0` (`main_window`) | `EZMainQueryWindow.shared` | Titled ｜ Resizable ｜ Miniaturizable ｜ Closable | 单例，可销毁重建 |
| 迷你窗口 | `1` (`mini_window`) | `EZMiniQueryWindow` | 同上 **+ NonactivatingPanel**，`collectionBehavior |= Transient｜IgnoresCycle` | 每次新建 |
| 固定窗口 | `2` (`fixed_window`) | `EZFixedQueryWindow.shared` | 同迷你窗口 | 单例 |
| OCR 窗口 | — | `OCRWindowManager` | 独立窗口 | 按需 |
| 设置窗口 | — | `SettingView`（SwiftUI `Settings` 场景） | 不可缩放（`styleMask.remove(.resizable)`），缩放按钮禁用 | 按需 |

公共窗口属性：`releasedWhenClosed = NO`、`movableByWindowBackground = YES`、`titlebarAppearsTransparent = YES`、`titleVisibility = NSWindowTitleHidden`、`hidesOnDeactivate = NO`；显示时层级 `EZFloatingWindowLevel = kCGModalPanelWindowLevel`(8)，固定后 `kCGUtilityWindowLevel`(19)，未固定 `kCGNormalWindowLevel`(0)。标题栏 `EZTitlebarHeight_28 = 28`。

### 0.2 Windows 移植（`tauri.conf.json`）

| label | 标题 | 初始尺寸 | 最小尺寸 | 关键标志 |
|---|---|---|---|---|
| `main` | Easydict | **500 × 240** | 320 × 120 | 原生边框、可缩放、启动时 `visible:false` + `center:true` |
| `mini` | Easydict | **400 × 200** | 260 × 120 | `decorations:false`、`alwaysOnTop:true`、`skipTaskbar:true`、可缩放 |
| `fixed` | Easydict | **420 × 240** | 260 × 120 | 同 `mini` |
| `settings` | Easydict 设置 | **820 × 580** | 640 × 460 | 居中、可缩放 |
| `ocr` | Easydict OCR | **620 × 440** | 420 × 300 | 居中、可缩放 |

- 五个窗口都加载**同一份前端 bundle**，`App.tsx` 用 `getCurrentWindow().label` 决定挂载哪个根组件（`settings → SettingsApp`、`ocr → OcrApp`、`mini|fixed|默认 → QueryWindow`）。
- 任何窗口的关闭请求都被拦截为**隐藏**（`api.prevent_close(); window.hide()`）；只有托盘「退出 Easydict」真正结束进程。
- 启动时按 `hide_main_window`（默认 `true`）决定主窗口是否显示；新装用户看到的是托盘常驻而不是窗口。

---

## 1. 窗口定位：`EZShowWindowPosition`

### 1.1 四个取值（上游 `objc/Service/Model/EZEnumTypes.h`）

| 值 | case | 英文文案 | 说明 |
|---|---|---|---|
| 0 | `.right` | Right side of screen | 屏幕右侧 |
| 1 | `.mouse` | Mouse position | 光标位置 |
| 2 | `.former` | Last position | 上次位置 |
| 3 | `.center` | Center of screen | 屏幕居中 |

各窗口的默认位置来自设置：`fixedWindowPosition = .right`（`EZConfiguration_kShowFixedWindowPositionKey`）、`miniWindowPosition = .mouse`（`EZConfiguration_kShowMiniWindowPositionKey`）。

### 1.2 定位数学（上游 `EZWindowManager.m`，坐标为「左上角点」）

- `.right`：`x = visibleFrame.x + visibleFrame.width − window.width`，`y = visibleFrame.y + visibleFrame.height`（即屏幕右上角）。
- `.mouse`：基准 `NSEvent.mouseLocation`；弹出按钮可见时 `x = popButton.x + 5`（若鼠标已在按钮右侧则用 `mouse.x + 5`）、`y = popButton.y`；`getPopButtonWindowLocation` 为零点时直接返回原点。
- `.former`：读 `windowFrameWithType:` 记录的 frame，用 `getFrameTopLeftPoint:` 把左下角换算成左上角；同时恢复 `formerFixedScreenVisibleFrame` / `formerMiniScreenVisibleFrame`。
- `.center`：`x = origin.x + (visibleFrame.width − width)/2`，`y = origin.y + (visibleFrame.height − height)/2 + height`。

显示时统一走：保存前台应用 → 计算安全点（`getFrameSafePoint:`）→ `setFrameOrigin:` → 提升层级 → 隐藏主窗口 → `makeKeyAndOrderFront:` → 聚焦输入框。`.former` 之外的浮动窗口在**失去焦点**时自动关闭（`windowDidResignKey:` → `closeFloatingWindowIfNotPinned:`），主窗口视作常驻。WebView 在关闭 `EZFloatingWindowIdleWebViewDiscardDelay = 60.0` 秒后被回收。

### 1.3 移植实现（`app/src-tauri/src/window.rs`）

```rust
pub enum Placement { Right, Mouse, Former, Center }
```

- `Placement::parse` 对大小写不敏感，无法识别或为 `None` 时回落 `Right`。
- `position(window, raw)`：`Former` 直接返回（交给系统/ Tauri 恢复）；`Mouse` 用 `cursor_position()` 换算成逻辑像素，并做 `min(screen − size).max(0)` 夹取，保证窗口不越界；`Center` 与 `Right` 都由主屏逻辑尺寸算出。全部计算在**逻辑像素**下完成，再由 Tauri 做 DPI 缩放。
- 主屏取不到时回退 `(1920, 1080)`。

**保真度损失**：上游按「最后点击的屏幕」({
`EZCoordinateUtils screenForPoint:`}) 定位，Windows 版统一用**主屏**；`.former` 依赖 Tauri 自己的窗口位置恢复，而不是显式记录的 frame + 屏幕可见区域。

---

## 2. 随内容自适应（auto-fit）

### 2.1 上游算法（`EZBaseQueryViewController.m` + `EZLayoutManager.m`）

行高：输入框行 = `queryModel.queryViewHeight`；语言行 = 35；结果行 = 可见时 `max(result.viewHeight, 30)`，隐藏时 `30`（`EZResultViewMiniHeight = 30`）。

```
contentHeight   = Σ(rowHeight + EZVerticalCellSpacing_7)             // 每行 +7
height          = clamp(contentHeight, 40, maxWindowSize.height)
scrollViewHeight = height + 0 + 6.5                                  // top/bottom inset
scrollViewHeight = MIN(scrollViewHeight, maxWindowSize.height − 28)   // 减去标题栏
showingWindowHeight = MIN(scrollViewHeight + 28, maxWindowSize.height)
y = window.y + (window.height − showingWindowHeight)                  // 保持左上角不动
```

- 最大高度 = `屏幕可见高度 × maxWindowHeightPercentage / 100`（默认 **100%**，可选 20/30/…/100），下限 40。
- 动画时长 `EZUpdateTableViewRowHeightAnimationDuration = 0.2` s，缓动 `kCAMediaTimingFunctionEaseInEaseOut`；`windowAnimationResizeTime:` 同步返回 0.2。
- 递归保护：`lockResizeWindow` / `isUpdatingWindowFrameInternally`，动画结束后（0.2 s）解锁；frame 变化小于 `0.5` 容差时提前返回。
- 输入框高度：主窗口最小 **75**（三行），固定窗口 **65**，迷你窗口 **54**（两行）～ **75**；上限均为 `屏幕高 × 0.3`；`showInputTextField:type` 为假时为 0。输入框总高 = 文本高 + `EZQueryViewExceptInputViewHeight = 32`。

### 2.2 移植实现（`QueryWindow.tsx` + `window.rs::fit_to_content`）

- 前端用 `ResizeObserver` 观察「标题栏 + 查询头 + 结果列」三个元素，测量 `chrome + head + content.scrollHeight`，去抖 **80 ms** 后调用 `fit_window_to_content(label, height)`。
- Rust 侧 `chrome = outer_height − inner_height`，`target = clamp(content_height + chrome, 120, 屏幕高 × percentage)`，`percentage` 为 `max_window_height_percentage.clamp(20, 100) / 100`。
- **手动缩放会永久关闭自适应**：`onResized` 事件里若高度与「最近一次自适应请求的高度」相差 > 4 px，即判定为用户拖动，置 `manualResizeRef`；最大化窗口不参与自适应。
- 与上游差异：上游是同步的 AppKit 动画（0.2 s），移植是「测量 → 命令 → 系统窗口缩放」，`resize()` 的 `animate` 参数**被接受但忽略**。

---

## 3. 查询窗口anatomy

### 3.1 输入区（上游 `EZQueryView.m`）

| 元素 | 规格 |
|---|---|
| 容器 | 圆角 `EZCornerRadius_8 = 8`；浅色底 `#F4F4F4`，深色底 `#303132` |
| 文本视图 | `EZTextView`（`allowsUndo`），`textContainerInset = (6, 8)`，字体 `systemFontOfSize:14 × fontSizeRatio`，滚动条按需 |
| 加载动画 | 5 个圆点，点宽 4、间距 6、边距 4，颜色 `#FF8E27`，圆角 = 宽/2；缩放关键帧 `[1.0, 2.0, 1.0]`，时长 `0.5`，逐点延迟 `0.12`，定时器间隔 `1.0` |
| 发音按钮 | 24×24，`bottom −4`、`left 6`；播放中换 `pause.circle`；提示「Play Audio, ⌘+S」 |
| 复制按钮 | 24×24，图标缩放 16×16；提示「Copy Text, ⌘+⇧+C」 |
| 语言检测按钮 | 高 20、圆角 10，左侧间距 6.5；浅色底 `#E8E8E8`（悬停 `#DCDCDC`、按下 `#CCCCCC`），深色底 `#3D3E3F`（悬停 `#47494A`、按下 `#585A5C`）；仅在检测到非 Auto 语言时 alpha = 1 |
| 清空按钮 | 24×24，`right/bottom −4`，`xmark.circle.fill`，色 `#868686`；提示「Clear All, ⌘+⇧+K」 |
| 占位文案 | 「`Enter` to query, `Shift + Enter` to newline」，查询超过 100 次后不再显示 |

按键：`Enter` 查询、`Shift+Enter` 换行、`Esc` 关闭窗口。`textDidChange` 后按 `EZAutoQueryWhenTextChangedDelay = 0.8` s 去抖自动查询；语言检测去抖 `EZDelayDetectTextLanguageInterval = 1.0` s。

**移植对照**（`QueryInput.tsx` / `LanguageBar.tsx`）：

| 上游 | 移植 |
|---|---|
| 语言选择在**结果行内的下拉**中切换 | 独立的 `LanguageBar`（from / to 两个下拉 + 「交换语言」按钮），语言列表来自 `service_languages` 命令 |
| 检测按钮在输入区左下方 | 语言条里的「自动检测」项显示在分隔线上方 |
| Enter / Shift+Enter / Esc | 一致；自动查询去抖改为 **400 ms** |
| 占位文案 | `main` 为「输入要翻译的文字」，浮动窗口为「输入翻译」 |

### 3.2 标题栏（上游 `EZTitlebar.m`）

按钮统一 24×24（图标 20×20），堆叠间距 4，`right −10`、`top +4`，`layoutDirection = RightToLeft`。

| 按钮 | 位置 | 行为 |
|---|---|---|
| `pinButton` | 最左 | 切换固定；**主窗口隐藏**该按钮；提示附带 `pinShortcutString` |
| `quickActionButton` | 堆叠最右 | 弹出快捷菜单：`add_to_favorites`、`replace_newline_with_space`、`remove_code_comment_symbols`、`split_words`、分隔线、`open_app_settings` |
| `googleButton` | 堆叠 | 打开 `https://www.google.com/search?q=%@` |
| `appleDictionaryButton` | 堆叠 | 打开 `dict://%@` |
| `eudicButton` | 堆叠 | 打开 `eudic://dict/%@` |

窗口本身**没有**自绘的最小化/关闭按钮——关闭键由 `Esc` / `closeWindowOrExitSreenshot` 提供，「清空」在输入区。

**移植对照**（`TitleBar.tsx`）：

- `mini` / `fixed` 是**无边框窗口**，自绘标题栏：左侧固定按钮、中间 `data-tauri-drag-region` 拖动区、右侧隐藏按钮；带一个显示窗口类型的小徽标（「迷你窗口」/「固定窗口」）。
- `main` 使用 Windows 原生边框，另加一行工具栏：标题 `Easydict` +（忙碌时）转圈 + 设置 / 历史 / 收藏 / 关于 四个按钮 + 固定按钮。
- **无对应物**：快捷动作菜单（换行转空格、去注释符、分词）只在设置里作为开关存在，未提供窗口内菜单；上游标题栏的 Google / Eudic / Apple Dictionary **链接按钮**未做按钮控件（对应功能改由 `google` / `eudic` / `appleDic` 三个应用内快捷键触发，见 §8.3；结果卡片页脚另有「Google 搜索」入口）。

---

## 4. 结果卡片（`EZResultView` + `EZWordResultView`）

### 4.1 卡片骨架

```
┌ topBarView（高 30，浅 #F1F1F1 / 深 #2C2D2E）────────────────────────────┐
│ [图标16] 服务名(13pt) [模型标签] [错误图标16] [加载点] … [重试22][停止22][折叠22] │
├ wordResultView（词典结果，高 = 30 + 内容高）或 纯文本/HTML ────────────────┤
│ 大字词头(24pt, semibold) 音标 /xx/ 🔊 标签行 词性释义 …                    │
├ 底部工具栏（高 34 = 6 + 24 + 4）───────────────────────────────────────┤
│ 🔊 复制 🔗 替换 Anki Markdown 切换                                       │
└─────────────────────────────────────────────────────────────────────┘
```

- 卡片圆角 8，底色浅 `#F6F6F6` / 深 `#303132`；`topBarView` 覆盖 `top.left.right`，用 `NSClickGestureRecognizer` 折叠/展开（展开时置 `manualShow = YES`）。
- 服务名宽度上限 `127 × windowWidthRatio`（windowWidthRatio = 宽 / 340），模型标签宽度上限 `120 × windowWidthRatio`、圆角 3、10 pt。
- 错误图标：警告类（UnsupportedLanguage / UnsupportedQueryType / 非流式的 NoResult）用 `disabled`，其余用 `error`。
- 底部按钮 24×24，间距 1，顺序：发音 → 复制 → 打开链接 → **替换选中文本**（`showReplaceButton` 为真时）→ Anki（`enableAnkiConnect` 且有词典内容时）→ Markdown 渲染切换（仅流式服务）。
- 悬停效果只作用于 `EZHoverButton` 系控件（折叠/停止/重试）：圆角 5，浅色悬停 `#E6E6E6`、深色 `#3D3F3F`。

### 4.2 词典区（`EZWordResultView.m`）

按构建顺序与间距常量：边距 8、垂直边距 12、垂直内边距 6。区块：大字词头（24 pt semibold）→ 译文/解释（14 pt，`translateResultsTopInset` 在词典查询下为 6）→ 「请看」提示 + 蓝色链接 → 音标（标签 13 pt medium `#7A7A7A`，值 `/…/`，音频按钮 23×23）→ 标签行（水平滚动，12 pt，边框 1.2、圆角 3、间距 6）→ 词性释义（词性 13 pt、释义 14 pt，多条用 `"; "` 连接）→ 变形（`"%@:"`）→ 简单词（最多 3 条，按钮最大宽 = 卡片宽/2）→ 同义词/反义词/搭配（最多 5 条）→ 词源。

字体全部乘 `fontSizeRatio`（见 §7）。

### 4.3 移植对照（`ResultCard.tsx` / `WordResultView.tsx` / `ErrorView.tsx` / `TranslatedTextView.tsx`）

- 卡片头：服务图标（首字母彩色圆底）→ 服务名（有 `link` 时可点击并在浏览器打开）→ 「流式」徽标 → 右侧操作区（加载转圈 / 停止 / 发音 / 复制）。
- 校验提示（`validationMessage`）以独立一行显示在卡片头下方。
- 卡片体优先级：错误 → 词典结果 → 译文（Markdown 或纯文本）；都没有时显示「正在查询…」或「没有结果」。
- 卡片尾：替换（仅 `showReplaceButton`）、Google 搜索、收藏（星标，异步查询 `is_favorite`）。
- **差异**：未实现 Anki 按钮、Markdown 手动切换按钮（Markdown 由设置总开关控制）、模型标签、「请看」提示与蓝色链接；卡片折叠交互改为「结果列表可滚动 + 自动适配高度」，没有单卡折叠。

---

## 5. 设置窗口

### 5.1 上游 8 个标签（`SettingView.swift`）

窗口宽 **960**、默认高 **720**；标签顺序与按需高度：

| # | 标签 | 键 | 图标 | 高度 |
|---|---|---|---|---|
| 1 | 通用 | `setting_general` | `gear` | 720 |
| 2 | 服务 | `service` | `briefcase` | 720 |
| 3 | 收藏 | `favorites.tab` | `star` | 640 |
| 4 | 禁用 | `disabled_app_list` | `nosign` | 500 |
| 5 | 快捷键 | `shortcut` | `command.square` | 720 |
| 6 | 高级 | `advanced` | `gearshape.2` | 720 |
| 7 | 隐私 | `privacy` | `hand.raised.square` | 340 |
| 8 | 关于 | `setting.about` | `info.bubble` | 300 |

**通用**：第一/第二语言 + 语言检测优化；输入框（清空输入、保留上次结果、激活时全选）；自动查询（选中文本 / OCR / 剪贴板 / 文本变化 + 自动播放发音 + 英式或美式发音）；自动复制（选中文本 / OCR / 第一段译文）；快捷链接（Google / Eudic / Apple Dictionary / 设置）；显示（Markdown 渲染开关）；应用设置（界面语言、外观、检查更新、自动检查、包含 Beta、开机启动、隐藏菜单栏图标、菜单栏图标样式）；字体（字号滑块 0…4）。

**服务**：左侧 `HSplitView`（窗口类型分段选择 `[fixed, mini, main]` + 「窗口配置」项 + 服务列表 + 添加/删除分段控件），右侧详情。窗口配置只有两项：`显示输入文本框`（`InputFieldCellVisible`）与 `显示语言选择栏`（`SelectLanguageCellVisible`），按窗口类型分别存储。添加服务弹窗按 `ServiceAPIKeyRequirement` 分组（内置 / 无密钥 / CLI / 用户密钥），支持多实例服务（`Type#uuid`）。

**高级**：通用（默认 TTS 服务、英文单词优先有道 TTS、文言文最小检测长度）；鼠标划词（自动显示图标、排除语言、最小长度滑块 0…50 步进 10、点击图标直接查询）；取词与替换（强制取词、取词方式、优先 AppleScript、兼容替换、自动全选、移除图书摘录）；文本处理（换行转空格、去代码注释符、自动分词）；OCR（有道 OCR、文本规范化、显示 OCR 菜单项、隐藏截图提示层）；窗口管理（划词窗口类型、快捷键窗口类型、固定/迷你窗口位置、显示时固定、隐藏主窗口、最大窗口高度百分比）；HTTP 服务（开关 + 端口）；Anki Connect（端点、牌组、模型、字段映射、预览）；词汇笔记本（目录选择）。

**快捷键**：全局 12 项 + 应用内 15 项，录制控件宽 150、高 28；冲突校验读系统符号热键 + 菜单项 `keyEquivalent` + 已存快捷键（自定义校验函数是空实现）。

**隐私**：隐私声明文本 + 崩溃日志开关 + 匿名统计开关。**关于**：图标（100×100）、名称（35 pt）、版本、版权、GitHub / 贡献者 / 致谢三个按钮。**收藏**：历史/收藏分段（默认选中「收藏」），导出 CSV（表头 `queryText,queryFromLanguage,queryToLanguage,translatedResult,timestamp`，文件名 `Easydict Favorites yyyy-MM-dd HH-mm-ss.csv`）。**禁用**：被禁用的应用列表（导入 `.app`）。

### 5.2 移植的 8 个标签（`SettingsApp.tsx`）

同样的八项，中文标签：**通用 / 服务 / 收藏 / 禁用 / 快捷键 / 高级 / 隐私 / 关于**。

| 标签 | 移植内容 |
|---|---|
| 通用 | 语言（第一/第二语言、语言检测优化）、外观（外观、字体大小、启用 Markdown 渲染）、自动查询（选中文本 / 剪贴板 / OCR / 文本变化）、自动复制（三项 + 选中文本为空时保留上次结果）、查询图标（自动显示、点击图标直接翻译、最小文本长度、排除语言）、启动与朗读（开机自启动、自动播放发音…） |
| 服务 | 「可用服务」列表 + 服务详情（启用、使用状态、名称、API Key、API 端点、模型、温度、流式输出、推理强度、隐藏 thinking 内容、启用自定义提示词、系统/用户提示词）；底部「不可用服务」分组（Apple Dictionary、Apple 翻译、MDict、Agent CLI 三项，标注「Windows 版不可用 / 尚未实现」） |
| 收藏 | 收藏列表（删除单条） |
| 禁用 | 已禁用的服务列表 |
| 快捷键 | 「全局快捷键」+「应用内快捷键」两组，录制控件、冲突提示（「与其他快捷键冲突」/「未设置快捷键，或该组合注册失败」）、清除按钮 |
| 高级 | 历史记录（数量）、服务排序、OCR（显示 OCR 菜单项、启用有道 OCR）、HTTP 服务（启用、端口）、导入 / 导出设置（文件路径 + 操作） |
| 隐私 | 允许发送崩溃日志、允许发送匿名统计 |
| 关于 | 名称、版本、数据目录、最近查询、项目链接 |

**未移植的设置项**（保留了字段但不产生行为，或干脆没有 UI）：Anki Connect、词汇笔记本、Eudic 快捷链接、Apple Dictionary 快捷链接、菜单栏图标样式、强制取词方式与 AppleScript 偏好、移除图书摘录、文言文最小检测长度、显示划线提示层、有道 OCR、HTTP 服务的实际服务端、`enableBetaFeature` 之外的 Beta 项。

---

## 6. 托盘菜单

### 6.1 上游（`MenuItemView.swift`，`MenuBarExtra` 样式 `.menu`）

顺序：版本号（点击打开 releases 页）→ 分隔 → 输入翻译 / 划词翻译 / 剪贴板翻译 /（条件）迷你窗口 → 分隔 → 截图翻译 / 静默截图 OCR /（条件）截图 OCR /（条件）剪贴板 OCR /（条件）显示 OCR 窗口 → 分隔 → 翻译并替换 / 润色并替换 → 分隔 → 设置(`⌘,`) / 检查更新 / 帮助（反馈、导出日志、日志目录）→ 分隔 → 退出(`⌘Q`)。每个菜单项都绑定对应快捷键；「显示 OCR 菜单项」为假时隐藏三个 OCR 项。

### 6.2 移植（`tray.rs`）

图标左键单击 → 显示主窗口（对应 macOS 点菜单栏图标）。菜单项（逐字）：

```
输入翻译 · 截图翻译 · 划词翻译 · 剪贴板翻译
────
迷你窗口 · OCR 窗口
────
☑ 自动划词翻译
────
收藏 · 历史记录
────
偏好设置… · 关于 Easydict
────
退出 Easydict
```

- 「自动划词翻译」是 `CheckMenuItem`，切换 `auto_show_query_icon` 后**重建整个菜单**（Tauri 的 check item 不能原地更新）。
- 「偏好设置…」「收藏」「历史记录」「关于」都先打开设置窗口，再向前端发 `action` 事件。
- **差异**：没有版本号项、检查更新、帮助子菜单（反馈 / 导出日志 / 日志目录）、静默截图 OCR、翻译并替换 / 润色并替换两项（它们只在全局快捷键与设置里存在）。

---

## 7. 外观与字体

### 7.1 外观

`AppearanceType`：`followSystem = 0` / `light = 1` / `dark = 2`（键 `EZConfiguration_kApperanceKey`，注意上游拼写为 `Apperance`）。上游通过 `NSApplication.shared.appearance` 设置，并监听 `AppleInterfaceThemeChangedNotification`；移植由 `useAppearance()` 在 `document.documentElement` 上写 `data-theme`，CSS 变量 `--ez-*` 在 `:root[data-theme="dark"]` 下切换，主色 `--ez-accent: #0a84ff`。

窗口/卡片配色（上游 `NSColor+MyColors.m`，供移植对齐色板）：

| 用途 | 浅色 | 深色 |
|---|---|---|
| 查询区底色 | `#F4F4F4` | `#303132` |
| 查询/结果文字 | `#262626` | `#E0E0E0` |
| 结果卡底色 | `#F6F6F6` | `#303132` |
| 卡片头条 | `#F1F1F1` | `#2C2D2E` |
| 按钮悬停 | `#E2E2E2` / `#E6E6E6` | `#515253` / `#3D3F3F` |
| 表格行 | `#FFFFFF` | `#28292A` |
| 加载点 / 停止 / 清空图标 | `#FF8E27` / `#707070` / `#868686` | 同 |

### 7.2 字体大小

- 键 `EZConfiguration_kTranslationControllerFontKey`，`Key<UInt>`，默认 `0`；滑块 `0…4` 步进 1，比例表 **`[1, 1.1, 1.2, 1.3, 1.4]`**（`MyConfiguration.fontSizeRatio`）。
- 变更广播 `didChangeFontSize`；所有视图重算字体：输入框 `14 × ratio`，卡片头 `13`，大字词头 `24 × ratio`，译文 `14 × ratio`，音标 `13 × ratio`，标签 `12 × ratio`；词典 WebView 注入 `changeWebViewBodyFontSize(ratio)` / `changeIframeBodyFontSize(ratio)`。
- 快捷键 `⌘+` / `⌘-` 调整（不越界）。
- 移植：`Settings::font_size_option_index` 同样五档（`FONT_SIZE_RATIOS = [1.0, 1.1, 1.2, 1.3, 1.4]`，`useFontRatio()` 做夹取），行内快捷键 `Ctrl+=` / `Ctrl+-` 复用同一逻辑。

---

## 8. 快捷键

### 8.1 键位映射规则

| macOS | Windows |
|---|---|
| `⌘`（Command） | **Ctrl** |
| `⌥`（Option） | **Alt** |
| `⇧`（Shift） | Shift |
| `⌃`（Control） | Ctrl（与 `⌘` 合并，Windows 无独立映射） |

上游用 Magnet 的 `KeyCombo`（`cocoaModifiers` + `Key` 枚举）序列化为 **JSON `Data`** 存进以 `_keyHolder` 结尾的 UserDefaults 键；移植存成 `settings.shortcuts` 里的字符串（如 `"Alt+A"`、`"Ctrl+Shift+C"`），由 `tauri-plugin-global-shortcut` 注册。

### 8.2 全局快捷键（12 项）

| 动作 id | 上游默认 | Windows 默认 | 说明 |
|---|---|---|---|
| `inputTranslate` | `⌥A` | **Alt+A** | 输入翻译 |
| `snipTranslate` | `⌥S` | **Alt+S** | 截图翻译 |
| `selectTranslate` | `⌥D` | **Alt+D** | 划词翻译 |
| `showMiniWindow` | `⌥F` | **Alt+F** | 显示迷你窗口 |
| `silentScreenshotOCR` | `⌥⇧S` | **Alt+Shift+S** | 静默截图 OCR |
| `toggleAutoSelectText` | 无 | 无 | 切换自动划词 |
| `pasteboardTranslate` | 无 | 无 | 剪贴板翻译 |
| `polishAndReplace` | 无 | 无 | 润色并替换 |
| `translateAndReplace` | 无 | 无 | 翻译并替换 |
| `screenshotOCR` | 无 | 无 | 截图 OCR |
| `pasteboardOCR` | 无 | 无 | 剪贴板 OCR |
| `showOCRWindow` | 无 | 无 | 显示 OCR 窗口 |

上游的全局键由 Carbon `RegisterEventHotKey`（Magnet）注册，标识即 `ShortcutAction.rawValue`；移植注册成功与否只记警告，失败不阻断启动。

### 8.3 应用内快捷键（上游 15 项 / 移植 15 项）

| 动作 id | 上游默认 | Windows 默认 | 移植 |
|---|---|---|---|
| `clearInput` | `⌘K` | Ctrl+K | ✅ |
| `clearAll` | `⌘⇧K` | Ctrl+Shift+K | ✅ |
| `copy` | `⌘⇧C` | Ctrl+Shift+C | ✅ 复制输入框文本（上游 `copyQueryText()` → `inputText`） |
| `copyFirstResult` | `⌘⇧J` | Ctrl+Shift+J | ✅ 复制第一个服务的 `result.copiedText`（上游 `copyFirstTranslatedText()`） |
| `focus` | `⌘I` | Ctrl+I | ✅ |
| `play` | `⌘S` | Ctrl+S | ✅ |
| `retry` | `⌘R` | Ctrl+R | ✅ |
| `toggle` | `⌘T` | Ctrl+T | ✅（交换语言并重查） |
| `pin` | `⌘P` | Ctrl+P | ✅ |
| `hide` | `⌘Y` | Ctrl+Y | ✅ |
| `increaseFontSize` | `⌘` + 小键盘 `+` | Ctrl+= | ✅（也接受 `+`） |
| `decreaseFontSize` | `⌘` + 小键盘 `-` | Ctrl+- | ✅（也接受 `_`） |
| `google` | `⌘Return` | Ctrl+Enter | ✅ 打开 `https://www.google.com/search?q=<文本>` |
| `eudic` | `⌘⇧Return` | Ctrl+Shift+Enter | ✅ 打开 `eudic://dict/<文本>` |
| `appleDic` | `⌘⇧D` | Ctrl+Shift+D | ✅ 打开 `dict://<文本>`（Windows 通常无 `dict:` 处理器，由系统提示打开方式） |

另外：`Esc` 在浮动窗口关闭窗口（上游 `cancelOperation:` 同）；上游的小键盘 `+`/`-` 在 Windows 上换成了主键盘的 `=`/`-`（因为全局快捷键注册不区分小键盘）。

**重绑定**：应用内快捷键由 `QueryWindow` 在按键时读取 `settings.shortcuts` 决定，与上游一致——改绑立即生效，留空即"未设置"（不回退到默认值）。绑定的解析同时比对 `event.key` 与 `event.code`，因此 `Numpad5` 一类由 `code` 拼写的绑定也能命中。

### 8.4 上游菜单内的隐含快捷键

主菜单 `CommandMenu("shortcut")` 顺序：清空输入 / 清空全部 / 复制 / 复制第一段译文 / 聚焦 / 播放 / 重试 / 交换语言 /（分隔）/ 固定 / 隐藏 /（分隔）/ 放大字号 / 缩小字号 / Google / Eudic / Apple Dictionary。托盘菜单里 `设置 = ⌘,`、`退出 = ⌘Q`；移植由系统窗口菜单与托盘「退出」承担。

---

## 9. OCR 窗口

- 上游：`OCRWindowManager` 独立窗口；截图 OCR 有独立覆盖层（`Esc` 取消、`D` 复用上次选区、右键取消），静默模式不显示覆盖层；`AppleOCREngine.warmUpVisionOCRIfNeeded()` 在启动时预热。
- 移植（`app/src/ocr/OcrApp.tsx`，窗口 620×440）：
  - 左栏：识别文本（**可编辑**，便于用户修正后再翻译）+ 「识别」（重跑同一来源：截图 / 剪贴板）+「显示行」开关（显示每行与置信度行数信息）+ 复制；
  - 右栏：译文（`translate_text` 产出），可自动（`auto_query_ocr_text` 为真）或手动触发；
  - `onOcrResult` 事件在窗口级订阅，因此截图 OCR / 静默截图 OCR 的结果可以直接推进来；
  - `normaliseOcrText()`（`QueryWindow.tsx`）：按行去多余空白、丢空行、再用 `\n` 连接——与上游的 OCR 文本规范化对应。
- **差异**：无截图覆盖层（选区由 `screenshot_ocr` 命令自带），无有道 OCR 后端（`enable_youdao_ocr` 只是设置项），`Windows.Media.Ocr` 不返回置信度（见 `05-architecture.md`）。

---

## 10. 没有 Windows 对应物的交互

| macOS 交互 | 上游实现 | Windows 处理 |
|---|---|---|
| 划词后自动弹出的圆形图标 | `EZPopButtonWindow`（层级 `kCGScreenSaverWindowLevel`，偏移 (18, −12)） | **未实现**：`auto_show_query_icon` 只是设置项 + 托盘勾选项；划词翻译只能通过全局快捷键（Alt+D）触发 |
| 菜单栏图标样式（方形/圆角） | `MenuBarIconType` + `NSStatusItem` | 未实现（Windows 托盘图标固定） |
| 隐藏菜单栏图标（带二次确认） | `EZConfiguration_kHideMenuBarIconKey` | 未实现（隐藏托盘图标会让进程失去入口） |
| 双击修饰键作为快捷键 | Magnet `doubledModifiers` | 未实现（需要钩子监听修饰键按下/抬起） |
| 系统符号热键冲突检测 | Carbon `CopySymbolicHotKeys` | 未实现（只检查已保存的快捷键与注册结果） |
| 「强制取词」四种策略（模拟 ⌘C / 菜单栏复制 / Accessibility / AppleScript） | `SelectedTextKit` | 统一走 UI Automation（`TextPattern` → `ValuePattern`），失败即无结果 |
| 菜单栏「操作复制」回退 | 读前台应用 Edit 菜单 | 未实现 |
| 截图权限提示与深链 | `CGRequestScreenCaptureAccess` + 系统偏好设置深链 | 无对应（GDI 截图不需要授权） |
| Eudic / Apple Dictionary 快速链接 | 标题栏按钮 + `dict://`、`eudic://` | 未实现 |
| 自动更新（Sparkle） | `updater.checkForUpdates()` | 未实现（仅保留版本号展示） |
| 翻译快捷指令（AppleScript） | `runTranslateShortcut` | 无对应（Apple 服务整体不可用） |

---

## 11. 字符串与本地化

- 上游用 `Localizable.xcstrings`（含 `menu_input_translate`、`setting_general`、`service.mdict.name` 等数百条）；移植直接在前端写**简体中文**字面量（如「输入翻译」「偏好设置…」「已复制」「正在查询…」），并用 `ServiceType::name_key()` 保留上游键名以便未来接 i18n。
- 上游界面语言支持 English / 简体中文 / 繁體中文 / Slovak / 日本語 / Español；移植目前只有简体中文一份文案。
- 服务显示名沿用上游本地化键（`youdao_dict` → 有道词典、`built_in_ai` → 内置 AI 翻译、`polishing_service` → 润色、`summary_service` → 总结 等）。
