// The prompt box + style row + "Paste from CivitAI".
// PRIVACY: the prompt and anything pasted stay in memory (React state) only.
import { useRef, useState } from "react";
import { ClipboardPaste, Sparkles } from "lucide-react";
import { StylePicker } from "../../components/StylePicker";
import { AutoTextarea, Button, cx, focusRing } from "../../components/ui";
import { looksLikeGenerationData } from "../../lib/paste/parse";
import type { FamilyUi } from "../../lib/types";
import { useAppState, useDispatch } from "../../lib/state/store";

export function PromptBox({ ui, onOpenPaste, onApplyPasted }: { ui: FamilyUi | null; onOpenPaste: () => void; onApplyPasted: (text: string) => void }) {
  const prompt = useAppState((s) => s.create.prompt);
  const styleId = useAppState((s) => s.create.styleId);
  const dispatch = useDispatch();
  const area = useRef<HTMLTextAreaElement>(null);
  // Generation data pasted into the box, waiting for "Apply these settings?".
  const [pending, setPending] = useState<{ text: string; start: number; end: number } | null>(null);

  const insertAsText = () => {
    if (!pending) return;
    const next = prompt.slice(0, pending.start) + pending.text + prompt.slice(pending.end);
    dispatch({ type: "patchCreate", patch: { prompt: next } });
    setPending(null);
    requestAnimationFrame(() => area.current?.focus());
  };

  return (
    <div className="space-y-2">
      <div className="rounded-xl border border-neutral-300 bg-white shadow-xs transition-colors focus-within:border-amber-500 focus-within:ring-2 focus-within:ring-amber-500/25 dark:border-neutral-700 dark:bg-neutral-950">
        <label htmlFor="prompt" className="sr-only">
          Prompt
        </label>
        <AutoTextarea
          ref={area}
          id="prompt"
          minRows={4}
          maxRows={14}
          value={prompt}
          placeholder="What do you want to see?"
          className="rounded-b-none border-0 bg-transparent px-3.5 pt-3 text-[15px] shadow-none focus:ring-0 dark:bg-transparent"
          onChange={(e) => dispatch({ type: "patchCreate", patch: { prompt: e.target.value } })}
          onPaste={(e) => {
            const text = e.clipboardData.getData("text/plain");
            if (text && looksLikeGenerationData(text)) {
              e.preventDefault();
              const el = e.currentTarget;
              setPending({ text, start: el.selectionStart ?? prompt.length, end: el.selectionEnd ?? prompt.length });
            }
          }}
        />
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-t border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
          <StylePicker value={styleId} onChange={(id) => dispatch({ type: "patchCreate", patch: { styleId: id } })} familyId={ui?.familyId} familyLabel={ui?.label} />
          <button
            type="button"
            onClick={onOpenPaste}
            className={cx(
              "ml-auto inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white",
              focusRing,
            )}
            title="Use the prompt and settings of an image from CivitAI"
          >
            <ClipboardPaste className="h-3.5 w-3.5" /> Paste from CivitAI
          </button>
        </div>
      </div>

      {pending && (
        <div className="pinhole-pop flex flex-wrap items-center gap-2 rounded-xl border border-amber-300 bg-amber-50 px-3 py-2 text-sm dark:border-amber-500/30 dark:bg-amber-500/10" role="alert">
          <Sparkles className="h-4 w-4 shrink-0 text-amber-600" />
          <span className="min-w-0 flex-1">That looks like generation data from CivitAI. Apply these settings?</span>
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              const t = pending.text;
              setPending(null);
              onApplyPasted(t);
            }}
          >
            Apply settings
          </Button>
          <Button size="sm" variant="ghost" onClick={insertAsText}>
            Paste as text
          </Button>
        </div>
      )}
    </div>
  );
}
