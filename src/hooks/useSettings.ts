/**
 * The settings store.
 *
 * Every window needs the settings, and the settings window writes them, so a
 * single module-level cache with subscribers keeps the five windows coherent
 * without a state library. Writes go through `patch`, which merges on the Rust
 * side and broadcasts settings changes to every window. Focus refreshes share
 * one listener and one in-flight read per window.
 */

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import * as api from "../api";
import type { DeepPartial, Settings } from "../types";

let cache: Settings | null = null;
let inflight: Promise<Settings> | null = null;
const listeners = new Set<(settings: Settings) => void>();
let detach: (() => void) | null = null;
let revision = 0;

function publish(settings: Settings) {
  revision += 1;
  cache = settings;
  for (const listener of listeners) {
    listener(settings);
  }
}

export function currentSettings(): Settings | null {
  return cache;
}

export function subscribeSettings(listener: (settings: Settings) => void): () => void {
  listeners.add(listener);
  if (!detach) {
    let active = true;
    const subscriptions = [
      api.onSettingsChanged((settings) => { if (active) publish(settings); }),
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (active && focused) void loadSettings(true).catch(() => undefined);
      }),
    ];
    detach = () => {
      active = false;
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      }
    };
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      detach?.();
      detach = null;
    }
  };
}

export async function loadSettings(force = false): Promise<Settings> {
  if (cache && !force) {
    return cache;
  }
  if (!inflight) {
    const startedAt = revision;
    inflight = api
      .getSettings()
      .then((settings) => {
        if (startedAt === revision) publish(settings);
        return cache ?? settings;
      })
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

export async function patchSettings(
  patch: DeepPartial<Settings>,
): Promise<Settings> {
  const settings = await api.updateSettings(patch);
  publish(settings);
  return settings;
}

export async function replaceSettings(settings: Settings): Promise<Settings> {
  publish(settings);
  return settings;
}

/** Re-reads the settings whenever this window regains focus. */
export function useSettings(): {
  settings: Settings | null;
  error: string | null;
  patch: (patch: DeepPartial<Settings>) => Promise<void>;
  reload: () => Promise<void>;
} {
  const [settings, setSettings] = useState<Settings | null>(cache);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const unsubscribe = subscribeSettings((settings) => {
      setSettings(settings);
      setError(null);
    });
    void loadSettings().catch((error) => { if (active) setError(String(error)); });
    return () => { active = false; unsubscribe(); };
    // Subscribing once is enough; the store handles refreshes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    settings,
    error,
    patch: async (patch) => {
      await patchSettings(patch);
    },
    reload: async () => {
      await loadSettings(true);
    },
  };
}
