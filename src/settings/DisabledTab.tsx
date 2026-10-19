/**
 * 禁用 — the services the user switched off, with a one-click re-enable.
 *
 * "Disabled" here means either `enabled: false` (hidden from the windows) or a
 * usage status of 始终关闭, which keeps the service configured but never runs it.
 */

import { useState } from "react";

import * as api from "../api";
import { Banner, Icon, Section, errorMessage, serviceName, serviceMonogram } from "./components";
import type { ServiceListProps } from "./components";
import type { ServiceDescriptor } from "../types";

function isDisabled(service: ServiceDescriptor): boolean {
  return !service.config.enabled || service.config.usageStatus === "1";
}

function disabledReason(service: ServiceDescriptor): string {
  const reasons: string[] = [];
  if (!service.config.enabled) {
    reasons.push("已关闭");
  }
  if (service.config.usageStatus === "1") {
    reasons.push("使用状态为「始终关闭」");
  }
  return reasons.join("，");
}

export function DisabledTab({ services, reloadServices }: ServiceListProps) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const disabled = services.filter((service) => service.available && isDisabled(service));

  const enable = async (service: ServiceDescriptor) => {
    setBusy(service.identifier);
    setError(null);
    try {
      await api.updateServiceConfig(service.identifier, {
        enabled: true,
        usageStatus: "0",
      });
      await reloadServices();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const enableAll = async () => {
    setBusy("*");
    setError(null);
    try {
      for (const service of disabled) {
        await api.updateServiceConfig(service.identifier, {
          enabled: true,
          usageStatus: "0",
        });
      }
      await reloadServices();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      {error !== null && (
        <Banner kind="error" onDismiss={() => setError(null)}>
          {error}
        </Banner>
      )}

      <Section
        title="已禁用的服务"
        description="这些服务不会参与查询，点击「启用」即可恢复。"
        actions={
          disabled.length > 0 ? (
            <button
              type="button"
              className="ez-btn"
              disabled={busy !== null}
              onClick={() => void enableAll()}
            >
              <Icon name="check" size={12} />
              全部启用
            </button>
          ) : undefined
        }
      >
        {services.length === 0 ? (
          <div className="ezy-empty-block">正在读取服务列表…</div>
        ) : disabled.length === 0 ? (
          <div className="ezy-empty-block">没有被禁用的服务。</div>
        ) : (
          disabled.map((service) => (
            <div key={service.identifier} className="ezy-service-row">
              <span className="ezy-service-icon">{serviceMonogram(service)}</span>
              <div className="ezy-service-main">
                <span className="ezy-service-name">{serviceName(service)}</span>
                <span className="ezy-service-meta">
                  {service.identifier} · {disabledReason(service)}
                </span>
              </div>
              <div className="ezy-service-actions">
                <button
                  type="button"
                  className="ez-btn ez-btn-primary"
                  disabled={busy !== null}
                  onClick={() => void enable(service)}
                >
                  <Icon name="check" size={12} />
                  启用
                </button>
              </div>
            </div>
          ))
        )}
      </Section>
    </>
  );
}
