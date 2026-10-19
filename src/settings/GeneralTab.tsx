/**
 * 通用 — languages, appearance, the auto-* behaviours and startup.
 *
 * Every control writes immediately except the two numeric text fields, which go
 * through the debounced `TextField`.
 */

import { FONT_SIZE_RATIOS } from "../api";
import { useLanguages } from "../hooks/useLanguages";
import type { AppearanceType, DesktopStatus, LanguageDetectOptimize } from "../types";
import {
  Row,
  Section,
  Segmented,
  Select,
  Slider,
  Toggle,
  type SelectOption,
  type SettingsTabProps,
} from "./components";

const FONT_SIZE_LABELS = ["很小", "小", "标准", "大", "很大"];

const APPEARANCE_OPTIONS: readonly SelectOption<AppearanceType>[] = [
  { value: "followSystem", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

const DETECT_OPTIONS: readonly SelectOption<LanguageDetectOptimize>[] = [
  { value: "none", label: "关闭" },
  { value: "baidu", label: "百度" },
  { value: "google", label: "谷歌" },
];

export function GeneralTab({ settings, patch, desktop }: SettingsTabProps & { desktop: DesktopStatus | null }) {
  const languages = useLanguages();

  const languageOptions: SelectOption<string>[] = languages.languages
    .filter((language) => language.value !== "auto")
    .map((language) => ({ value: language.value, label: language.chineseName }));

  // The excluded-language picker keeps `自动` as the "no exclusion" entry.
  const excludedOptions: SelectOption<string>[] = languages.languages.map((language) => ({
    value: language.value,
    label: language.chineseName,
  }));

  const fontOptions: SelectOption<string>[] = FONT_SIZE_RATIOS.map((ratio, index) => ({
    value: String(index),
    label: `${FONT_SIZE_LABELS[index] ?? String(index)}（${Math.round(ratio * 100)}%）`,
  }));

  return (
    <>
      <Section title="语言">
        <Row label="第一语言">
          <Select
            value={settings.firstLanguage}
            options={languageOptions}
            label="第一语言"
            onChange={(value) => void patch({ firstLanguage: value })}
          />
        </Row>
        <Row label="第二语言" description="与第一语言互换使用，用于自动判断翻译方向">
          <Select
            value={settings.secondLanguage}
            options={languageOptions}
            label="第二语言"
            onChange={(value) => void patch({ secondLanguage: value })}
          />
        </Row>
        <Row label="语言检测优化" description="服务端校正尚未接入；当前使用本地语言识别">
          <Select
            value={settings.languageDetectOptimize}
            options={DETECT_OPTIONS}
            label="语言检测优化"
            disabled
            onChange={(value) => void patch({ languageDetectOptimize: value })}
          />
        </Row>
      </Section>

      <Section title="外观">
        <Row label="外观">
          <Segmented
            value={settings.appearanceType}
            options={APPEARANCE_OPTIONS}
            label="外观"
            onChange={(value) => void patch({ appearanceType: value })}
          />
        </Row>
        <Row label="字体大小" description="翻译结果的字号">
          <Select
            value={String(settings.fontSizeOptionIndex)}
            options={fontOptions}
            label="字体大小"
            onChange={(value) => void patch({ fontSizeOptionIndex: Number(value) })}
          />
        </Row>
        <Row label="启用 Markdown 渲染" description="AI 服务的流式结果按 Markdown 排版">
          <Toggle
            checked={settings.enableMarkdownRendering}
            label="启用 Markdown 渲染"
            onChange={(value) => void patch({ enableMarkdownRendering: value })}
          />
        </Row>
      </Section>

      <Section title="自动查询">
        <Row label="自动查询选中文本" description="打开划词结果窗口后立即翻译；关闭时只填入原文，按 Enter 翻译">
          <Toggle
            checked={settings.autoQuerySelectedText}
            label="自动查询选中文本"
            onChange={(value) => void patch({ autoQuerySelectedText: value })}
          />
        </Row>
        <Row label="粘贴后自动翻译" description="粘贴到输入框时发起查询，不监听后台剪贴板">
          <Toggle
            checked={settings.autoQueryPastedText}
            label="粘贴后自动翻译"
            onChange={(value) => void patch({ autoQueryPastedText: value })}
          />
        </Row>
        <Row
          label="无法读取选区时改用复制"
          description="部分程序需模拟 Ctrl+C 读取选区并恢复剪贴板；无法完整备份、密码控件或焦点改变时会跳过。关闭后仅使用系统选区接口"
        >
          <Toggle
            checked={settings.enableForceGetSelectedText}
            label="无法读取选区时改用复制"
            onChange={(value) => void patch({ enableForceGetSelectedText: value })}
          />
        </Row>
        <Row label="自动查询 OCR 文本">
          <Toggle
            checked={settings.autoQueryOcrText}
            label="自动查询 OCR 文本"
            onChange={(value) => void patch({ autoQueryOcrText: value })}
          />
        </Row>
        <Row label="文本变化时自动查询" description="输入框内容变化后延迟发起查询">
          <Toggle
            checked={settings.autoQueryWhenTextChanged}
            label="文本变化时自动查询"
            onChange={(value) => void patch({ autoQueryWhenTextChanged: value })}
          />
        </Row>
      </Section>

      <Section title="自动复制">
        <Row label="自动复制选中文本">
          <Toggle
            checked={settings.autoCopySelectedText}
            label="自动复制选中文本"
            onChange={(value) => void patch({ autoCopySelectedText: value })}
          />
        </Row>
        <Row label="自动复制 OCR 文本">
          <Toggle
            checked={settings.autoCopyOcrText}
            label="自动复制 OCR 文本"
            onChange={(value) => void patch({ autoCopyOcrText: value })}
          />
        </Row>
        <Row label="自动复制第一个翻译结果">
          <Toggle
            checked={settings.autoCopyFirstTranslatedText}
            label="自动复制第一个翻译结果"
            onChange={(value) => void patch({ autoCopyFirstTranslatedText: value })}
          />
        </Row>
        <Row label="选中文本为空时保留上次结果">
          <Toggle
            checked={settings.keepPrevResultWhenSelectedTextIsEmpty}
            label="选中文本为空时保留上次结果"
            onChange={(value) =>
              void patch({ keepPrevResultWhenSelectedTextIsEmpty: value })
            }
          />
        </Row>
      </Section>

      <Section title="查询图标" description={
        desktop?.autoSelection.error
        ?? (settings.autoShowQueryIcon
          ? "拖选、双击/三击、Shift+点击或在编辑框按 Ctrl+A 后显示原生小图标。识别选区本身不联网；打开结果窗后才按设置查询。"
          : "监听已关闭，不在后台读取选区。")
      }>
        <Row label="自动显示查询图标" description="划词后弹出图标">
          <Toggle
            checked={settings.autoShowQueryIcon}
            label="自动显示查询图标"
            onChange={(value) => void patch({ autoShowQueryIcon: value })}
          />
        </Row>
        <Row label="点击图标打开" description="关闭时悬停即打开；未隐藏主窗口模式下始终悬停打开，与原版一致">
          <Toggle
            checked={settings.clickQuery}
            label="点击图标打开"
            onChange={(value) => void patch({ clickQuery: value })}
          />
        </Row>
        <Row label="最小文本长度" description="选中文本短于该长度时不显示图标">
          <Slider
            value={settings.autoShowQueryIconMinTextLength}
            min={0}
            max={50}
            step={1}
            label="最小文本长度"
            format={(value) => `${value} 字符`}
            onChange={(value) =>
              void patch({ autoShowQueryIconMinTextLength: Math.round(value) })
            }
          />
        </Row>
        <Row label="排除语言" description="选中该语言的文本时不显示图标">
          <Select
            value={settings.autoShowQueryIconExcludedLanguage}
            options={excludedOptions}
            label="排除语言"
            onChange={(value) => void patch({ autoShowQueryIconExcludedLanguage: value })}
          />
        </Row>
        <Row label="鼠标划词结果窗口">
          <Select
            value={settings.mouseSelectTranslateWindowType}
            options={[
              { value: "main", label: "主窗口" },
              { value: "mini", label: "迷你窗口" },
              { value: "fixed", label: "固定窗口" },
            ]}
            label="鼠标划词结果窗口"
            onChange={(value) => void patch({ mouseSelectTranslateWindowType: value })}
          />
        </Row>
      </Section>

      <Section title="启动与朗读">
        <Row label="开机自启动" description={
          desktop?.autostartError
          ?? (desktop?.autostartEnabled == null
            ? "正在读取 Windows 自启动状态…"
            : `Windows 当前${desktop.autostartEnabled ? "已开启" : "已关闭"}${
              desktop.autostartEnabled !== settings.launchAtStartup
                ? "，与上次保存的偏好不同；已保留系统选择，点击开关可重新设置。"
                : "；尊重任务管理器中的更改，不在启动时强行重写。"
            }`)
        }>
          <Toggle
            checked={desktop?.autostartEnabled ?? settings.launchAtStartup}
            disabled={desktop?.autostartEnabled == null}
            label="开机自启动"
            onChange={(value) => void patch({ launchAtStartup: value })}
          />
        </Row>
        <Row label="自动播放发音" description="查询完成后朗读原文">
          <Toggle
            checked={settings.autoPlayAudio}
            label="自动播放发音"
            onChange={(value) => void patch({ autoPlayAudio: value })}
          />
        </Row>
      </Section>
    </>
  );
}
