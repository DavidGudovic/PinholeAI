// The Linux WebView is WebKitGTK. With NVIDIA drivers it repaints `backdrop-filter` on
// every scroll frame (laggy Models list and details page) and draws an image that zooms
// inside a CSS column layout as a gray box on hover. On Linux only, these rules switch
// both off; Windows (WebView2) keeps them.
export const WEBKITGTK_CSS = `
html.webkitgtk *, html.webkitgtk *::before, html.webkitgtk *::after {
  -webkit-backdrop-filter: none !important;
  backdrop-filter: none !important;
}
html.webkitgtk .pinhole-hover-zoom {
  scale: none !important;
  transform: none !important;
  transition: none !important;
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

/**
 * A link or file dropped where nothing takes it would make the WebView open it: the app would
 * be replaced (unsaved pictures lost) and the page loaded outside the Rust network client.
 * Window listeners run after the drop targets (React listens at its root), so a DropTarget
 * that takes the drop has already called preventDefault; any other drop is refused here.
 * Text dragged into a text field still drops as usual.
 */
export function blockStrayDrops(win: Window = window): () => void {
  const editable = (t: EventTarget | null) => t instanceof Element && t.closest("input, textarea, [contenteditable]:not([contenteditable='false'])") != null;
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");
  const guard = (e: DragEvent) => {
    if (e.defaultPrevented) return;
    if (editable(e.target) && !hasFiles(e)) return;
    e.preventDefault();
    if (e.type === "dragover" && e.dataTransfer) e.dataTransfer.dropEffect = "none";
  };
  win.addEventListener("dragover", guard);
  win.addEventListener("drop", guard);
  return () => {
    win.removeEventListener("dragover", guard);
    win.removeEventListener("drop", guard);
  };
}
