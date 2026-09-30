// Keyboard shortcuts (SPEC §12). Views register what each key does for their current image; the
// App shell listens once and runs the active tab's handler. Letter keys never act while typing.
import { useEffect, useRef, type MutableRefObject } from "react";
import type { TabId } from "./state/model";

export type ShortcutAction = "save" | "saveAs" | "edit" | "describe" | "fullscreen" | "tryAgain";
export type ShortcutHandlers = Partial<Record<ShortcutAction, () => void>>;

// Per tab, the registered views' (always current) handler sets, oldest first.
const registry = new Map<TabId, MutableRefObject<ShortcutHandlers>[]>();

/** Register the tab's shortcut handlers while mounted. A key with no handler here does nothing. */
export function useShortcuts(tab: TabId, h: ShortcutHandlers) {
  const ref = useRef(h);
  useEffect(() => {
    ref.current = h;
  });
  useEffect(() => {
    registry.set(tab, [...(registry.get(tab) ?? []), ref]);
    return () => {
      registry.set(
        tab,
        (registry.get(tab) ?? []).filter((r) => r !== ref),
      );
    };
  }, [tab]);
}

function isTyping(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  return !!el && (el.tagName === "TEXTAREA" || el.tagName === "INPUT" || el.tagName === "SELECT" || el.isContentEditable);
}

type KeyInfo = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

/** Which action a key press means, or null. Exported for tests. */
export function shortcutFor(e: KeyInfo): ShortcutAction | null {
  if (e.altKey) return null;
  const k = e.key.toLowerCase();
  if (e.ctrlKey || e.metaKey) return k === "s" ? (e.shiftKey ? "saveAs" : "save") : null;
  if (e.shiftKey) return null;
  switch (k) {
    case "s":
      return "save";
    case "e":
      return "edit";
    case "d":
      return "describe";
    case "f":
      return "fullscreen";
    case "r":
      return "tryAgain";
    default:
      return null;
  }
}

/** Run the shortcut for this key press in the active tab. Returns true when it was handled. */
export function runShortcut(e: KeyboardEvent, tab: TabId): boolean {
  const action = shortcutFor(e);
  if (!action) return false;
  if (!(e.ctrlKey || e.metaKey) && isTyping(e.target)) return false;
  if (document.querySelector('[role="dialog"]')) return false;
  for (const r of [...(registry.get(tab) ?? [])].reverse()) {
    const f = r.current[action];
    if (!f) continue;
    if (!e.repeat) f();
    return true;
  }
  return false;
}

/** The list shown in Settings and by the ? key. */
export const SHORTCUT_LIST: { keys: string[]; what: string }[] = [
  { keys: ["Mod", "Enter"], what: "Generate, make the edit, or describe (current tab)" },
  { keys: ["E"], what: "Edit the current image" },
  { keys: ["D"], what: "Describe the current image" },
  { keys: ["S"], what: "Save the current image" },
  { keys: ["Mod", "Shift", "S"], what: "Save as…" },
  { keys: ["F"], what: "View the current image full screen" },
  { keys: ["R"], what: "Try again (in Edit)" },
  { keys: ["Mod", "Z"], what: "Undo an edit (Shift to redo)" },
  { keys: ["Esc"], what: "Close a window or the full-screen view" },
  { keys: ["?"], what: "Show this list" },
];
