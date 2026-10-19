/**
 * 高级 — history, service ordering, OCR, the HTTP server and settings transfer.
 *
 * The history "size" is whatever the store currently holds in SQLite, so the tab shows the live count from
 * `api.historyCount()` next to the button that empties it.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { documentDir, isAbsolute, join } from "@tauri-apps/api/path";

import * as api from "../api";
import { useLanguages } from "../hooks/useLanguages";
import type { QueryRecord, Settings, ShowWindowPosition, WindowType } from "../types";
import { windowConfigurationValue } from "../hooks/windowSettings";
import {
  Banner,
  Icon,
  Row,
  Section,
  Select,
  TextField,
  Toggle,
  errorMessage,
  formatTimestamp,
  serviceName,
  serviceMonogram,
  sortByServiceOrder,
  type AdvancedTabProps,
  type SelectOption,
} from "./components";

/** How many history entries the tab lists at a time. */
const HISTORY_PAGE = 20;

async function transferPath(path: string, fallback = ""): Promise<string> {
  const value = path.trim() || fallback;
  return value && await isAbsolute(value) ? value : join(await documentDir(), value);
}

/** `EZShowWindowPosition` (`EZEnumTypes.h:24`), in its raw-value order. */
const WINDOW_POSITION_OPTIONS: readonly SelectOption<ShowWindowPosition>[] = [
  { value: "right", label: "屏幕右侧" },
  { value: "mouse", label: "鼠标位置" },
  { value: "former", label: "上次位置" },
  { value: "center", label: "屏幕中间" },
];

/**
 * `WindowConfigurationView.swift` — every query window switches its own input
 * field and language row on and off. `EZLayoutManager.m:119/136` reads the same
 * two keys before it gives the input view any height, so with the input field
 * off the window really does start smaller.
 */
const WINDOW_CONFIGURATION: readonly {
  kind: WindowType;
  label: string;
  description: string;
  inputField: keyof Settings;
  selectLanguage: keyof Settings;
}[] = [
  {
    kind: "main",
    label: "主窗口",
    description: "托盘图标、快捷键打开的固定窗口",
    inputField: "inputFieldCellVisibleMain",
    selectLanguage: "selectLanguageCellVisibleMain",
  },
  {
    kind: "mini",
    label: "迷你窗口",
    description: "选中文字后弹出的小窗口",
    inputField: "inputFieldCellVisibleMini",
    selectLanguage: "selectLanguageCellVisibleMini",
  },
  {
    kind: "fixed",
    label: "侧悬浮窗口",
    description: "贴屏幕右侧显示的翻译窗口",
    inputField: "inputFieldCellVisibleFixed",
    selectLanguage: "selectLanguageCellVisibleFixed",
  },
];

