// Small platform helpers that work both inside Tauri and in the browser mock.
// PRIVACY: clipboard text may be prompt text — return it, never log or store it.
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { save } from "@tauri-apps/plugin-dialog";
import { isTauri } from "../mock";

export async function copyText(text: string): Promise<void> {
  if (isTauri()) {
    await writeText(text);
    return;
  }
  await navigator.clipboard.writeText(text);
}

/** Clipboard text, or null when unavailable / not permitted. */
export async function readClipboardText(): Promise<string | null> {
  try {
    if (isTauri()) {
      return await readText();
    }
    if (navigator.clipboard?.readText) return await navigator.clipboard.readText();
  } catch {
    /* no permission / empty / not text */
  }
  return null;
}

/** Native "Save as" dialog (Tauri only). Returns null when cancelled or unavailable. */
export async function chooseSavePath(defaultName: string): Promise<string | null> {
  if (!isTauri()) return null;
  return (await save({ defaultPath: defaultName, filters: [{ name: "PNG image", extensions: ["png"] }] })) ?? null;
}

export const canSaveAs = () => isTauri();

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** "Ctrl+Enter" / "⌘ Enter" label. */
export const modKey = isMac ? "⌘" : "Ctrl";
