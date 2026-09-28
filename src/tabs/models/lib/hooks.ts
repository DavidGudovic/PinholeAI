// Small React hooks shared by Models, Settings and First run.
import { useCallback, useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { asCoreError, engineStatus, getHardware, installEngine, onEngine, onHardwareReady } from "../../../lib/api";
import type { CoreError, EngineStatus, GroupStatus, HardwareView } from "../../../lib/types";
import { onSettingsChanged } from "../../../settings/events";
import { getTagged, knownGroupIds, newestActiveOfKind, newestGroupSince, tagGroup, useDownloadsVersion } from "./downloads";

/** Subscribe to a Tauri event helper from api.ts for the component's lifetime. */
export function useTauriEvent<A extends unknown[]>(subscribe: (cb: (...args: A) => void) => Promise<UnlistenFn>, cb: (...args: A) => void) {
  const ref = useRef(cb);
  ref.current = cb;
  useEffect(() => {
    let off: UnlistenFn | null = null;
    let alive = true;
    subscribe((...args: A) => ref.current(...args))
      .then((un) => {
        if (alive) off = un;
        else un();
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      off?.();
    };
  }, [subscribe]);
}

/**
 * Effective hardware (`get_hardware`: GPU, VRAM, backend), refreshed when detection
 * finishes and when Settings change (GPU / VRAM / backend overrides). null while loading.
 */
export function useHardware(): HardwareView | null {
  const [hw, setHw] = useState<HardwareView | null>(null);
  const load = useCallback(() => {
    getHardware()
      .then(setHw)
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    load();
    return onSettingsChanged(() => load());
  }, [load]);
  useTauriEvent(onHardwareReady, load);
  return hw;
}

export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

const isEngineGroup = (g: GroupStatus) => g.kind === "engine";

/** Engine status + install with progress (the engine group is the newest group with kind "engine"). */
export function useEngine() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [busy, setBusy] = useState(false);
  const before = useRef<Set<string> | null>(null);
  useDownloadsVersion();

  const refresh = useCallback(async () => {
    try {
      setStatus(await engineStatus());
    } catch (e) {
      setError(asCoreError(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useTauriEvent(onEngine, (s: EngineStatus) => setStatus(s));

  const install = useCallback(async () => {
    setError(null);
    setBusy(true);
    before.current = knownGroupIds();
    try {
      setStatus(await installEngine());
    } catch (e) {
      const ce = asCoreError(e);
      if (ce.code !== "cancelled") setError(ce);
      void refresh();
    } finally {
      setBusy(false);
    }
  }, [refresh]);

  let group: GroupStatus | null = null;
  if (busy && before.current) group = newestGroupSince(before.current, isEngineGroup);
  // Started elsewhere (Top bar, error "Set up engine" button, another screen).
  if (!group && status?.installing) group = newestActiveOfKind("engine");
  if (!group) {
    const tagged = getTagged("engine");
    if (tagged && (busy || status?.installing || tagged.state === "failed")) group = tagged;
  }
  const groupId = group?.groupId ?? null;
  useEffect(() => {
    if (groupId && getTagged("engine")?.groupId !== groupId) tagGroup("engine", groupId);
  }, [groupId]);

  return { status, error, setError, busy: busy || !!status?.installing, install, group, refresh };
}