export function AdvancedTab({
  settings,
  patch,
  reload,
  services,
  reloadServices,
}: AdvancedTabProps) {
  const languages = useLanguages();
  const [historyCount, setHistoryCount] = useState<number | null>(null);
  const [records, setRecords] = useState<QueryRecord[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [path, setPath] = useState("");
  const [transferBusy, setTransferBusy] = useState(false);
  const [includeSecrets, setIncludeSecrets] = useState(false);

  const [order, setOrder] = useState<string[]>([]);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropIndex, setDropIndex] = useState<number | null>(null);

  const ordered = useMemo(
    () => sortByServiceOrder(services, settings.serviceOrder),
    [services, settings.serviceOrder],
  );
  const draggable = useMemo(
    () => ordered.filter((service) => service.available),
    [ordered],
  );

  // The local list follows the stored order until the user starts dragging.
  useEffect(() => {
    setOrder(draggable.map((service) => service.identifier));
  }, [draggable]);

  const refreshCount = useCallback(async () => {
    try {
      const [count, recent] = await Promise.all([
        api.historyCount(),
        api.history(HISTORY_PAGE, 0),
      ]);
      setHistoryCount(count);
      setRecords(recent);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, []);

  useEffect(() => {
    void refreshCount();
  }, [refreshCount]);

  const removeRecord = async (record: QueryRecord) => {
    try {
      await api.deleteHistory(record);
      await refreshCount();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const clearHistory = async () => {
    try {
      await api.clearHistory();
      setConfirmClear(false);
      setMessage("历史记录已清空。");
      await refreshCount();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const persistOrder = async (next: string[]) => {
    try {
      // Identifiers that are not draggable (the unavailable ones) keep their
      // relative position at the end of the list.
      const rest = ordered
        .filter((service) => !service.available)
        .map((service) => service.identifier);
      await api.setServiceOrder([...next, ...rest]);
      await reloadServices();
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const move = (from: number, to: number) => {
    if (from === to || to < 0 || to >= order.length) {
      return;
    }
    const next = [...order];
    const [moved] = next.splice(from, 1);
    if (moved === undefined) {
      return;
    }
    next.splice(to, 0, moved);
    setOrder(next);
    void persistOrder(next);
  };

  const resetOrder = () => {
    const next = draggable.map((service) => service.identifier);
    setOrder(next);
    void persistOrder(next);
  };

  const exportTo = async () => {
    setTransferBusy(true);
    setError(null);
    setMessage(null);
    try {
      if (includeSecrets && !await confirm(
        "导出文件将包含明文 API Key、Cookie 和服务凭据。任何得到文件的人都能读取它们。确定继续？",
        { title: "导出敏感设置", kind: "warning" },
      )) return;
      const selected = await save({
        title: "导出设置",
        defaultPath: await transferPath(path, "easydict-settings.json"),
        filters: [{ name: "JSON 设置", extensions: ["json"] }],
      });
      if (!selected) return;
      setPath(selected);
      await api.exportSettings(selected, includeSecrets);
      setMessage(`设置已导出到 ${selected}${includeSecrets ? "（包含明文凭据，请妥善保管）" : "（不含 API Key、Cookie 和服务凭据）"}`);
      setIncludeSecrets(false);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setTransferBusy(false);
    }
  };

  const importFrom = async () => {
    setTransferBusy(true);
    setError(null);
    setMessage(null);
    try {
      const selected = await open({
        title: "导入设置",
        defaultPath: await transferPath(path),
        multiple: false,
        directory: false,
        filters: [{ name: "JSON 设置", extensions: ["json"] }],
      });
      if (!selected) return;
      if (!await confirm(
        "导入会替换当前设置和服务凭据。不含密钥的导出文件会清空已有密钥；历史和收藏不受影响。确定继续？",
        { title: "导入设置", kind: "warning" },
      )) return;
      setPath(selected);
      await api.importSettings(selected);
      await reload();
      await reloadServices();
      setMessage("设置已导入。");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setTransferBusy(false);
    }
  };

  return (
    <>
      {error !== null && (
        <Banner kind="error" onDismiss={() => setError(null)}>
          {error}
        </Banner>
      )}
      {message !== null && (
        <Banner kind="success" onDismiss={() => setMessage(null)}>
          {message}
        </Banner>
      )}

      <Section
        title="历史记录"
        description="历史记录保存在本机数据库中，最多显示最近 20 条。"
        actions={
          <button type="button" className="ez-btn" onClick={() => void refreshCount()}>
            <Icon name="refresh" size={12} />
            刷新
          </button>
        }
      >
        <Row
          label="历史记录数量"
          description={
            historyCount === null ? "正在读取…" : `当前保存了 ${historyCount} 条查询记录`
          }
        >
          <div className="ezy-inline">
            {confirmClear ? (
              <>
                <span className="ezy-row-description">确认清空全部历史记录？</span>
                <button
                  type="button"
                  className="ez-btn ez-btn-primary"
                  onClick={() => void clearHistory()}
                >
                  确认清空
                </button>
                <button
                  type="button"
                  className="ez-btn"
                  onClick={() => setConfirmClear(false)}
                >
                  取消
                </button>
              </>
            ) : (
              <button
                type="button"
                className="ez-btn"
                onClick={() => setConfirmClear(true)}
              >
                <Icon name="trash" size={12} />
                清空历史记录
              </button>
            )}
          </div>
        </Row>
        <Row label="最近查询" stacked>
          <div className="ezy-history">
            {records === null ? (
              <span className="ezy-row-description">正在读取…</span>
            ) : records.length === 0 ? (
              <span className="ezy-row-description">还没有查询记录。</span>
            ) : (
              records.map((record) => (
                <div
                  key={`${record.timestamp}|${record.queryText}`}
                  className="ezy-history-row"
                >
                  <div className="ezy-record-main">
                    <span className="ezy-record-text ez-selectable">
                      {record.queryText}
                    </span>
                    <span className="ezy-record-meta">
                      {languages.name(record.fromLanguage)} →{" "}
                      {languages.name(record.toLanguage)}
                      {" · "}
                      {formatTimestamp(record.timestamp)}
                    </span>
                  </div>
                  <button
                    type="button"
                    className="ez-icon-btn"
                    aria-label="删除这条记录"
                    title="删除"
                    onClick={() => void removeRecord(record)}
                  >
                    <Icon name="trash" size={13} />
                  </button>
                </div>
              ))
            )}
          </div>
        </Row>
      </Section>

      <Section
        title="服务排序"
        description="拖动调整服务在结果列表中的顺序，顺序会立即保存。"
        actions={
          <button type="button" className="ez-btn" onClick={resetOrder}>
            <Icon name="refresh" size={12} />
            重置顺序
          </button>
        }
      >
        {order.length === 0 ? (
          <div className="ezy-empty-block">正在读取服务列表…</div>
        ) : (
          order.map((identifier, index) => {
            const service = draggable[index];
            return (
              <div
                key={identifier}
                className="ezy-order-row"
                draggable
                data-dragging={dragId === identifier ? "true" : undefined}
                data-drop-target={
                  dropIndex === index && dragId !== identifier ? "true" : undefined
                }
                onDragStart={(event) => {
                  setDragId(identifier);
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", identifier);
                }}
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  setDropIndex(index);
                }}
                onDragLeave={() => {
                  setDropIndex((current) => (current === index ? null : current));
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  const from = order.indexOf(dragId ?? "");
                  setDropIndex(null);
                  setDragId(null);
                  if (from >= 0) {
                    move(from, index);
                  }
                }}
                onDragEnd={() => {
                  setDragId(null);
                  setDropIndex(null);
                }}
              >
                <span className="ezy-order-handle">
                  <Icon name="grip" size={14} />
                </span>
                <span className="ezy-order-index">{index + 1}</span>
                <span className="ezy-service-icon">
                  {service === undefined ? "?" : serviceMonogram(service)}
                </span>
                <div className="ezy-service-main">
                  <span className="ezy-service-name">
                    {service === undefined ? identifier : serviceName(service)}
                  </span>
                  <span className="ezy-service-meta">{identifier}</span>
                </div>
              </div>
            );
          })
        )}
      </Section>

      <Section title="OCR">
        <Row label="显示 OCR 菜单项">
          <Toggle
            checked={settings.showOcrMenuItems}
            label="显示 OCR 菜单项"
            onChange={(value) => void patch({ showOcrMenuItems: value })}
          />
        </Row>
        <Row label="启用有道 OCR" description="尚未接入；当前仅使用本机 Windows OCR，不上传截图">
          <Toggle
            checked={false}
            disabled
            label="启用有道 OCR"
            onChange={(value) => void patch({ enableYoudaoOcr: value })}
          />
        </Row>
      </Section>

      <Section title="窗口">
        {/* Both pickers used to be missing entirely, which left the settings
            readable but unsettable — the two floating windows then always took
            the `Right` default, so the mini window's own `Mouse` default could
            never apply. `AdvancedTab.swift:321/335` places them here. */}
        <Row label="侧悬浮窗口位置" description="固定窗口每次显示时的位置">
          <Select
            value={settings.fixedWindowPosition}
            options={WINDOW_POSITION_OPTIONS}
            label="侧悬浮窗口位置"
            onChange={(value) => void patch({ fixedWindowPosition: value })}
          />
        </Row>
        <Row label="迷你窗口位置" description="选中文字后弹出的小窗口的位置">
          <Select
            value={settings.miniWindowPosition}
            options={WINDOW_POSITION_OPTIONS}
            label="迷你窗口位置"
            onChange={(value) => void patch({ miniWindowPosition: value })}
          />
        </Row>
        <Row
          label="隐藏主窗口"
          description="启动和日常使用时只保留托盘；关闭后，划词图标始终悬停打开，并可从托盘唤回主窗口"
        >
          <Toggle
            checked={settings.hideMainWindow}
            label="隐藏主窗口"
            onChange={(value) => void patch({ hideMainWindow: value })}
          />
        </Row>
      </Section>

      <Section
        title="逐窗口配置"
        description="输入框和语言栏独立显示。智能查询按单词、英文句子或普通文本筛选服务，默认关闭；服务详情可调整参与类型。"
      >
        {WINDOW_CONFIGURATION.map(({ kind, label, description, inputField, selectLanguage }) => (
          <Row key={kind} label={label} description={description}>
            <div className="ezy-inline-switches">
              <label className="ezy-inline-switch">
                <span>输入框</span>
                <Toggle
                  checked={windowConfigurationValue(settings, kind, "inputField")}
                  label={`${label}显示输入框`}
                  onChange={(value) => void patch({ [inputField]: value })}
                />
              </label>
              <label className="ezy-inline-switch">
                <span>语言栏</span>
                <Toggle
                  checked={windowConfigurationValue(settings, kind, "selectLanguage")}
                  label={`${label}显示语言栏`}
                  onChange={(value) => void patch({ [selectLanguage]: value })}
                />
              </label>
              <label className="ezy-inline-switch">
                <span>智能查询</span>
                <Toggle
                  checked={settings.intelligentQueryMode?.[kind] === true}
                  label={`${label}智能查询`}
                  onChange={(value) => void patch({ intelligentQueryMode: { [kind]: value } })}
                />
              </label>
            </div>
          </Row>
        ))}
      </Section>

      <Section title="HTTP 服务">
        <Row label="启用 HTTP 服务" description="Windows 版尚未实现本地 HTTP 接口，不会监听端口">
          <Toggle
            checked={false}
            disabled
            label="启用 HTTP 服务"
            onChange={(value) => void patch({ enableHttpServer: value })}
          />
        </Row>
        <Row label="端口">
          <TextField
            value={settings.httpPort}
            disabled
            placeholder="12345"
            label="端口"
            monospace
            width={120}
            onChange={(value) => void patch({ httpPort: value })}
          />
        </Row>
      </Section>

      <Section
        title="导入 / 导出设置"
        description="使用 Windows 文件选择器导入或导出。默认导出不含服务凭据；自定义地址与提示词仍会保留，分享前请检查。"
      >
        <Row label="设置文件路径" stacked>
          <TextField
            value={path}
            placeholder="C:\\Users\\you\\easydict-settings.json"
            label="设置文件路径"
            monospace
            onChange={setPath}
            delay={0}
          />
        </Row>
        <Row label="导出时包含密钥" description="仅在迁移到自己的设备时开启；导出文件不加密，不能公开分享。">
          <Toggle
            checked={includeSecrets}
            disabled={transferBusy}
            label="导出时包含密钥"
            onChange={setIncludeSecrets}
          />
        </Row>
        <Row label="操作">
          <div className="ezy-inline">
            <button
              type="button"
              className="ez-btn"
              disabled={transferBusy}
              onClick={() => void exportTo()}
            >
              <Icon name="folder" size={12} />
              导出设置
            </button>
            <button
              type="button"
              className="ez-btn"
              disabled={transferBusy}
              onClick={() => void importFrom()}
            >
              <Icon name="folder" size={12} />
              导入设置
            </button>
          </div>
        </Row>
      </Section>
    </>
  );
}
