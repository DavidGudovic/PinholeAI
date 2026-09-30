// A model creator's description (HTML written on CivitAI), shown safely.
// The HTML is parsed into an inert document (no scripts run, nothing loads) and
// rebuilt as React elements from a short allow-list: text formatting, lists and
// https links. Images, video, iframes, styles and everything else are dropped, so
// the page never contacts another server (and Safe mode can't be bypassed by an
// embedded picture). Links open in the system browser through Rust.
import { Fragment, useMemo, useState, type ReactNode } from "react";
import { Button } from "./ui";

const DROP = new Set(["script", "style", "iframe", "object", "embed", "img", "picture", "svg", "video", "audio", "source", "form", "input", "button", "textarea", "select", "noscript", "template", "link", "meta", "head", "canvas", "map"]);
const INLINE: Record<string, string> = { strong: "font-semibold", b: "font-semibold", em: "italic", i: "italic", u: "underline", s: "line-through", del: "line-through", code: "rounded bg-neutral-100 px-1 font-mono text-[0.85em] dark:bg-neutral-800" };
const MAX_DEPTH = 24;

/** `https:` links only; anything else (javascript:, data:, http:, relative) shows as plain text. */
export function safeHref(href: string | null): string | null {
  if (!href) return null;
  try {
    const u = new URL(href.trim());
    return u.protocol === "https:" && u.hostname && !u.username && !u.password ? u.toString() : null;
  } catch {
    return null;
  }
}

function build(node: Node, onLink: (url: string) => void, key: string, depth: number): ReactNode {
  if (node.nodeType === 3) return node.textContent;
  if (node.nodeType !== 1 || depth > MAX_DEPTH) return null;
  const el = node as Element;
  const tag = el.tagName.toLowerCase();
  if (DROP.has(tag)) return null;
  const kids = Array.from(el.childNodes).map((c, i) => build(c, onLink, `${key}.${i}`, depth + 1));
  if (tag === "br") return <br key={key} />;
  if (tag === "hr") return <hr key={key} className="my-2 border-neutral-200 dark:border-neutral-800" />;
  if (Object.hasOwn(INLINE, tag)) return <span key={key} className={INLINE[tag]}>{kids}</span>;
  if (tag === "a") {
    const href = safeHref(el.getAttribute("href"));
    if (!href) return <Fragment key={key}>{kids}</Fragment>;
    return (
      <a
        key={key}
        href={href}
        title={href}
        rel="noreferrer noopener"
        className="text-amber-700 underline hover:text-amber-600 dark:text-amber-400"
        onClick={(e) => {
          e.preventDefault();
          onLink(href);
        }}
        onAuxClick={(e) => e.preventDefault()}
      >
        {kids}
      </a>
    );
  }
  if (/^h[1-6]$/.test(tag)) return <div key={key} className="mt-2 font-semibold first:mt-0">{kids}</div>;
  if (tag === "p" || tag === "div" || tag === "section" || tag === "article" || tag === "tr") return <div key={key}>{kids}</div>;
  if (tag === "td" || tag === "th") return <span key={key} className="mr-3">{kids}</span>;
  if (tag === "ul") return <ul key={key} className="list-disc space-y-0.5 pl-5">{kids}</ul>;
  if (tag === "ol") return <ol key={key} className="list-decimal space-y-0.5 pl-5">{kids}</ol>;
  if (tag === "li") return <li key={key}>{kids}</li>;
  if (tag === "blockquote") return <blockquote key={key} className="border-l-2 border-neutral-300 pl-3 text-neutral-600 dark:border-neutral-700 dark:text-neutral-400">{kids}</blockquote>;
  if (tag === "pre") return <pre key={key} className="overflow-x-auto rounded bg-neutral-100 p-2 font-mono text-xs dark:bg-neutral-800">{el.textContent}</pre>;
  // Unknown or layout-only tags (span, table cells, font…): keep their text.
  return <Fragment key={key}>{kids}</Fragment>;
}

/** Sanitized, rendered description plus its plain-text length (to decide on "Show more"). */
export function renderCreatorHtml(html: string, onLink: (url: string) => void): { content: ReactNode; textLength: number } {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const body = doc.body;
  const textLength = (body.textContent ?? "").trim().length;
  return { content: Array.from(body.childNodes).map((c, i) => build(c, onLink, String(i), 0)), textLength };
}

/** True when the description has text left after sanitizing (image-only descriptions have none). */
export function hasCreatorText(html: string | null | undefined): boolean {
  return !!html && renderCreatorHtml(html, () => {}).textLength > 0;
}

const COLLAPSE_OVER = 600;

export function CreatorText({ html, onLink }: { html: string; onLink: (url: string) => void }) {
  const { content, textLength } = useMemo(() => renderCreatorHtml(html, onLink), [html, onLink]);
  const [expanded, setExpanded] = useState(false);
  if (textLength === 0) return null;
  const long = textLength > COLLAPSE_OVER;
  return (
    <div>
      <div className={`relative space-y-2 break-words text-sm select-text ${long && !expanded ? "max-h-40 overflow-hidden" : ""}`}>
        {content}
        {long && !expanded && <div className="pointer-events-none absolute inset-x-0 bottom-0 h-12 bg-gradient-to-t from-white to-transparent dark:from-neutral-900" />}
      </div>
      {long && (
        <Button variant="ghost" className="mt-1" onClick={() => setExpanded((e) => !e)}>
          {expanded ? "Show less" : "Show more"}
        </Button>
      )}
    </div>
  );
}
