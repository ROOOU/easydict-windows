/**
 * The settings window.
 *
 * One level of state: which tab is showing and the service registry, both of
 * which every tab may need. The settings document itself lives in the
 * `useSettings` store, so a write from any tab is immediately visible to the
 * others — and to the query windows on their next focus.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import * as api from "../api";
import { useAppearance } from "../hooks/useAppearance";
import { useSettings } from "../hooks/useSettings";
import { useActions } from "../hooks/useQuery";
import type { DeepPartial, DesktopStatus, ServiceDescriptor, Settings } from "../types";
import { AboutTab } from "./AboutTab";
import { AdvancedTab } from "./AdvancedTab";
import { Banner, errorMessage } from "./components";
import { DisabledTab } from "./DisabledTab";
import { FavoritesTab } from "./FavoritesTab";
import { GeneralTab } from "./GeneralTab";
import { PrivacyTab } from "./PrivacyTab";
import { ServiceTab } from "./ServiceTab";
import { ShortcutTab } from "./ShortcutTab";
import { attachScrollPane, resetScroll } from "./scroll";
import "./settings.css";

/** The tabs, in the order the macOS toolbar lists them. */
const TABS = [
  { id: "general", label: "通用" },
  { id: "service", label: "服务" },
  { id: "favorites", label: "收藏" },
  { id: "disabled", label: "禁用" },
  { id: "shortcut", label: "快捷键" },
  { id: "advanced", label: "高级" },
  { id: "privacy", label: "隐私" },
  { id: "about", label: "关于" },
] as const;

type TabId = (typeof TABS)[number]["id"];

export function SettingsApp() {
  const { settings, patch: savePatch, reload } = useSettings();
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [desktop, setDesktop] = useState<DesktopStatus | null>(null);
  const [desktopError, setDesktopError] = useState<string | null>(null);
  const [desktopRevision, setDesktopRevision] = useState(0);
  const [tab, setTab] = useState<TabId>("general");
  const [services, setServices] = useState<ServiceDescriptor[]>([]);
  const [servicesError, setServicesError] = useState<string | null>(null);
  const content = useRef<HTMLDivElement>(null);

  // The tab content is swapped inside one scroller, so a new tab would open at
  // the previous tab's offset. The reset runs in a layout effect, before the
  // frame is painted: doing it after paint shows the new page scrolled down for
  // one frame. Ref callbacks run before layout effects, so the pane is attached
  // by the time this asks for the top.
  useLayoutEffect(() => {
    attachScrollPane(content.current);
    resetScroll();
    // Detach on unmount so a late reset from another tab cannot write to a node
    // that is gone.
    return () => attachScrollPane(null);
  }, [tab]);

  const patch = async (value: DeepPartial<Settings>) => {
    try {
      await savePatch(value);
      setSettingsError(null);
    } catch (error) {
      setSettingsError(errorMessage(error));
    } finally {
      setDesktopRevision((revision) => revision + 1);
    }
  };

  // useSettings also refreshes on focus, so external Windows changes show up
  // without reapplying the stored preference to the operating system.
  useEffect(() => {
    let active = true;
    void api.getDesktopStatus().then((status) => {
      if (active) {
        setDesktop(status);
        setDesktopError(null);
      }
    }).catch((error) => {
      if (active) {
        setDesktop(null);
        setDesktopError(errorMessage(error));
      }
    });
    return () => { active = false; };
  }, [settings, desktopRevision, tab]);

  useEffect(() => {
    let active = true;
    const subscription = api.onDesktopStatusChanged(() => {
      if (active) setDesktopRevision((revision) => revision + 1);
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  useActions(({ action }) => {
    if (action === "history") setTab("advanced");
    else if (TABS.some((entry) => entry.id === action)) setTab(action as TabId);
  });

  // Keeps `data-theme` in step with the appearance picker in the General tab.
  useAppearance();

  const reloadServices = useCallback(async () => {
    try {
      setServices(await api.listServices());
      setServicesError(null);
    } catch (error) {
      setServicesError(errorMessage(error));
    }
  }, []);

  useEffect(() => {
    void reloadServices();
  }, [reloadServices]);

  const body = (() => {
    if (!settings) {
      return (
        <div className="ezy-loading">
          <span className="ez-spinner" />
          正在加载设置…
        </div>
      );
    }
    switch (tab) {
      case "service":
        return (
          <ServiceTab
            settings={settings}
            patch={patch}
            reload={reload}
            services={services}
            reloadServices={reloadServices}
          />
        );
      case "favorites":
        return <FavoritesTab />;
      case "disabled":
        return <DisabledTab services={services} reloadServices={reloadServices} />;
      case "shortcut":
        return <ShortcutTab settings={settings} patch={patch} desktop={desktop} />;
      case "advanced":
        return (
          <AdvancedTab
            settings={settings}
            patch={patch}
            reload={reload}
            services={services}
            reloadServices={reloadServices}
          />
        );
      case "privacy":
        return <PrivacyTab />;
      case "about":
        return <AboutTab />;
      default:
        return <GeneralTab settings={settings} patch={patch} desktop={desktop} />;
    }
  })();

  return (
    <div className="ezy-root">
      <div className="ezy-toolbar" role="tablist" aria-label="设置">
        {TABS.map((entry) => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            aria-selected={tab === entry.id}
            className="ezy-tab"
            onClick={() => setTab(entry.id)}
          >
            {entry.label}
          </button>
        ))}
      </div>
      <div className="ez-scroll ezy-content" ref={content}>
        {settingsError !== null && (
          <Banner kind="error" onDismiss={() => setSettingsError(null)}>
            保存失败：{settingsError}
          </Banner>
        )}
        {servicesError !== null && (
          <Banner kind="error" onDismiss={() => setServicesError(null)}>
            读取服务列表失败：{servicesError}
          </Banner>
        )}
        {desktopError !== null && (
          <Banner kind="error">读取 Windows 集成状态失败：{desktopError}</Banner>
        )}
        {body}
      </div>
    </div>
  );
}
