# EasyDict Windows

轻量级 Windows 桌面翻译工具，基于 [Tauri 2](https://tauri.app/) 构建。

## 📥 下载安装

[![GitHub Release](https://img.shields.io/github/v/release/ROOOU/easydict-windows?style=flat-square&label=最新版本)](https://github.com/ROOOU/easydict-windows/releases/latest)

1. 前往 [Releases 页面](https://github.com/ROOOU/easydict-windows/releases/latest) 下载最新版 `.exe` 安装包
2. 双击运行安装（无需管理员权限）
3. 安装完成后从开始菜单或桌面启动 **EasyDict**

> **系统要求：** Windows 10/11

## ✨ 功能特性

- **多引擎翻译** — 同时对比多个翻译结果
  - Google 翻译（免费）
  - Bing 翻译（免费）
  - DeepL 翻译（需 API Key）
  - 百度翻译（需 API Key）
  - OpenAI / 自定义 LLM（支持任意兼容 API）
- **划词翻译** — 选中文本后自动翻译，支持浮动图标模式
- **截图翻译 (OCR)** — 框选屏幕区域，自动识别文字并翻译（基于 Windows OCR API）
- **TTS 朗读** — 使用 Windows 语音合成引擎朗读原文/译文
- **全局快捷键**
  - `Alt+A` 呼出输入翻译
  - `Alt+D` 划词翻译
  - `Alt+S` 截图翻译
  - `Alt+Shift+S` 截图 OCR（仅识别不翻译）
- **系统托盘** — 最小化到托盘，常驻后台
- **窗口置顶** — 一键置顶翻译窗口
- **深色/浅色主题** — 支持跟随系统或手动切换
- **自动语言检测** — 智能识别源语言

## 📦 技术栈

| 层级 | 技术 |
|------|------|
| 框架 | Tauri 2 |
| 前端 | 原生 HTML/CSS/JS（无框架） |
| 后端 | Rust |
| 截图 | xcap |
| OCR | Windows OCR API |
| TTS | Windows Speech Synthesis API |

## 🚀 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://rustup.rs/) ≥ 1.70
- Windows 10/11（OCR 和 TTS 功能依赖 Windows API）

### 开发运行

```bash
# 安装依赖
npm install

# 启动开发模式
npm run dev
```

### 构建发布

```bash
npm run build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## ⚙️ 配置

首次运行后，配置文件自动生成在：

```
%APPDATA%/EasyDictWin/config.json
```

可在应用内的设置页面修改：
- 启用/禁用翻译引擎
- 配置 API Key（DeepL、百度、OpenAI）
- 自定义 OpenAI API 地址和模型
- 设置默认目标语言
- 选择主题

## 📄 License

MIT
