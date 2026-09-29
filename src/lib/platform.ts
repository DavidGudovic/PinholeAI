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
