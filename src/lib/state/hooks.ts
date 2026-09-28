import { useEffect, useState } from "react";
import type { FamilyUi, InstalledModel } from "../types";
import { useActions } from "./AppProvider";
import { useAppState } from "./store";

/** Family UI for a family id (fetched once, cached in state). */
export function useFamilyUi(familyId: string | null | undefined): FamilyUi | null {
  const ui = useAppState((s) => (familyId ? (s.familyUi[familyId] ?? null) : null));
  const actions = useActions();
  useEffect(() => {
    if (familyId && !ui) void actions.ensureFamilyUi(familyId).catch(() => undefined);
  }, [familyId, ui, actions]);
  return ui;
}

export function useModel(id: string | null | undefined): InstalledModel | null {
  return useAppState((s) => (id ? ((s.models ?? []).find((m) => m.id === id) ?? null) : null));
}

/** Seconds since `start` (ms), ticking while `active`. */
export function useElapsed(start: number | null, active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [active]);
  return start ? Math.max(0, Math.floor((now - start) / 1000)) : 0;
}

/** A value that follows `value` after `ms` of quiet. */
export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
