// The Linux WebView is WebKitGTK. With NVIDIA drivers it repaints `backdrop-filter` on
// every scroll frame (laggy Models list and details page) and draws an image that zooms
// inside a CSS column layout as a gray box on hover. On Linux only, these rules switch
// both off; Windows (WebView2) keeps them. Browse cards also drop their shadow and their
// pulsing placeholders there: repainting ~100 shadows and pulses per scroll frame made
// scrolling the grid lag (measured in WebKitGTK 2.52: average frame 57 ms → 22 ms).
export const WEBKITGTK_CSS = `
html.webkitgtk *, html.webkitgtk *::before, html.webkitgtk *::after {
  -webkit-backdrop-filter: none !important;
  backdrop-filter: none !important;
}
html.webkitgtk .pinhole-hover-zoom {
  scale: none !important;
  transform: none !important;
  transition: none !important;
}
html.webkitgtk .pinhole-card {
  box-shadow: none !important;
  transition: none !important;
}
html.webkitgtk .pinhole-card .animate-pulse {
  animation: none !important;
}`;

/** True for the Linux WebView (WebKitGTK). */
export function isWebKitGtk(userAgent: string): boolean {
  return /Linux/.test(userAgent) && /AppleWebKit/.test(userAgent) && !/Chrome|Chromium|Android/.test(userAgent);
}

/** On Linux, tag <html> and add the rules above. */
export function markPlatform(doc: Document = document, userAgent: string = navigator.userAgent): void {
  if (!isWebKitGtk(userAgent)) return;
  doc.documentElement.classList.add("webkitgtk");
  const style = doc.createElement("style");
  style.textContent = WEBKITGTK_CSS;
  doc.head.appendChild(style);
}

/** Takes a drop that no drop area took (see `blockStrayDrops`). */
export type StrayDropHandler = (dt: DataTransfer) => void;
let strayDrop: StrayDropHandler | null = null;

/** Hands drops that no drop area took to `h` (the paste chooser) instead of refusing them. */
export function onStrayDrop(h: StrayDropHandler): () => void {
  strayDrop = h;
  return () => {
    if (strayDrop === h) strayDrop = null;
  };
}

/**
 * A link or file dropped where nothing takes it would make the WebView open it: the app would
 * be replaced (unsaved pictures lost) and the page loaded outside the Rust network client.
 * Window listeners run after the drop targets (React listens at its root), so a DropTarget
 * that takes the drop has already called preventDefault; any other drop is kept from the
 * WebView here and given to the `onStrayDrop` handler, which treats a picture like a paste.
 * Text dragged into a text field still drops as usual.
 */
export function blockStrayDrops(win: Window = window): () => void {
  const editable = (t: EventTarget | null) => t instanceof Element && t.closest("input, textarea, [contenteditable]:not([contenteditable='false'])") != null;
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");
  const guard = (e: DragEvent) => {
    if (e.defaultPrevented) return;
    const files = hasFiles(e);
    if (editable(e.target) && !files) return;
    e.preventDefault();
    // A link or page markup is taken too (and never opened), so the handler can read a picture
    // held in it or say what to do.
    const types = Array.from(e.dataTransfer?.types ?? []);
    const link = types.includes("text/uri-list") || types.includes("text/html");
    if (e.type === "dragover" && e.dataTransfer) e.dataTransfer.dropEffect = (files || link) && strayDrop ? "copy" : "none";
    if (e.type === "drop" && e.dataTransfer && strayDrop) strayDrop(e.dataTransfer);
  };
  win.addEventListener("dragover", guard);
  win.addEventListener("drop", guard);
  return () => {
    win.removeEventListener("dragover", guard);
    win.removeEventListener("drop", guard);
  };
}
