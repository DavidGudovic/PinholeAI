// Results: big preview of the selected image, its actions and settings summary,
// and a strip of every image made this session (in memory until Save).
import { memo, useEffect, useState } from "react";
import {
  Copy,
  ImageUp,
  Layers,
  ChevronDown,
  Maximize2,
  PanelsLeftRight,
  ScanText,
  Shuffle,
  Trash,
  UserRound,
  WandSparkles,
} from "lucide-react";
import { CheckReadings } from "../../components/CheckReadings";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { SaveButton, UpscaleMenu } from "../../components/ImageActions";
import { ImageViewer } from "../../components/ImageViewer";
import { Logo } from "../../components/Logo";
import { SideBySide } from "../../components/SideBySide";
import { Button, IconButton, Kbd, MenuItem, Popover, cx, focusRing } from "../../components/ui";
import * as api from "../../lib/api";
import type { CoreError, ResultImage } from "../../lib/types";
import { useShortcuts } from "../../lib/shortcuts";
import { useActions } from "../../lib/state/AppProvider";
import { unsavedIds, type ImgRef } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import { settingsSummary } from "../../lib/state/request";
import { TipLine } from "./TipLine";
import { useAppState, useDispatch } from "../../lib/state/store";

export function Results() {
  const results = useAppState((s) => s.results);
  const selectedId = useAppState((s) => s.selectedResultId);
  const images = useAppState((s) => s.images);
  // Only the job's kind and image count: progress ticks must not re-render the results.
  const jobKind = useAppState((s) => s.job?.kind ?? null);
  const jobCount = useAppState((s) => s.job?.count ?? 1);
  const selected = results.find((r) => r.id === selectedId) ?? null;
  const generating = jobKind === "create" || jobKind === "upscale";
  const pending = generating ? jobCount : 0;
  const dispatch = useDispatch();
  const [viewing, setViewing] = useState(false);
  const viewable = results.filter((r) => images[r.id]);
  // Removing the last image closes the viewer for good.
  useEffect(() => {
    if (!selected) setViewing(false);
  }, [selected]);
  const viewIndex = Math.max(
    0,
    viewable.findIndex((r) => r.id === selectedId),
  );

  return (
    <section
      aria-label="Results"
      className="flex min-h-0 min-w-0 flex-1 flex-col"
    >
      {selected && images[selected.id] ? (
        <Preview
          key={selected.id}
          result={selected}
          img={images[selected.id]}
          onExpand={() => setViewing(true)}
        />
      ) : generating ? (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6">
          <div className="pinhole-shimmer aspect-square w-full max-w-md rounded-2xl bg-neutral-200 dark:bg-neutral-900" />
        </div>
      ) : (
        <EmptyResults />
      )}
      {(results.length > 0 || pending > 0) && (
        <Strip
          results={results}
          images={images}
          selectedId={selectedId}
          pending={pending}
        />
      )}
      {viewing && selected && viewable.length > 0 && (
        <ImageViewer
          images={viewable.map((r) => ({
            url: images[r.id].url,
            width: r.width,
            height: r.height,
            alt: `Generated image, ${r.width}×${r.height}, seed ${r.seed}`,
          }))}
          index={viewIndex}
          onIndex={(i) =>
            dispatch({ type: "selectResult", id: viewable[i].id })
          }
          onClose={() => setViewing(false)}
        />
      )}
    </section>
  );
}

function EmptyResults() {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-8 text-center">
      <div className="mb-5 rounded-3xl bg-white p-5 shadow-sm ring-1 ring-neutral-200 dark:bg-neutral-900 dark:ring-neutral-800">
        <Logo className="h-12 w-12" />
      </div>
      <h2 className="text-lg font-semibold tracking-tight">
        Your images appear here
      </h2>
      <p className="mt-1.5 max-w-sm text-sm text-neutral-500">
        Describe what you want to see, then press Generate. Drop a picture you
        saved with Pinhole here to reuse its settings.
      </p>
      <p className="mt-3 inline-flex items-center gap-1 text-xs text-neutral-400">
        <Kbd>{modKey}</Kbd>
        <Kbd>Enter</Kbd>
        <span className="ml-1">generates from anywhere in this tab</span>
      </p>
    </div>
  );
}

