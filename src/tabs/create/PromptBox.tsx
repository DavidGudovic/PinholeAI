// The prompt box + style row + "Paste from CivitAI".
// PRIVACY: the prompt and anything pasted stay in memory (React state) only.
import { useRef, useState } from "react";
import { ClipboardPaste, Sparkles } from "lucide-react";
import { StylePicker } from "../../components/StylePicker";
import { AutoTextarea, Button, cx, focusRing } from "../../components/ui";
import { looksLikeGenerationData } from "../../lib/paste/parse";
import type { FamilyUi } from "../../lib/types";
import { recallStep, shouldRecall, type Browse } from "../../lib/state/promptRecall";
import { useAppState, useDispatch } from "../../lib/state/store";
import { useImprovePrompt } from "./ImprovePrompt";

export function PromptBox({ ui, onOpenPaste, onApplyPasted }: { ui: FamilyUi | null; onOpenPaste: () => void; onApplyPasted: (text: string) => void }) {
  const prompt = useAppState((s) => s.create.prompt);
  const styleId = useAppState((s) => s.create.styleId);
  const history = useAppState((s) => s.promptHistory);
  const dispatch = useDispatch();
  const browse = useRef<Browse | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  const improve = useImprovePrompt(ui?.familyId);
  // Generation data pasted into the box, waiting for "Apply these settings?".
  // `base` is the prompt at paste time: the offsets only fit that text.
  const [pending, setPending] = useState<{ text: string; start: number; end: number; base: string } | null>(null);

  const insertAsText = () => {
    if (!pending) return;
    // Typed on since the paste: the old offsets would cut into the new text, so add it at the end.
    const next =
      prompt === pending.base
        ? prompt.slice(0, pending.start) + pending.text + prompt.slice(pending.end)
        : prompt + (prompt && !/\s$/.test(prompt) ? " " : "") + pending.text;
    dispatch({ type: "patchCreate", patch: { prompt: next } });
    setPending(null);
    requestAnimationFrame(() => area.current?.focus());
  };

  return (
    <div className="space-y-2">
      <div className="flex items-end justify-between gap-2">
        <label htmlFor="prompt" className="text-sm font-medium text-neutral-800 dark:text-neutral-200">
          Prompt
        </label>
        <button
          type="button"
          onClick={onOpenPaste}
          className={cx(
            "-mb-0.5 inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white",
            focusRing,
          )}
          title="Use the prompt and settings of an image from CivitAI"
        >
          <ClipboardPaste className="h-3.5 w-3.5" /> Paste from CivitAI
        </button>
      </div>
      <div className="rounded-xl border border-neutral-300 bg-white shadow-xs transition-colors focus-within:border-amber-500 focus-within:ring-2 focus-within:ring-amber-500/25 dark:border-neutral-700 dark:bg-neutral-950">
        <AutoTextarea
          ref={area}
          bare
          id="prompt"
          minRows={4}
          maxRows={14}
          value={prompt}
          placeholder="What do you want to see?"
          className="px-3.5 pt-3 text-[15px]"
          onChange={(e) => {
            browse.current = null;
            dispatch({ type: "patchCreate", patch: { prompt: e.target.value } });
          }}
          onKeyDown={(e) => {
            if ((e.key !== "ArrowUp" && e.key !== "ArrowDown") || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey || e.nativeEvent.isComposing) return;
            const dir = e.key === "ArrowUp" ? -1 : 1;
            const el = e.currentTarget;
            if (!history.length || !shouldRecall(el.value, el.selectionStart, el.selectionEnd, dir, browse.current !== null)) return;
            const step = recallStep(history, browse.current, el.value, dir);
            browse.current = step.browse;
            if (step.text === null) return;
            e.preventDefault();
            dispatch({ type: "patchCreate", patch: { prompt: step.text } });
            const end = step.text.length;
            requestAnimationFrame(() => area.current?.setSelectionRange(end, end));
          }}
          onPaste={(e) => {
            const text = e.clipboardData.getData("text/plain");
            if (text && looksLikeGenerationData(text)) {
              e.preventDefault();
              const el = e.currentTarget;
              setPending({ text, start: el.selectionStart ?? prompt.length, end: el.selectionEnd ?? prompt.length, base: prompt });
            }
          }}
        />
        {/* Wraps instead of squeezing: Style + "Save as style" and the Improve model picker don't fit on one line in the sidebar. */}
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-t border-neutral-200 px-2 py-1.5 dark:border-neutral-800">
          <StylePicker value={styleId} onChange={(id) => dispatch({ type: "patchCreate", patch: { styleId: id } })} familyId={ui?.familyId} familyLabel={ui?.label} />
          <div className="ml-auto flex items-center gap-1">
            {improve.picker}
            {improve.button}
          </div>
        </div>
      </div>

      {improve.notice}

      {pending && (
        <div className="pinhole-pop rounded-xl border border-amber-300 bg-amber-50 px-3 py-2.5 text-sm dark:border-amber-500/30 dark:bg-amber-500/10" role="alert">
          <div className="flex items-start gap-2">
            <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
            <span className="min-w-0 flex-1">That looks like generation data from CivitAI. Apply its settings too?</span>
          </div>
          <div className="mt-2 flex flex-wrap gap-2 pl-6">
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
        </div>
      )}
    </div>
  );
}
