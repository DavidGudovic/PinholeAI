// Results: big preview of the selected image, its actions and settings summary,
// and a strip of every image made this session (in memory until Save).
import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import {
  ArrowLeftRight,
  ChevronLeft,
  Copy,
  ImageUp,
  Images,
  Layers,
  ChevronDown,
  Grid2x2,
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
import { StyleBadge } from "../../components/ModelPicker";
import { ImageViewer } from "../../components/ImageViewer";
import { Logo } from "../../components/Logo";
import { SideBySide } from "../../components/SideBySide";
import { Button, IconButton, Kbd, MenuItem, MenuSeparator, Popover, VramBadge, cx, focusRing } from "../../components/ui";
import * as api from "../../lib/api";
import type { CoreError, ResultImage } from "../../lib/types";
import { useShortcuts } from "../../lib/shortcuts";
import { useActions } from "../../lib/state/AppProvider";
import { createModels, sheetIds, takesReference, unsavedIds, type ImgRef } from "../../lib/state/model";
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
  // "Finish at Best quality": for pictures made below Best (not upscales).
  const canFinish = useAppState(
    (s) => result.kind !== "upscaled" && (s.batches[s.resultBatch[result.id]]?.request.dials?.quality ?? "best") !== "best",
  );
  const waits = busy ? " (waits for the current job)" : "";
  // The reference picture this result was made with, while it is still in memory.
  const reference = useAppState((s) => {
    const id = s.batches[s.resultBatch[result.id]]?.request.refImageIds?.[0];
    return id ? s.images[id] : undefined;
  });
  // "On another model": the result this one was made to compare with, while it is in memory.
  const compare = useAppState((s) => {
    const id = s.batches[s.resultBatch[result.id]]?.compareWith;
    return id && s.images[id] ? s.results.find((r) => r.id === id) : undefined;
  });
  const compareImg = useAppState((s) => (compare ? s.images[compare.id] : undefined));
  const [sideBySide, setSideBySide] = useState(!!compare);
  const [pickModel, setPickModel] = useState(false);
  // "Show tiled": a picture made with "Repeats without seams", four times, edges touching.
  const [tiledFor, setTiledFor] = useState<string | null>(null);
  const tiled = !!result.seamless && tiledFor === result.id;
  const tiledUrl = useTiledUrl(img.url, result.width, result.height, tiled);
  const unsavedCount = useAppState((s) => unsavedIds(s).length);
  // Joined to a string so the selector returns a stable value.
  const sheet = useAppState((s) => sheetIds(s, result.id).join(" "));
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
        {sideBySide && compare && compareImg ? (
          <SideBySide
            first={{ url: compareImg.url, width: compare.width, height: compare.height, label: compare.modelLabel === result.modelLabel ? "Before" : compare.modelLabel }}
            second={{ url: img.url, width: result.width, height: result.height, label: compare.modelLabel === result.modelLabel ? "This one" : result.modelLabel }}
          />
        ) : sideBySide && reference ? (
          <SideBySide
            first={{ ...reference, label: "Reference" }}
            second={{ url: img.url, width: result.width, height: result.height, label: "Result" }}
          />
        ) : (
          <img
            src={tiledUrl ?? img.url}
            alt={`Generated image, ${result.width}×${result.height}, seed ${result.seed}${tiledUrl ? ", shown tiled 2×2" : ""}`}
            className="max-h-full max-w-full rounded-lg object-contain shadow-lg ring-1 ring-black/5 dark:ring-white/10"
            draggable={false}
            onDoubleClick={onExpand}
          />
        )}
      </div>

      <div className="flex flex-wrap items-center justify-center gap-1.5">
        <div data-testid="result-actions" className="flex flex-wrap items-center justify-center gap-1.5">
          <SaveButton
            id={result.id}
            seed={result.seed}
            run={run}
            tab="create"
            unsavedCount={unsavedCount}
            onSaveAll={() => void run(() => actions.saveAll())}
            sheetCount={sheet ? sheet.split(" ").length : 0}
            onSaveSheet={() => void run(() => actions.saveSheet(sheet.split(" ")))}
          />
          <Button onClick={() => actions.sendToEdit(result.id)}>
            <WandSparkles className="h-4 w-4" /> Edit this
          </Button>
          <Popover
            width={pickModel ? 300 : 250}
            onOpenChange={(open) => {
              if (!open) setPickModel(false);
            }}
            trigger={(p) => (
              <Button {...p}>
                <Shuffle className="h-4 w-4" /> More like this{" "}
                <ChevronDown className="h-3.5 w-3.5 opacity-60" />
              </Button>
            )}
          >
            {(close) =>
              pickModel ? (
                <OtherModelList
                  resultId={result.id}
                  onBack={() => setPickModel(false)}
                  onClose={close}
                  onPick={(modelId) => {
                    close();
                    void run(() => actions.onOtherModel(result.id, modelId));
                  }}
                />
              ) : (
                <FocusFirst>
                  <MenuItem
                    icon={<Images className="h-4 w-4" />}
                    disabled={!hasBatch}
                    hint={hasBatch ? `Same layout, new details${waits}` : "Only for images made in this session"}
                    onClick={() => {
                      close();
                      void run(() => actions.closeTo(result.id));
                    }}
                  >
                    Close to this one
                  </MenuItem>
                  <MenuItem
                    icon={<Shuffle className="h-4 w-4" />}
                    disabled={!hasBatch}
                    hint={hasBatch ? `Same prompt, new layouts${waits}` : "Only for images made in this session"}
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
                  <MenuItem
                    icon={<ArrowLeftRight className="h-4 w-4" />}
                    disabled={!hasBatch}
                    hint={hasBatch ? "Same prompt and seed, shown side by side" : "Only for images made in this session"}
                    onClick={() => setPickModel(true)}
                  >
                    On another model…
                  </MenuItem>
                </FocusFirst>
              )
            }
          </Popover>
          <UpscaleMenu
            width={result.width}
            height={result.height}
            onPick={(f) => void run(() => actions.upscale(result.id, f))}
            onFinish={canFinish ? () => void run(() => actions.finishAtBest(result.id)) : undefined}
          />
          <Button
            title="Write a prompt from this picture"
            onClick={() => actions.sendToDescribe(result.id)}
          >
            <ScanText className="h-4 w-4" /> Describe
          </Button>
        </div>
        <div data-testid="result-icons" className="flex items-center gap-1.5">
          {(compare || reference) && (
            <IconButton
              label={compare ? `Side by side with the picture from ${compare.modelLabel}` : "Side by side with the reference picture"}
              variant="secondary"
              className={sideBySide ? "ring-2 ring-amber-500" : undefined}
              aria-pressed={sideBySide}
              onClick={() => setSideBySide((v) => !v)}
            >
              <PanelsLeftRight className="h-4 w-4" />
            </IconButton>
          )}
          {result.seamless && (
            <IconButton
              label="Show tiled"
              variant="secondary"
              className={tiled ? "ring-2 ring-amber-500" : undefined}
              aria-pressed={tiled}
              onClick={() => setTiledFor(tiled ? null : result.id)}
            >
              <Grid2x2 className="h-4 w-4" />
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

/**
 * "On another model…": installed models that can make the same picture (they fit this
 * computer, are set up, and take the reference picture when it used one).
 */
function OtherModelList({
  resultId,
  onBack,
  onClose,
  onPick,
}: {
  resultId: string;
  onBack: () => void;
  onClose: () => void;
  onPick: (modelId: string) => void;
}) {
  const actions = useActions();
  const models = useAppState((s) => s.models);
  const req = useAppState((s) => s.batches[s.resultBatch[resultId]]?.request);
  const usedReference = !!req?.refImageIds?.length;
  const list = createModels(models).filter(
    (m) => m.id !== req?.modelId && m.fit !== "tooBig" && !m.missingComponents.length && (!usedReference || takesReference(m)),
  );
  return (
    <FocusFirst role="menu" label="On another model">
      <MenuItem icon={<ChevronLeft className="h-4 w-4" />} onClick={onBack}>
        On another model
      </MenuItem>
      <MenuSeparator />
      {list.length ? (
        list.map((m) => (
          <MenuItem
            key={m.id}
            onClick={() => onPick(m.id)}
            hint={
              <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span>{m.familyLabel ?? "Unknown family"}</span>
                <VramBadge vram={m.vram} fit={m.fit} />
              </span>
            }
          >
            <span className="flex items-center gap-2">
              <span className="truncate font-medium">{m.friendlyName}</span>
              <StyleBadge badge={m.styleBadge} />
            </span>
          </MenuItem>
        ))
      ) : (
        <p className="px-2.5 py-2 text-xs text-neutral-500">
          {usedReference ? "No other installed model can use the reference picture." : "No other installed model fits this computer."}
        </p>
      )}
      <MenuSeparator />
      <MenuItem icon={<Layers className="h-4 w-4" />} onClick={() => {
          onClose();
          actions.setTab("models");
        }}
      >
        Get more models…
      </MenuItem>
    </FocusFirst>
  );
}

/** Moves focus into a menu whose items just replaced the ones that had it. */
function FocusFirst({ children, role, label }: { children: ReactNode; role?: string; label?: string }) {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.querySelector<HTMLElement>("[role=menuitem]:not(:disabled)")?.focus();
  }, []);
  return (
    <div ref={box} role={role} aria-label={label}>
      {children}
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

const TILED_MAX_SIDE = 4096;

/** The picture at `url` drawn 2×2 into one image (object URL), while `on`. */
function useTiledUrl(url: string, width: number, height: number, on: boolean): string | null {
  const [tiledUrl, setTiledUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!on) return;
    let made: string | null = null;
    let live = true;
    const pic = new Image();
    pic.onload = () => {
      // A preview only: at most 4096 px a side, well inside every webview's canvas limit.
      const scale = Math.min(1, TILED_MAX_SIDE / (2 * Math.max(width, height)));
      const w = Math.max(1, Math.round(width * scale));
      const h = Math.max(1, Math.round(height * scale));
      const canvas = document.createElement("canvas");
      canvas.width = w * 2;
      canvas.height = h * 2;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      for (const [x, y] of [[0, 0], [w, 0], [0, h], [w, h]]) ctx.drawImage(pic, x, y, w, h);
      canvas.toBlob((blob) => {
        if (!blob || !live) return;
        made = URL.createObjectURL(blob);
        setTiledUrl(made);
      });
    };
    pic.src = url;
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
      setTiledUrl(null);
    };
  }, [url, width, height, on]);
  return on ? tiledUrl : null;
}
