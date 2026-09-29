// Settings → Engine: the image engine's recent output, on request (memory only;
// prompt text is redacted in Rust before a line is stored). Helps find out why
// the engine is slow or not using the graphics card.
import { useRef, useState } from "react";
import { asCoreError, engineOutput } from "../lib/api";
import { cx, focusRing } from "../components/ui";

export function EngineOutput() {
  const [text, setText] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Only the latest request may show its answer (Hide or another click drops older ones).
  const request = useRef(0);
  const load = async () => {
    const id = ++request.current;
    let out: string;
    try {
      out = await engineOutput();
    } catch (e) {
      out = asCoreError(e).message;
    }
    if (id === request.current) setText(out);
  };
  const hide = () => {
    request.current++;
    setText(null);
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text ?? "");
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Selecting the text still works.
    }
  };
  const link = cx("rounded text-xs font-medium text-neutral-600 underline underline-offset-2 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-white", focusRing);
  return (
    <div className="mt-2">
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" className={link} onClick={() => (text === null ? void load() : hide())}>
          {text === null ? "Show engine output" : "Hide engine output"}
        </button>
        {text !== null && (
          <>
            <button type="button" className={link} onClick={() => void load()}>
              Refresh
            </button>
            {text && (
              <button type="button" className={link} onClick={() => void copy()}>
                {copied ? "Copied" : "Copy"}
              </button>
            )}
          </>
        )}
      </div>
      {text !== null && (
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-black/5 p-2 font-mono text-[11px] select-text dark:bg-white/5">
          {text || "Nothing yet: the engine hasn't run since Pinhole started or was reset."}
        </pre>
      )}
    </div>
  );
}
