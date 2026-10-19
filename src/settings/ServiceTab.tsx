/**
 * 服务 — the registry list and the entry point to each service's config.
 *
 * The list is grouped by availability: everything that runs on Windows (in the
 * user's result order) first, then the entries kept for settings-file
 * round-tripping that this port cannot build.
 */

import { useLayoutEffect, useMemo, useState } from "react";

import * as api from "../api";
import { resetScroll } from "./scroll";
import {
  Banner,
  Icon,
  QUERY_TYPE_DICTIONARY,
  QUERY_TYPE_SENTENCE,
  QUERY_TYPE_TRANSLATION,
  Section,
  Toggle,
  errorMessage,
  serviceName,
  serviceMonogram,
  sortByServiceOrder,
  type ServiceTabProps,
} from "./components";
import { ServiceDetail } from "./ServiceDetail";
import type { ServiceDescriptor } from "../types";

/** Why a descriptor is not usable here, keyed by the Rust `nameKey`. */
const UNAVAILABLE_NOTES: Record<string, string> = {
  apple_dictionary: "依赖 macOS 专有框架 DictionaryServices，Windows 版不可用。",
  apple_translate: "依赖 macOS 系统翻译面板，Windows 版不可用。",
  "service.mdict.name": "本地 MDict 词典在 Windows 版中尚未实现。",
  "service.claude_code.name": "Agent CLI 服务在 Windows 版中尚未实现。",
  "service.codex_cli.name": "Agent CLI 服务在 Windows 版中尚未实现。",
  "service.github_copilot.name": "Agent CLI 服务在 Windows 版中尚未实现。",
};

function unavailableNote(service: ServiceDescriptor): string {
  return UNAVAILABLE_NOTES[service.nameKey] ?? "该服务在当前版本中不可用。";
}

function capabilityLabels(service: ServiceDescriptor): string[] {
  const labels: string[] = [];
  if ((service.supportedQueryType & QUERY_TYPE_TRANSLATION) !== 0) {
    labels.push("翻译");
  }
  if ((service.supportedQueryType & QUERY_TYPE_DICTIONARY) !== 0) {
    labels.push("词典");
  }
  if ((service.supportedQueryType & QUERY_TYPE_SENTENCE) !== 0) {
    labels.push("句子");
  }
  return labels;
}

/** `CustomOpenAI#<uuid>` — the uuid the Rust side splits identifiers on. */
function newInstanceIdentifier(): string {
  const webCrypto = globalThis.crypto;
  if (webCrypto && typeof webCrypto.randomUUID === "function") {
    return webCrypto.randomUUID().replace(/-/g, "");
  }
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

export function ServiceTab({ settings, patch, services, reloadServices }: ServiceTabProps) {
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Opening a service and going back are two different pages of this tab, so
  // each one starts at the top rather than at the list row that was clicked.
  // A layout effect resets before the frame is painted, so the new page is
  // never shown scrolled down.
  useLayoutEffect(() => {
    resetScroll();
  }, [selected]);

  const ordered = useMemo(
    () => sortByServiceOrder(services, settings.serviceOrder),
    [services, settings.serviceOrder],
  );
  const available = ordered.filter((service) => service.available);
  const unavailable = ordered.filter((service) => !service.available);

  const current =
    selected === null
      ? null
      : (services.find((service) => service.identifier === selected) ?? null);

  const setEnabled = async (service: ServiceDescriptor, enabled: boolean) => {
    setBusy(service.identifier);
    setError(null);
    try {
      await api.updateServiceConfig(service.identifier, { enabled });
      await reloadServices();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const addInstance = async (service: ServiceDescriptor) => {
    setBusy(service.identifier);
    setError(null);
    try {
      const identifier = `${service.serviceType}#${newInstanceIdentifier()}`;
      const existing = services.filter((entry) =>
        entry.identifier.startsWith(`${service.serviceType}#`),
      ).length;
      await api.updateServiceConfig(identifier, {
        name: `${serviceName(service)} ${existing + 1}`,
        enabled: true,
      });
      await reloadServices();
      setSelected(identifier);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (current !== null) {
    return (
      <ServiceDetail
        service={current}
        settings={settings}
        patch={patch}
        onBack={() => setSelected(null)}
        onChanged={reloadServices}
      />
    );
  }

  return (
    <>
      {error !== null && (
        <Banner kind="error" onDismiss={() => setError(null)}>
          {error}
        </Banner>
      )}

      <Section
        title="可用服务"
        description="点击一行可配置该服务；开关控制它是否参与查询。"
      >
        {available.length === 0 ? (
          <div className="ezy-empty-block">正在读取服务列表…</div>
        ) : (
          available.map((service) => (
            <div
              key={service.identifier}
              className="ezy-service-row"
              role="button"
              tabIndex={0}
              onClick={() => setSelected(service.identifier)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setSelected(service.identifier);
                }
              }}
            >
              <span className="ezy-service-icon">{serviceMonogram(service)}</span>
              <div className="ezy-service-main">
                <span className="ezy-service-name">
                  {serviceName(service)}
                  {service.isStream && <span className="ez-badge">流式</span>}
                  {service.requiresApiKey && !service.hasApiKey && (
                    <span className="ez-badge ez-badge-error">未填写密钥</span>
                  )}
                  {!service.config.enabled && <span className="ez-badge">已禁用</span>}
                </span>
                <span className="ezy-service-meta">
                  {service.identifier}
                  {capabilityLabels(service).length > 0 &&
                    ` · ${capabilityLabels(service).join(" / ")}`}
                </span>
              </div>
              <div
                className="ezy-service-actions"
                onClick={(event) => event.stopPropagation()}
                onKeyDown={(event) => event.stopPropagation()}
              >
                {service.allowsMultipleInstances && (
                  <button
                    type="button"
                    className="ez-btn"
                    disabled={busy === service.identifier}
                    onClick={() => void addInstance(service)}
                  >
                    <Icon name="plus" size={12} />
                    添加实例
                  </button>
                )}
                <Toggle
                  checked={service.config.enabled}
                  disabled={busy === service.identifier}
                  label={`启用 ${serviceName(service)}`}
                  onChange={(value) => void setEnabled(service, value)}
                />
                <Icon name="chevron" size={12} className="ezy-service-chevron" />
              </div>
            </div>
          ))
        )}
      </Section>

      {unavailable.length > 0 && (
        <Section title="不可用服务" description="这些服务没有 Windows 实现，仅为兼容旧设置文件保留。">
          {unavailable.map((service) => (
            <div key={service.identifier} className="ezy-service-row" data-unavailable="true">
              <span className="ezy-service-icon">{serviceMonogram(service)}</span>
              <div className="ezy-service-main">
                <span className="ezy-service-name">{serviceName(service)}</span>
                <span className="ezy-service-meta">{unavailableNote(service)}</span>
              </div>
              <div className="ezy-service-actions">
                {service.config.enabled && <span className="ez-badge">已启用</span>}
              </div>
            </div>
          ))}
        </Section>
      )}
    </>
  );
}
