/**
 * 关于 — name, version, data directory and the project links.
 *
 * `appInfo()` is the only place the version comes from; the Rust side writes
 * `CARGO_PKG_VERSION` into it, so this tab never drifts from the build.
 */

import { useEffect, useState } from "react";

import * as api from "../api";
import type { AppInfo } from "../types";
import { Banner, Icon, Row, Section, errorMessage } from "./components";

const LINKS: ReadonlyArray<{ label: string; url: string }> = [
  { label: "项目主页", url: "https://github.com/tisfeng/Easydict" },
  { label: "使用文档", url: "https://github.com/tisfeng/Easydict/blob/main/docs/README.md" },
  { label: "问题反馈", url: "https://github.com/tisfeng/Easydict/issues" },
  { label: "许可证 (GPL-3.0)", url: "https://github.com/tisfeng/Easydict/blob/main/LICENSE" },
];

export function AboutTab() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let active = true;
    void api
      .appInfo()
      .then((value) => {
        if (active) {
          setInfo(value);
        }
      })
      .catch((cause: unknown) => {
        if (active) {
          setError(errorMessage(cause));
        }
      });
    return () => {
      active = false;
    };
  }, []);

  const copyDataDir = async () => {
    if (info === null) {
      return;
    }
    try {
      await api.setClipboardText(info.dataDir);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <>
      {error !== null && (
        <Banner kind="error" onDismiss={() => setError(null)}>
          {error}
        </Banner>
      )}

      <Section>
        <div className="ezy-detail-header" style={{ marginBottom: 0 }}>
          <span className="ezy-service-icon" style={{ width: 40, height: 40, fontSize: 18 }}>
            E
          </span>
          <div className="ezy-detail-heading">
            <span className="ezy-detail-title">{info?.name ?? "Easydict"}</span>
            <span className="ezy-detail-sub">
              版本 {info?.version ?? "…"} · {info?.platform ?? "windows"}
            </span>
          </div>
        </div>
      </Section>

      <Section title="数据目录" description="设置文件、历史记录与本地词典都存放在这里。">
        <Row label="数据目录" stacked>
          <div className="ezy-inline">
            <span className="ezy-detail-sub ez-selectable" style={{ flex: "1 1 auto" }}>
              {info?.dataDir ?? "正在读取…"}
            </span>
            <button
              type="button"
              className="ez-btn"
              disabled={info === null}
              onClick={() => void copyDataDir()}
            >
              <Icon name="copy" size={12} />
              {copied ? "已复制" : "复制路径"}
            </button>
          </div>
        </Row>
      </Section>

      <Section title="链接">
        {LINKS.map((link) => (
          <Row key={link.url} label={link.label}>
            <button
              type="button"
              className="ez-btn"
              onClick={() => void api.openLink(link.url)}
            >
              <Icon name="external" size={12} />
              打开
            </button>
          </Row>
        ))}
      </Section>

      <Section title="致谢">
        <Row
          label="Easydict"
          description="本项目以 macOS 版 Easydict（GPL-3.0）为蓝本重写，感谢原作者与贡献者。"
        >
          <span className="ez-badge">GPL-3.0</span>
        </Row>
      </Section>
    </>
  );
}
