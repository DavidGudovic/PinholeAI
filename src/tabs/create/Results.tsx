// Results: big preview of the selected image, its actions and settings summary,
// and a strip of every image made this session (in memory until Save).
import { memo, useState } from "react";
import { ChevronDown, Copy, FolderOpen, ImageUp, Save, ScanText, Shuffle, Trash, WandSparkles } from "lucide-react";
import { Logo } from "../../components/Logo";
import { Button, ErrorNotice, IconButton, Kbd, MenuItem, Popover, cx, focusRing } from "../../components/ui";
import * as api from "../../lib/api";
import type { CoreError, ResultImage } from "../../lib/types";
import { useActions } from "../../lib/state/AppProvider";
import type { ImgRef } from "../../lib/state/model";
import { canSaveAs, modKey } from "../../lib/state/platform";
import { settingsSummary } from "../../lib/state/request";
import { useAppState, useDispatch } from "../../lib/state/store";

export function Results() {
  const results = useAppState((s) => s.results);
  const selectedId = useAppState((s) => s.selectedResultId);
  const images = useAppState((s) => s.images);
  const job = useAppState((s) => s.job);
  const count = useAppState((s) => s.create.count);
  const selected = results.find((r) => r.id === selectedId) ?? null;
  const generating = job?.kind === "create" || job?.kind === "upscale";
  const pending = generating ? (job?.kind === "upscale" ? 1 : count) : 0;

  return (
    <section aria-label="Results" className="flex min-h-0 min-w-0 flex-col">
      {selected && images[selected.id] ? (
        <Preview result={selected} img={images[selected.id]} />
      ) : generating ? (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6">
          <div className="pinhole-shimmer aspect-square w-full max-w-md rounded-2xl bg-neutral-200 dark:bg-neutral-900" />
        </div>
      ) : (
        <EmptyResults />
      )}
      {(results.length > 0 || pending > 0) && <Strip results={results} images={images} selectedId={selectedId} pending={pending} />}
    </section>
  );
}

function EmptyResults() {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-8 text-center">
      <div className="mb-5 rounded-3xl bg-white p-5 shadow-sm ring-1 ring-neutral-200 dark:bg-neutral-900 dark:ring-neutral-800">
        <Logo className="h-12 w-12" />
      </div>
      <h2 className="text-lg font-semibold tracking-tight">Your images appear here</h2>
      <p className="mt-1.5 max-w-sm text-sm text-neutral-500">
        Describe what you want to see and press <span className="font-medium text-neutral-700 dark:text-neutral-300">Generate</span>
        <span className="mx-1 inline-flex translate-y-[-1px] gap-0.5 text-neutral-500">
          <Kbd>{modKey}</Kbd>
          <Kbd>Enter</Kbd>
        </span>
        . Images stay in memory until you save them.
      </p>
    </div>
  );
}

