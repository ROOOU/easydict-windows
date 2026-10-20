import type { ActionPayload } from "../types";

type Handler = (payload: ActionPayload) => void | Promise<void>;

/** One inbox per WebView, not per React effect. A StrictMode cleanup cannot
 * discard a payload already removed from the native queue. At most one is held. */
export function createActionInbox(
  next: () => Promise<ActionPayload | null>,
  listen: (wake: () => void) => Promise<unknown>,
  onError: (error: unknown) => void,
  complete: () => Promise<void> = async () => {},
) {
  let consumer: Handler | null = null;
  let pending: ActionPayload | null = null;
  let subscription: Promise<unknown> | null = null;
  let listening = false;
  let pumping = false;
  let dirty = false;

  async function deliver(payload: ActionPayload, handler: Handler) {
    try {
      await handler(payload);
    } catch (error) {
      onError(error);
    } finally {
      try { await complete(); } catch (error) { onError(error); }
    }
  }

  async function pump() {
    if (pumping || !listening) return;
    pumping = true;
    try {
      while (consumer && (pending || dirty)) {
        if (!pending) {
          dirty = false;
          pending = await next();
          if (!pending) continue;
        }
        if (consumer) {
          const payload = pending;
          pending = null;
          // Start actions in order, but do not block newer cancellation/query
          // generations behind a slow translation or interactive screenshot.
          void deliver(payload, consumer);
          dirty = true;
        }
      }
    } catch (error) {
      dirty = false;
      onError(error);
    } finally {
      pumping = false;
    }
  }

  function wake() {
    dirty = true;
    void pump();
  }

  return (handler: Handler): (() => void) => {
    consumer = handler;
    // Keep one notification listener for the WebView lifetime, including
    // StrictMode's listener gap. There is no idle polling.
    if (!subscription) {
      subscription = listen(wake).then(() => { listening = true; wake(); }).catch((error) => {
        subscription = null;
        onError(error);
      });
    }
    wake();
    return () => { if (consumer === handler) consumer = null; };
  };
}
