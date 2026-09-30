// Small platform helpers that work both inside Tauri and in the browser mock.
// PRIVACY: clipboard text may be prompt text — return it, never log or store it.
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open, save } from "@tauri-apps/plugin-dialog";
import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
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

/** Native folder picker (Tauri only). Returns null when cancelled or unavailable. */
export async function chooseFolder(title: string): Promise<string | null> {
  if (!isTauri()) return null;
  const dir = await open({ directory: true, multiple: false, title });
  return typeof dir === "string" ? dir : null;
}

/** Closes the window for real (after the user chose "Close without saving"). */
export async function closeWindow(): Promise<void> {
  if (isTauri()) await getCurrentWindow().destroy();
  else window.close();
}

/** Calls `handler` when the user closes the window; it calls `prevent()` to keep the window open. */
export function onCloseRequested(handler: (prevent: () => void) => void): () => void {
  if (!isTauri()) return () => undefined;
  let un: (() => void) | null = null;
  let gone = false;
  void getCurrentWindow()
    .onCloseRequested((e) => handler(() => e.preventDefault()))
    .then((u) => (gone ? u() : (un = u)))
    .catch(() => undefined);
  return () => {
    gone = true;
    un?.();
  };
}

let audio: AudioContext | null = null;

/** Create/resume the audio context while the user's click is still fresh (WebViews block sound otherwise). */
export function primeSound(): void {
  try {
    audio ??= new AudioContext();
    if (audio.state === "suspended") void audio.resume();
  } catch {
    /* no audio available */
  }
}

/** A short two-note chime, made in code (no sound file, nothing loaded). */
function playChime(): void {
  try {
    audio ??= new AudioContext();
    const ctx = audio;
    const t0 = ctx.currentTime + 0.02;
    [659.25, 880].forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      const t = t0 + i * 0.14;
      gain.gain.setValueAtTime(0.0001, t);
      gain.gain.exponentialRampToValueAtTime(0.12, t + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.45);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t);
      osc.stop(t + 0.5);
    });
  } catch {
    /* no audio available */
  }
}

/** True when the user is probably looking at something else. */
export const windowInBackground = () => typeof document !== "undefined" && (document.hidden || !document.hasFocus());

/** "Your picture is ready": flash the taskbar icon (and play a soft chime when `sound`). Call only when the window isn't focused. */
export function notifyDone(sound: boolean): void {
  if (isTauri()) void getCurrentWindow().requestUserAttention(UserAttentionType.Informational).catch(() => undefined);
  if (sound) playChime();
}
