import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError } from "./apiClient";

export type PollingStatus = "idle" | "connecting" | "live" | "reconnecting" | "unavailable";
type Snapshot<T> = { key: string | null; data: T | null; status: PollingStatus; error: Error | null };

function emptySnapshot<T>(key: string | null): Snapshot<T> {
  return { key, data: null, status: key === null ? "idle" : "connecting", error: null };
}

/** Serialize reads, cancel obsolete sessions, and retain the last successful value during an outage. */
export function usePollingResource<T>(key: string | null, load: (signal: AbortSignal) => Promise<T>, intervalMs = 1_000) {
  const loadRef = useRef(load);
  loadRef.current = load;
  const [snapshot, setSnapshot] = useState<Snapshot<T>>(() => emptySnapshot(key));
  const [revision, setRevision] = useState(0);
  const cancelRef = useRef<(() => void) | null>(null);
  const refresh = useCallback(() => {
    cancelRef.current?.();
    setRevision((current) => current + 1);
  }, []);

  useEffect(() => {
    setSnapshot((current) => current.key === key ? current : emptySnapshot(key));
    if (key === null) return;
    let active = true;
    let pending = false;
    let terminal = false;
    let failures = 0;
    let timer: number | undefined;
    let controller: AbortController | undefined;
    let deadline: number | undefined;

    const poll = async () => {
      if (!active || pending || terminal) return;
      pending = true;
      controller = new AbortController();
      let timedOut = false;
      deadline = window.setTimeout(() => { timedOut = true; controller?.abort(); }, 10_000);
      let delay = intervalMs;
      try {
        const data = await loadRef.current(controller.signal);
        if (!active) return;
        failures = 0;
        setSnapshot({ key, data, status: "live", error: null });
      } catch (cause) {
        if (!active || (controller.signal.aborted && !timedOut)) return;
        const error = timedOut ? new Error("The server did not respond within 10 seconds.")
          : cause instanceof Error ? cause : new Error(String(cause));
        terminal = error instanceof ApiError && (error.status === 403 || error.status === 404);
        delay = Math.min(intervalMs * 2 ** Math.min(failures++, 4), 10_000);
        setSnapshot((current) => ({
          key, data: terminal ? null : current.key === key ? current.data : null,
          status: terminal ? "unavailable" : "reconnecting", error
        }));
      } finally {
        if (deadline !== undefined) window.clearTimeout(deadline);
        controller.abort();
        pending = false;
        if (active && !terminal) timer = window.setTimeout(() => void poll(), delay);
      }
    };
    const wake = () => {
      if (document.visibilityState === "hidden" || pending || terminal) return;
      if (timer !== undefined) window.clearTimeout(timer);
      void poll();
    };
    void poll();
    window.addEventListener("online", wake);
    document.addEventListener("visibilitychange", wake);
    const cancel = () => {
      active = false;
      controller?.abort();
      if (deadline !== undefined) window.clearTimeout(deadline);
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener("online", wake);
      document.removeEventListener("visibilitychange", wake);
    };
    cancelRef.current = cancel;
    return () => {
      cancel();
      if (cancelRef.current === cancel) cancelRef.current = null;
    };
  }, [key, intervalMs, revision]);

  // Do not expose a previous game's or role's data for even one render.
  const current = snapshot.key === key ? snapshot : emptySnapshot<T>(key);
  return { ...current, refresh };
}