function Preview({
  result,
  img,
  onExpand,
}: {
  result: ResultImage;
  img: ImgRef;
  onExpand: () => void;
}) {
  const actions = useActions();
  const busy = useAppState((s) => !!s.job);
  const hasBatch = useAppState((s) => !!s.resultBatch[result.id]);
  // The reference picture this result was made with, while it is still in memory.
  const reference = useAppState((s) => {
    const id = s.batches[s.resultBatch[result.id]]?.request.refImageIds?.[0];
    return id ? s.images[id] : undefined;
  });
  const [sideBySide, setSideBySide] = useState(false);
  const unsavedCount = useAppState((s) => unsavedIds(s).length);
  const [error, setError] = useState<CoreError | null>(null);

  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(api.asCoreError(e));
    }
  };

  useShortcuts("create", {
    edit: () => actions.sendToEdit(result.id),
    describe: () => actions.sendToDescribe(result.id),
    fullscreen: onExpand,
  });

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 px-6 pt-5 pb-3">
      <div className="relative flex min-h-0 flex-1 items-center justify-center">
        {sideBySide && reference ? (
          <SideBySide
            first={{ ...reference, label: "Reference" }}
            second={{ url: img.url, width: result.width, height: result.height, label: "Result" }}
          />
        ) : (
          <img
            src={img.url}
            alt={`Generated image, ${result.width}×${result.height}, seed ${result.seed}`}
            className="max-h-full max-w-full rounded-lg object-contain shadow-lg ring-1 ring-black/5 dark:ring-white/10"
            draggable={false}
            onDoubleClick={onExpand}
          />
        )}
      </div>

      <div className="flex flex-wrap items-center justify-center gap-1.5">
        <SaveButton
          id={result.id}
          seed={result.seed}
          run={run}
          tab="create"
          unsavedCount={unsavedCount}
          onSaveAll={() => void run(() => actions.saveAll())}
        />
        <Button onClick={() => actions.sendToEdit(result.id)}>
          <WandSparkles className="h-4 w-4" /> Edit this
        </Button>
        <Popover
          width={250}
          trigger={(p) => (
            <Button {...p}>
              <Shuffle className="h-4 w-4" /> More like this{" "}
              <ChevronDown className="h-3.5 w-3.5 opacity-60" />
            </Button>
          )}
        >
          {(close) => (
            <>
              <MenuItem
                icon={<Shuffle className="h-4 w-4" />}
                disabled={!hasBatch}
                hint={
                  hasBatch
                    ? busy
                      ? "Same prompt, new seeds (waits for the current job)"
                      : "Same prompt, new seeds"
                    : "Only for images made in this session"
                }
                onClick={() => {
                  close();
                  void run(() => actions.variations(result.id));
                }}
              >
                Variations
              </MenuItem>
              <MenuItem
                icon={<UserRound className="h-4 w-4" />}
                hint="This character or subject in a different scene"
                onClick={() => {
                  close();
                  actions.sameCharacter(result.id);
                }}
              >
                Same character
              </MenuItem>
            </>
          )}
        </Popover>
        <UpscaleMenu
          width={result.width}
          height={result.height}
          disabled={busy}
          onPick={(f) => void run(() => actions.upscale(result.id, f))}
        />
        <Button
          title="Write a prompt from this picture"
          onClick={() => actions.sendToDescribe(result.id)}
        >
          <ScanText className="h-4 w-4" /> Describe
        </Button>
        {reference && (
          <IconButton
            label="Side by side with the reference picture"
            variant="secondary"
            className={sideBySide ? "ring-2 ring-amber-500" : undefined}
            aria-pressed={sideBySide}
            onClick={() => setSideBySide((v) => !v)}
          >
            <PanelsLeftRight className="h-4 w-4" />
          </IconButton>
        )}
        <IconButton
          label="View full screen"
          variant="secondary"
          onClick={onExpand}
        >
          <Maximize2 className="h-4 w-4" />
        </IconButton>
        <IconButton
          label="Copy image"
          variant="secondary"
          onClick={() => void run(() => actions.copyImage(result.id))}
        >
          <Copy className="h-4 w-4" />
        </IconButton>
        <IconButton
          label="Remove from this session"
          variant="ghost"
          onClick={() => actions.removeResult(result.id)}
        >
          <Trash className="h-4 w-4" />
        </IconButton>
      </div>

      <p className="text-center text-xs text-neutral-500 tabular-nums">
        {settingsSummary(result)}
      </p>
      {import.meta.env.DEV && <CheckReadings id={result.id} />}
      <TipLine hasBatch={hasBatch} />
      {error && (
        <div className="mx-auto w-full max-w-xl">
          <ErrorWithFix error={error} onDismiss={() => setError(null)} />
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
  const actions = useActions();
  return (
    <div className="shrink-0 border-t border-neutral-200 bg-white/60 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
      <div
        className="flex gap-2 overflow-x-auto p-1"
        role="listbox"
        aria-label="Images in this session"
        aria-orientation="horizontal"
      >
        {Array.from({ length: pending }, (_, i) => (
          <div
            key={`p${i}`}
            className="pinhole-shimmer h-18 w-18 shrink-0 rounded-lg bg-neutral-200 dark:bg-neutral-800"
            aria-hidden
          />
        ))}
        {results.map((r) => {
          const img = images[r.id];
          if (!img) return null;
          const active = r.id === selectedId;
          return (
            <div
              key={r.id}
              role="presentation"
              className="group relative h-18 w-18 shrink-0"
            >
              <button
                type="button"
                role="option"
                aria-selected={active}
                aria-label={`Image ${r.width}×${r.height}, seed ${r.seed}${r.parentId ? ", upscaled" : ""}`}
                onClick={() => dispatch({ type: "selectResult", id: r.id })}
                className={cx(
                  "relative h-full w-full overflow-hidden rounded-lg ring-2 transition-all",
                  focusRing,
                  active
                    ? "ring-amber-500"
                    : "ring-transparent opacity-80 hover:opacity-100 hover:ring-neutral-300 dark:hover:ring-neutral-600",
                )}
              >
                <img
                  src={img.url}
                  alt=""
                  className="h-full w-full object-cover"
                  draggable={false}
                />
                {r.parentId && (
                  <span
                    className="absolute right-1 bottom-1 rounded bg-black/60 p-0.5 text-white"
                    title="Upscaled"
                    aria-hidden
                  >
                    <ImageUp className="h-3 w-3" />
                  </span>
                )}
              </button>
              <button
                type="button"
                title="Use as image 2 in Edit"
                aria-label="Use as image 2 in Edit"
                onClick={() => actions.sendToEditSecond(r.id)}
                className={cx(
                  "absolute top-1 left-1 rounded bg-black/60 p-0.5 text-white opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100",
                  focusRing,
                )}
              >
                <Layers className="h-3 w-3" aria-hidden />
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
});
