/**
 * The query engine.
 *
 * `query` resolves as soon as the fan-out is dispatched; the results arrive on
 * the `query-update` event, one per service. This hook keeps that stream in
 * order and exposes it to the result list.
 */

import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import * as api from "../api";
import { currentSettings, useSettings } from "./useSettings";
import { emptyQueryState, queryReducer } from "./queryState";
import type { ActionPayload, QueryRequest, QueryResult } from "../types";

export interface QueryEngine {
  /** Visible results, in the configured service order. */
  results: QueryResult[];
  /** Which service last produced a result, for the "just updated" highlight. */
  lastIdentifier: string | null;
  running: boolean;
  finished: boolean;
  lastRequest: QueryRequest | null;
  run: (request: QueryRequest) => Promise<void>;
  stop: () => Promise<void>;
  retry: () => Promise<void>;
  clear: () => void;
  /** Reveals a service whose card was auto-hidden. */
  reveal: (identifier: string) => Promise<void>;
}

export function useQuery(windowType: NonNullable<QueryRequest["windowType"]>): QueryEngine {
  const { settings } = useSettings();
  const [{ byIdentifier, lastIdentifier, running, finished }, dispatch] = useReducer(queryReducer, emptyQueryState);
  const lastRequest = useRef<QueryRequest | null>(null);
  const activeRequest = useRef<string | null>(null);
  const listenerReady = useRef<Promise<void>>(Promise.resolve());

  // Rebuild the ordering map whenever the settings change.
  const order = useMemo(() => {
    const serviceOrder = settings?.serviceOrder ?? [];
    const map: Record<string, number> = {};
    serviceOrder.forEach((identifier, index) => {
      map[identifier] = index;
    });
    return map;
  }, [settings?.serviceOrder]);

  useEffect(() => {
    let disposed = false;
    const subscriptions = [
      api.onQueryUpdate((payload) => {
        if (disposed || payload.requestId !== activeRequest.current) {
          return;
        }
        dispatch({ type: "update", payload });
      }),
      api.onQueryFinished(({ requestId, cancelled }) => {
        if (disposed || requestId !== activeRequest.current) {
          return;
        }
        dispatch({ type: "finish", payload: { requestId, cancelled } });
      }),
    ];
    listenerReady.current = Promise.all(subscriptions).then(() => undefined);
    void listenerReady.current.catch(() => undefined);

    return () => {
      disposed = true;
      for (const subscription of subscriptions) {
        void subscription.then((unlisten) => unlisten()).catch(() => undefined);
      }
    };
  }, []);

  const run = useCallback(async (request: QueryRequest) => {
    if (!request.text.trim()) {
      return;
    }
    lastRequest.current = request;
    const requestId = crypto.randomUUID();
    activeRequest.current = requestId;
    dispatch({ type: "start", requestId });
    try {
      await listenerReady.current;
      if (activeRequest.current !== requestId) return;
      await api.query({ ...request, windowType, requestId });
    } catch (error) {
      if (activeRequest.current !== requestId) return;
      activeRequest.current = null;
      dispatch({ type: "stop" });
      throw error;
    }
  }, [windowType]);

  const stop = useCallback(async () => {
    const requestId = activeRequest.current;
    activeRequest.current = null;
    dispatch({ type: "stop" });
    if (requestId) await api.stopQuery(windowType, requestId);
  }, [windowType]);

  const retry = useCallback(async () => {
    if (lastRequest.current) {
      await run(lastRequest.current);
    }
  }, [run]);

  const clear = useCallback(() => {
    const requestId = activeRequest.current;
    activeRequest.current = null;
    if (requestId) void api.stopQuery(windowType, requestId).catch(() => undefined);
    dispatch({ type: "clear" });
    lastRequest.current = null;
  }, [windowType]);

  const reveal = useCallback(async (identifier: string) => {
    await api.showServiceResult(identifier, windowType);
    dispatch({ type: "reveal", identifier });
  }, [windowType]);

  const visible = Object.values(byIdentifier)
    .filter((result) => result.isShowing)
    .sort((a, b) => {
      const left = order[a.serviceTypeWithUniqueIdentifier] ?? Number.MAX_SAFE_INTEGER;
      const right = order[b.serviceTypeWithUniqueIdentifier] ?? Number.MAX_SAFE_INTEGER;
      if (left !== right) {
        return left - right;
      }
      return a.serviceTypeWithUniqueIdentifier.localeCompare(
        b.serviceTypeWithUniqueIdentifier,
      );
    });

  return {
    results: visible,
    lastIdentifier,
    running,
    finished,
    lastRequest: lastRequest.current,
    run,
    stop,
    retry,
    clear,
    reveal,
  };
}

/**
 * Drains this window's native inbox once its settings and handlers are ready.
 */
export function useActions(handler: (payload: ActionPayload) => void | Promise<void>) {
  const ref = useRef(handler);
  ref.current = handler;

  useEffect(() => {
    let active = true;
    const callback = (payload: ActionPayload) => ref.current(payload);
    let detach = api.onAction(callback);
    const focus = getCurrentWindow().onFocusChanged(({ payload }) => {
      if (active && payload) {
        detach();
        detach = api.onAction(callback);
      }
    });
    return () => {
      active = false;
      detach();
      void focus.then((unlisten) => unlisten()).catch(api.reportActionError);
    };
  }, []);
}

/** The default target language for a detected source, mirroring the Rust rule. */
export function defaultTarget(from: string): string {
  const settings = currentSettings();
  if (!settings) {
    return "Simplified-Chinese";
  }
  return from === settings.firstLanguage
    ? settings.secondLanguage
    : settings.firstLanguage;
}
