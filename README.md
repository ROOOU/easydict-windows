# Easydict for Windows

[Easydict](https://github.com/tisfeng/Easydict) 的 Windows 移植版，基于
**Tauri 2 + Rust + React 19 + TypeScript**。目标是复刻 macOS 原版的查询窗口、
划词翻译、截图 OCR、词典、朗读、快捷键和托盘体验，同时保持 Windows 端的轻量占用。

> **项目状态：** 可构建、核心翻译流程可用的功能移植版，尚非与 macOS 完全对等。
> 已实现与未验收的内容在下文分别说明。

[下载预览版](https://github.com/ROOOU/easydict-windows/releases/tag/v0.1.0-preview.1) ·
[Windows 验收记录](docs/windows-validation.md) ·
[macOS 对照记录](docs/mac-parity.md)

## 功能

- **多翻译服务聚合**：有道、DeepL、Google、Bing、百度、腾讯、阿里、火山、
  新译、彩云小译、豆包。
- **AI 翻译**：OpenAI、Gemini、Claude、DeepSeek、Groq、智谱、MiniMax、
  GitHub Models、Ollama、自定义 OpenAI 兼容服务，以及润色和总结工具。
- **词典查询**：音标、词性释义、词形变化、近义词、反义词、常用搭配、词源和例句。
- **多种入口**：主窗口输入、快捷键划词、截图翻译、剪贴板图片 OCR、`easydict://`
  URL 协议，以及可开关的划词翻译小图标。
- **原生 Windows 能力**：UI Automation 读取选区，Win32 框选截图，
  `Windows.Media.Ocr` 识别，系统语音合成兜底朗读。
- **多窗口与托盘**：主窗口、迷你窗口、固定窗口、OCR 窗口和设置窗口；
  关闭窗口默认隐藏复用。
- **可配置快捷键**：默认 `Alt+A` 输入翻译、`Alt+D` 划词翻译、
  `Alt+S` 截图翻译、`Alt+Shift+S` 静默截图 OCR；全部可在设置中修改。
- **轻量窗口生命周期**：WebView 按需创建；隐藏窗口申请挂起渲染进程，
  再次显示前唤醒。
- **本地数据**：设置、查询历史、收藏和窗口几何信息保存在本机。

## 下载与安装

1. 从 [Releases](https://github.com/ROOOU/easydict-windows/releases/tag/v0.1.0-preview.1)
   下载当前源码对应的 Windows x64 预览安装包。
2. 运行 NSIS 安装程序并按提示完成安装。
3. 从开始菜单或桌面快捷方式启动 **Easydict**。

**系统要求**

- Windows 10 或 Windows 11（x64）
- Microsoft Edge WebView2 Runtime（Windows 11 通常已内置）

安装包当前未做代码签名，Windows 可能显示“未知发布者”提示。

## 快速开始

### 环境要求

- Node.js 20+
- Rust stable（MSVC 工具链）
- Visual Studio 2022 Build Tools，安装“使用 C++ 的桌面开发”
- WebView2 Runtime

### 开发运行

```powershell
npm install
npm run tauri dev
```

### 构建

```powershell
npm install
.\node_modules\.bin\tauri.cmd build --bundles nsis
```

安装包输出到：

```text
src-tauri/target/release/bundle/nsis/
```

## 配置服务

应用不内置任何商业服务密钥。首次启动后打开 **设置 → 服务**，按需填写自己的凭据。

| 服务 | 凭据 |
|---|---|
| Google / Bing | 无需密钥 |
| DeepL | 可选 Auth Key；免费版 Key 以 `:fx` 结尾 |
| 有道 | `youdaoWebApiKey` |
| 百度 | App ID + Secret Key |
| 腾讯 | Secret ID + Secret Key |
| 阿里 | AccessKey ID + AccessKey Secret |
| 火山 | AccessKey ID + Secret AccessKey |
| 新译 / 彩云 | API Key / Token |
| 豆包 | API Key |
| OpenAI / Gemini / Claude / DeepSeek 等 | 对应服务商的 API Key |
| Ollama | 无需密钥，需在本机运行 `ollama serve` |

新安装默认启用 Google、Bing 和 DeepL。需要凭据的服务默认关闭；已有配置不会被覆盖。

## 验证

```powershell
npm run typecheck
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

最近一次完整检查结果：

- 前端：53 项测试通过，TypeScript 类型检查和 Vite 生产构建通过。
- Rust：394 项单元测试、9 项传输测试、8 项原生集成测试通过。
- 1 项私有 window station 剪贴板测试因本机访问权限被拒绝而保持 `ignored`，
  **未通过、未计入已验证项目**。
- Release x64 程序与 NSIS 安装器构建成功。

构建仍保留少量既有编译警告，不宣称零警告。

### 实际桌面验证

已在 Windows 桌面使用记事本完成以下检查：

- 选中文本后按 `Alt+D`，固定窗口显示原文。
- DeepL、Google、Bing 均返回中文译文。
- 拖动窗口、隐藏后更换选区再按 `Alt+D`，同一结果窗口正确更新原文和译文。
- 物理拖选英文后，选区旁的原生“译”图标能够显示。

以下项目仍待专项验收：

- 物理拖选后点击/悬停图标打开结果窗口。Computer Use 的鼠标事件会被程序按设计过滤，
  因此注入鼠标无法覆盖这条完整链路，需要人工操作验收。
- Chrome/Edge、WPS、微信等应用中的选区读取。
- 密码框、剪贴板复杂格式和并发复制场景。
- 混合 DPI、多显示器、负坐标和远程桌面。
- 安装、升级、卸载及 URL 协议冷启动。
- 可见窗口的长期 CPU、内存和句柄占用。

完整测试记录与验收边界见 [`docs/windows-validation.md`](docs/windows-validation.md)。

## 项目结构

```text
src/                          React 19 + TypeScript 前端
  api.ts                      Tauri 命令类型化封装
  windows/                    主窗口、迷你窗口、固定窗口
  settings/                   设置页
  ocr/                        OCR 窗口
  hooks/                      设置、语种、查询状态
src-tauri/
  src/
    model/                    领域模型
    config/                   设置结构、默认值和持久化
    service/                  翻译与 AI 服务
    detect/                   语种识别
    query/                    查询编排、取消和过期结果
    store/                    设置、历史与收藏
    ocr/ tts/ platform/       Windows 原生能力
    commands.rs               Tauri 命令入口
  tests/                      原生窗口集成测试
docs/
  windows-validation.md       Windows 验证与验收记录
  mac-parity.md               macOS 原版对照记录
```

## 隐私与数据

- 翻译文本会发送给当前启用的在线服务商，请先阅读对应服务商的隐私政策。
- 设置文件使用 Windows DPAPI 加密，只能由同一 Windows 账户解密。
- 历史与收藏数据库不做加密；请勿在其中保存敏感信息。
- 默认导出不包含 API Key、Cookie 和服务凭据；勾选“包含密钥”后会导出明文凭据。
- DPAPI 不能防御同一 Windows 账户下运行的恶意程序。

## 已知限制

- 后台鼠标自动划词、截图框选、多屏和混合 DPI 尚未完成真实桌面专项验收。
- HTTP 接口、有道 OCR、服务端语言检测校正、MDict 本地词典尚未实现。
- `Claude Code`、`Codex`、`GitHub Copilot CLI` 等本地 CLI 服务仍为占位。
- `Ctrl+Shift+D` 的 Apple 词典入口保留原版语义，但 Windows 默认没有 `dict:` 处理程序。
- 项目尚未与 macOS 原版做完整并排交互对比。

## 致谢与许可

本项目基于 [tisfeng/Easydict](https://github.com/tisfeng/Easydict) 的交互与实现进行
Windows 移植。上游项目以 **GPL-3.0** 发布，本移植沿用同一许可证，完整条款见
[`LICENSE`](LICENSE)。

翻译服务、词典数据、OCR 语言包和语音资源分别受其各自服务条款与许可证约束。