function Preview({ result, img }: { result: ResultImage; img: ImgRef }) {
  const actions = useActions();
  const job = useAppState((s) => s.job);
  const hasBatch = useAppState((s) => !!s.resultBatch[result.id]);
  const [error, setError] = useState<CoreError | null>(null);
  const [saved, setSaved] = useState<Record<string, string>>({});
  const busy = !!job;

  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(api.asCoreError(e));
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 px-6 pt-5 pb-3">
      <div className="relative flex min-h-0 flex-1 items-center justify-center">
        <img
          src={img.url}
          alt={`Generated image, ${result.width}×${result.height}, seed ${result.seed}`}
          className="max-h-full max-w-full rounded-lg object-contain shadow-lg ring-1 ring-black/5 dark:ring-white/10"
          draggable={false}
        />
      </div>

      <div className="flex flex-wrap items-center justify-center gap-1.5">
        <div className="inline-flex">
          <Button
            variant="secondary"
            className={cx(canSaveAs() && "rounded-r-none")}
            onClick={() =>
              void run(async () => {
                const s = await actions.save(result.id);
                setSaved((m) => ({ ...m, [result.id]: s.path }));
              })
            }
          >
            <Save className="h-4 w-4" /> Save
          </Button>
          {canSaveAs() && (
            <Popover
              align="end"
              width={180}
              trigger={(p) => (
                <button {...p} type="button" aria-label="More save options" className={cx("inline-flex h-9 items-center rounded-r-lg border border-l-0 border-neutral-200 bg-white px-1.5 hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:hover:bg-neutral-700", focusRing)}>
                  <ChevronDown className="h-4 w-4" />
                </button>
              )}
            >
              {(close) => (
                <MenuItem
                  onClick={() => {
                    close();
                    void run(() => actions.saveAs(result.id, result.seed));
                  }}
                >
                  Save as…
                </MenuItem>
              )}
            </Popover>
          )}
        </div>
        <Button onClick={() => actions.sendToEdit(result.id)}>
          <WandSparkles className="h-4 w-4" /> Edit this
        </Button>
        <Button onClick={() => actions.sendToDescribe(result.id)}>
          <ScanText className="h-4 w-4" /> Describe
        </Button>
        <Button disabled={busy || !hasBatch} title={hasBatch ? "Same prompt, new seeds" : "Only for images made in this session"} onClick={() => void run(() => actions.variations(result.id))}>
          <Shuffle className="h-4 w-4" /> Variations
        </Button>
        <Popover
          width={200}
          trigger={(p) => (
            <Button {...p} disabled={busy}>
              <ImageUp className="h-4 w-4" /> Upscale <ChevronDown className="h-3.5 w-3.5 opacity-60" />
            </Button>
          )}
        >
          {(close) => (
            <>
              {([2, 4] as const).map((f) => (
                <MenuItem
                  key={f}
                  hint={`${result.width * f}×${result.height * f}`}
                  onClick={() => {
                    close();
                    void run(() => actions.upscale(result.id, f));
                  }}
                >
                  Upscale {f}×
                </MenuItem>
              ))}
            </>
          )}
        </Popover>
        <IconButton label="Copy image" variant="secondary" onClick={() => void run(() => actions.copyImage(result.id))}>
          <Copy className="h-4 w-4" />
        </IconButton>
        <IconButton label="Remove from this session" variant="ghost" onClick={() => actions.removeResult(result.id)}>
          <Trash className="h-4 w-4" />
        </IconButton>
      </div>

      <p className="text-center text-xs text-neutral-500 tabular-nums">{settingsSummary(result)}</p>
      {saved[result.id] && (
        <p className="flex items-center justify-center gap-2 text-xs text-emerald-700 dark:text-emerald-400">
          Saved to <span className="max-w-md truncate font-mono">{saved[result.id]}</span>
          <button type="button" onClick={() => void api.openOutputsFolder()} className={cx("inline-flex items-center gap-1 rounded font-medium underline-offset-2 hover:underline", focusRing)}>
            <FolderOpen className="h-3.5 w-3.5" /> Show folder
          </button>
        </p>
      )}
      {error && (
        <div className="mx-auto w-full max-w-xl">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </div>
  );
}

const Strip = memo(function Strip({
  results,
  images,
  selectedId,
  pending,
}: {
  results: ResultImage[];
  images: Record<string, ImgRef>;
  selectedId: string | null;
  pending: number;
}) {
  const dispatch = useDispatch();
  return (
    <div className="shrink-0 border-t border-neutral-200 bg-white/60 px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900/40">
      <div className="flex gap-2 overflow-x-auto pb-1" role="listbox" aria-label="Images in this session" aria-orientation="horizontal">
        {Array.from({ length: pending }, (_, i) => (
          <div key={`p${i}`} className="pinhole-shimmer h-18 w-18 shrink-0 rounded-lg bg-neutral-200 dark:bg-neutral-800" aria-hidden />
        ))}
        {results.map((r) => {
          const img = images[r.id];
          if (!img) return null;
          const active = r.id === selectedId;
          return (
            <button
              key={r.id}
              type="button"
              role="option"
              aria-selected={active}
              aria-label={`Image ${r.width}×${r.height}, seed ${r.seed}`}
              onClick={() => dispatch({ type: "selectResult", id: r.id })}
              className={cx(
                "relative h-18 w-18 shrink-0 overflow-hidden rounded-lg ring-2 transition-all",
                focusRing,
                active ? "ring-amber-500" : "ring-transparent opacity-80 hover:opacity-100 hover:ring-neutral-300 dark:hover:ring-neutral-600",
              )}
            >
              <img src={img.url} alt="" className="h-full w-full object-cover" draggable={false} />
              {r.parentId && <span className="absolute right-1 bottom-1 rounded bg-black/60 px-1 text-[9px] font-medium text-white">UP</span>}
            </button>
          );
        })}
      </div>
    </div>
  );
});
