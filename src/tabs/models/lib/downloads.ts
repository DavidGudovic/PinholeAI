// One app-wide view of download groups, fed by `list_downloads` + `download-progress`
// events. Shared by the Models tab, Recommended cards, First run and Settings, so the
// event listener is registered once. Tags (RAM only) remember which group belongs to
// which button ("rec:edit", "civitai:123", "engine") so progress survives remounts.
import { useCallback, useRef, useSyncExternalStore } from "react";
import { cancelDownload, listDownloads, onDownload } from "../../../lib/api";
import type { GroupStatus } from "../../../lib/types";
import { isActive } from "./words";

const groups = new Map<string, GroupStatus>();
const order: string[] = [];
const tags = new Map<string, string>();
const hidden = new Set<string>();
const listeners = new Set<() => void>();
let snapshot: GroupStatus[] = [];
let version = 0;
let started = false;

function changed() {
  snapshot = order.map((id) => groups.get(id)!).filter(Boolean);
  version += 1;
  for (const l of listeners) l();
}

/** Merge a status update (event or list). */
export function upsertGroup(g: GroupStatus) {
  if (!groups.has(g.groupId)) order.push(g.groupId);
  groups.set(g.groupId, g);
  changed();
}

function start() {
  if (started) return;
  started = true;
  onDownload(upsertGroup).catch(() => {
    started = false;
  });
  listDownloads()
    .then((list) => {
      // Events that arrived meanwhile are newer: keep them.
      for (const g of list) if (!groups.has(g.groupId)) upsertGroup(g);
    })
    .catch(() => undefined);
}

function subscribe(l: () => void) {
  start();
  listeners.add(l);
  return () => listeners.delete(l);
}

/** Every known group, oldest first. */
export function useDownloads(): GroupStatus[] {
  return useSyncExternalStore(subscribe, () => snapshot);
}

/** Re-render on any change; returns a counter (for selectors below). */
function useVersion(): number {
  return useSyncExternalStore(subscribe, () => version);
}

export function tagGroup(key: string, groupId: string) {
  tags.set(key, groupId);
  hidden.delete(groupId);
  changed();
}

export function getTagged(key: string): GroupStatus | null {
  const id = tags.get(key);
  return id ? groups.get(id) ?? null : null;
}

/**
 * The group tagged `key`, or (fallback) the newest active group matching `match`.
 * Re-renders only when that group's status object changes (cards stay cheap).
 */
export function useTaggedGroup(key: string, match?: (g: GroupStatus) => boolean): GroupStatus | null {
  const matchRef = useRef(match);
  matchRef.current = match;
  const get = useCallback(() => {
    const tagged = getTagged(key);
    if (tagged) return tagged;
    const m = matchRef.current;
    if (!m) return null;
    for (let i = snapshot.length - 1; i >= 0; i--) if (isActive(snapshot[i]) && m(snapshot[i])) return snapshot[i];
    return null;
  }, [key]);
  return useSyncExternalStore(subscribe, get);
}

export function useGroup(groupId: string | null): GroupStatus | null {
  const get = useCallback(() => (groupId ? (groups.get(groupId) ?? null) : null), [groupId]);
  return useSyncExternalStore(subscribe, get);
}

/** Groups shown in the Models tab downloads list (finished ones can be hidden). */
export function useVisibleDownloads(): GroupStatus[] {
  const all = useDownloads();
  useVersion();
  return all.filter((g) => !hidden.has(g.groupId));
}

export function hideGroup(groupId: string) {
  hidden.add(groupId);
  changed();
}

export function hideFinished() {
  for (const g of snapshot) if (!isActive(g)) hidden.add(g.groupId);
  changed();
}

/**
 * "Clear finished" in the top bar and in Models → Downloads: the two lists have their own
 * stores (the app reducer and this one), so one button clears both.
 */
export function clearFinishedEverywhere(actions: { clearFinishedDownloads: () => void }) {
  actions.clearFinishedDownloads();
  hideFinished();
}

export async function cancelGroup(groupId: string) {
  await cancelDownload(groupId);
}

/** Ids known right now (to spot the group a call without a returned id started, e.g. the engine). */
export function knownGroupIds(): Set<string> {
  return new Set(order);
}

/** Newest group not in `before` (only groups matching `match` when given). */
export function newestGroupSince(before: Set<string>, match?: (g: GroupStatus) => boolean): GroupStatus | null {
  const fresh = snapshot.filter((g) => !before.has(g.groupId) && (!match || match(g)));
  return fresh.length ? fresh[fresh.length - 1] : null;
}

/** Newest active group of `kind` (e.g. the engine download started from another screen). */
export function newestActiveOfKind(kind: NonNullable<GroupStatus["kind"]>): GroupStatus | null {
  for (let i = snapshot.length - 1; i >= 0; i--) if (snapshot[i].kind === kind && isActive(snapshot[i])) return snapshot[i];
  return null;
}

export { useVersion as useDownloadsVersion };
