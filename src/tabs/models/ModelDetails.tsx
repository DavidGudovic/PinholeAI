// Model details page (SPEC §5.4 "Model details"): the model's CivitAI preview
// images. Each image can send its settings to Create or itself to Edit, and
// the model's CivitAI page opens in the system browser.
// PRIVACY: image generation data (prompts) stays in memory; nothing is logged.
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, Check, Download, ExternalLink, EyeOff, ImageOff, RotateCw, ShieldAlert, SlidersHorizontal, ThumbsUp, Wand2 } from "lucide-react";
import * as api from "../../lib/api";
import type { CatalogCard, ContentMode, CoreError, GalleryItem, ModelGallery } from "../../lib/types";
import { formatBytes, formatCount } from "../../lib/format";
import { galleryGenerationText, gallerySettingsSummary } from "../../lib/paste/fromGallery";
import { useActions } from "../../lib/state/AppProvider";
import { createModels } from "../../lib/state/model";
import { useAppState } from "../../lib/state/store";
import { Badge, Button, Dialog, ErrorNotice, Spinner } from "../../components/ui";
import { sendGenerationToCreate } from "../create/handoff";
import { GroupProgress, Skeleton, VramLine } from "./controls";
import { cancelGroup, useTaggedGroup } from "./lib/downloads";
import { isActive, ratioPercent } from "./lib/words";

// Image loading is local to this page (not ./lib/preview) so it doesn't depend
// on the Browse grid's loader. At most a few fetches run at once; bytes come
// from Rust and live only as blob: URLs while the page is open.
const MAX_PARALLEL = 4;
let running = 0;
const waiting: (() => void)[] = [];
async function limited<T>(job: () => Promise<T>): Promise<T> {
  if (running >= MAX_PARALLEL) await new Promise<void>((r) => waiting.push(r));
  running++;
  try {
    return await job();
  } finally {
    running--;
    waiting.shift()?.();
  }
}

function useImage(url: string | null): { src: string | null; failed: boolean } {
  const [state, setState] = useState<{ src: string | null; failed: boolean }>({ src: null, failed: false });
  useEffect(() => {
    if (!url) return;
    let alive = true;
    let objectUrl: string | null = null;
    setState({ src: null, failed: false });
    limited(() => (alive ? api.fetchPreview(url) : Promise.resolve(null)))
      .then((buf) => {
        if (!alive || !buf) return;
        objectUrl = URL.createObjectURL(new Blob([buf]));
        setState({ src: objectUrl, failed: false });
      })
      .catch(() => alive && setState({ src: null, failed: true }));
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [url]);
  return state;
}

export function ModelDetails({
  card,
  content,
  installed,
  onInstall,
  onClose,
}: {
  card: CatalogCard;
  content: ContentMode;
  installed: boolean;
  onInstall: (card: CatalogCard) => void;
  onClose: () => void;
}) {
  const actions = useActions();
  const models = useAppState((s) => s.models);
  const [gallery, setGallery] = useState<ModelGallery | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [reload, setReload] = useState(0);
  const [open, setOpen] = useState<GalleryItem | null>(null);
  const [busy, setBusy] = useState<"edit" | null>(null);
  const [actionError, setActionError] = useState<CoreError | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const isLora = card.type.toUpperCase() === "LORA";

  useEffect(() => {
    let alive = true;
    setGallery(null);
    setError(null);
    api
      .modelGallery(card.versionId, content, card.modelNsfw)
      .then((g) => alive && setGallery(g))
      .catch((e) => alive && setError(api.asCoreError(e)));
    return () => {
      alive = false;
    };
  }, [card.versionId, card.modelNsfw, content, reload]);

  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  // Esc goes back (unless the image viewer is open: it closes itself first).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !open && !document.querySelector("[role=dialog]")) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const hasCreateModel = useMemo(() => createModels(models).length > 0, [models]);

  const useSettings = (item: GalleryItem) => {
    const text = galleryGenerationText(item.generation, {
      type: card.type,
      versionId: card.versionId,
      modelName: card.name,
      versionName: card.versionName,
    });
    if (!text) return;
    if (!hasCreateModel) {
      actions.toast("Install a model first. Its settings can be used once it's on this computer.");
      return;
    }
    sendGenerationToCreate(text);
    setOpen(null);
    actions.setTab("create");
  };

  const editImage = async (item: GalleryItem) => {
    setBusy("edit");
    setActionError(null);
    try {
      let buf: ArrayBuffer;
      try {
        buf = await api.fetchPreview(item.fullUrl);
      } catch {
        // The full-size file can be too big to fetch; the grid rendition still works.
        buf = await api.fetchPreview(item.thumbUrl);
      }
      await actions.importToEdit(new Blob([buf]));
      setOpen(null);
      actions.setTab("edit");
    } catch (e) {
      setActionError(api.asCoreError(e));
    } finally {
      setBusy(null);
    }
  };

  const openCivitai = () => void api.openCivitaiPage(card.modelId, card.versionId, card.modelNsfw).catch((e) => actions.toast(api.asCoreError(e).message));

  return (
    <div ref={rootRef} tabIndex={-1} className="absolute inset-0 z-20 overflow-y-auto bg-neutral-50 outline-none dark:bg-neutral-950" aria-label={`${card.name} details`} role="region">
      <div className="mx-auto max-w-7xl space-y-5 px-6 py-5">
        <div className="flex items-center justify-between gap-3">
          <Button variant="ghost" onClick={onClose}>
            <ArrowLeft className="h-4 w-4" /> Back to models
          </Button>
          <Button onClick={openCivitai} title={card.modelNsfw ? "Opens civitai.red in your browser" : "Opens civitai.com in your browser"}>
            <ExternalLink className="h-4 w-4" /> Open on CivitAI
          </Button>
        </div>

        <Header card={card} installed={installed} isLora={isLora} onInstall={onInstall} trainedWords={gallery?.trainedWords ?? []} />

        <section className="space-y-3">
          <div>
            <h2 className="text-base font-semibold">Example images</h2>
            <p className="text-sm text-neutral-500">Made with this model by its creator. Click one to use its settings or edit it.</p>
          </div>
          {actionError && <ErrorNotice error={actionError} onDismiss={() => setActionError(null)} />}
          {error ? (
            <div className="max-w-lg space-y-3">
              <ErrorNotice error={error} />
              <Button onClick={() => setReload((r) => r + 1)}>
                <RotateCw className="h-4 w-4" /> Try again
              </Button>
            </div>
          ) : !gallery ? (
            <div className="columns-[220px] gap-4">
              {Array.from({ length: 8 }, (_, i) => (
                <Skeleton key={i} className={`mb-4 w-full break-inside-avoid ${i % 3 === 0 ? "aspect-[3/4]" : i % 3 === 1 ? "aspect-[2/3]" : "aspect-square"}`} />
              ))}
            </div>
          ) : gallery.offline ? (
            <p className="text-sm text-neutral-500">Offline mode is on, so Pinhole doesn't contact CivitAI. Turn it off in Settings to see example images.</p>
          ) : gallery.items.length === 0 ? (
            <p className="text-sm text-neutral-500">
              {gallery.hiddenNsfw > 0 ? "Every example image for this model is 18+, and 18+ content is off." : "This model has no example images."}
            </p>
          ) : (
            <>
              <div className="columns-[220px] gap-4">
                {gallery.items.map((it) => (
                  <Tile key={it.index} item={it} blur={it.nsfw && content === "safe"} onOpen={() => setOpen(it)} />
                ))}
              </div>
              {gallery.hiddenNsfw > 0 && (
                <p className="text-xs text-neutral-500">
                  {gallery.hiddenNsfw === 1 ? "1 image is" : `${gallery.hiddenNsfw} images are`} hidden because 18+ content is off.
                </p>
              )}
            </>
          )}
        </section>
      </div>

      <Viewer item={open} busy={busy} onClose={() => setOpen(null)} onUseSettings={useSettings} onEdit={(it) => void editImage(it)} />
    </div>
  );
}

function Header({
  card,
  installed,
  isLora,
  onInstall,
  trainedWords,
}: {
  card: CatalogCard;
  installed: boolean;
  isLora: boolean;
  onInstall: (card: CatalogCard) => void;
  trainedWords: string[];
}) {
  const group = useTaggedGroup(`civitai:${card.versionId}`);
  const downloading = !!group && isActive(group);
  const ratio = ratioPercent(card.thumbsUpRatio);

  let action: ReactNode;
  if (card.blockedReason)
    action = (
      <p className="flex items-start gap-1.5 rounded-lg bg-red-50 p-2 text-xs text-red-800 dark:bg-red-950/40 dark:text-red-300">
        <ShieldAlert className="mt-px h-3.5 w-3.5 shrink-0" />
        {card.blockedReason}
      </p>
    );
  else if (downloading && group) action = <GroupProgress group={group} onCancel={() => void cancelGroup(group.groupId)} />;
  else if (installed)
    action = (
      <div className="flex h-9 items-center justify-center gap-1.5 rounded-lg bg-emerald-50 px-4 text-sm font-medium text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-400">
        <Check className="h-4 w-4" /> Installed
      </div>
    );
  else
    action = (
      <Button variant="primary" onClick={() => onInstall(card)}>
        <Download className="h-4 w-4" /> {group?.state === "failed" ? "Try again" : "Install"}
      </Button>
    );

  return (
    <div className="flex flex-wrap items-start justify-between gap-4 rounded-xl border border-neutral-200 bg-white p-4 shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
      <div className="min-w-0 flex-1 space-y-2">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{card.name}</h1>
          <p className="text-sm text-neutral-500">
            {card.versionName} · {card.baseModel}
            {card.creator ? ` · by ${card.creator}` : ""}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-neutral-600 dark:text-neutral-400">
          {ratio && (
            <span className="inline-flex items-center gap-1" title="Share of thumbs-up ratings">
              <ThumbsUp className="h-4 w-4" /> {ratio}
            </span>
          )}
          <span className="inline-flex items-center gap-1" title="Downloads">
            <Download className="h-4 w-4" /> {formatCount(card.downloadCount)}
          </span>
          {card.downloadBytes != null && <span title="Download size">{formatBytes(card.downloadBytes)}</span>}
          {card.styleBadge && <Badge>{card.styleBadge}</Badge>}
          {isLora && <Badge>Style add-on</Badge>}
          {card.commercialOk ? <Badge tone="green">OK for client work</Badge> : <Badge>Not for client work</Badge>}
        </div>
        {card.vram ? <VramLine vram={card.vram} fit={card.fit} /> : isLora ? <p className="text-xs text-neutral-500">Adds a look to {card.baseModel} models</p> : null}
        {card.licenseNote && <p className="text-xs text-neutral-500">License: {card.licenseNote}</p>}
        {isLora && trainedWords.length > 0 && (
          <p className="text-xs text-neutral-500">
            Trigger words: <span className="font-medium text-neutral-700 dark:text-neutral-300">{trainedWords.join(", ")}</span>
          </p>
        )}
      </div>
      <div className="w-full max-w-60 shrink-0 sm:w-60">{action}</div>
    </div>
  );
}

const Tile = memo(function Tile({ item, blur, onOpen }: { item: GalleryItem; blur: boolean; onOpen: () => void }) {
  const preview = useImage(item.thumbUrl);
  const ratio = item.width && item.height ? `${item.width} / ${item.height}` : "3 / 4";
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={item.generation ? "Open image (settings available)" : "Open image"}
      className="group relative mb-4 block w-full break-inside-avoid overflow-hidden rounded-xl bg-neutral-100 focus-visible:ring-2 focus-visible:ring-amber-500/70 focus-visible:outline-none dark:bg-neutral-800"
      style={{ aspectRatio: ratio }}
    >
      {preview.src ? (
        <img src={preview.src} alt="" draggable={false} className={`h-full w-full object-cover transition-transform group-hover:scale-[1.02] ${blur ? "scale-125 blur-2xl" : ""}`} />
      ) : preview.failed ? (
        <span className="absolute inset-0 flex items-center justify-center text-neutral-400">
          <ImageOff className="h-6 w-6" />
        </span>
      ) : (
        <span className="absolute inset-0 animate-pulse bg-neutral-200 dark:bg-neutral-800" />
      )}
      {blur && preview.src && (
        <span className="absolute inset-0 flex items-center justify-center">
          <span className="inline-flex items-center gap-1.5 rounded-full bg-black/60 px-2.5 py-1 text-xs font-medium text-white">
            <EyeOff className="h-3.5 w-3.5" /> 18+ preview hidden
          </span>
        </span>
      )}
      {item.generation && (
        <span className="absolute bottom-2 left-2 inline-flex items-center gap-1 rounded-md bg-black/55 px-1.5 py-0.5 text-[11px] font-medium text-white backdrop-blur-sm">
          <SlidersHorizontal className="h-3 w-3" /> Settings
        </span>
      )}
    </button>
  );
});

function Viewer({
  item,
  busy,
  onClose,
  onUseSettings,
  onEdit,
}: {
  item: GalleryItem | null;
  busy: "edit" | null;
  onClose: () => void;
  onUseSettings: (item: GalleryItem) => void;
  onEdit: (item: GalleryItem) => void;
}) {
  const preview = useImage(item?.fullUrl ?? null);
  const small = useImage(item?.thumbUrl ?? null);
  const src = preview.src ?? small.src;
  const g = item?.generation ?? null;
  const prompt = typeof g?.prompt === "string" ? g.prompt : null;
  const negative = typeof g?.negativePrompt === "string" ? g.negativePrompt : null;
  const summary = gallerySettingsSummary(g);
  return (
    <Dialog
      open={!!item}
      onClose={onClose}
      wide
      title="Example image"
      footer={
        item && (
          <>
            <Button onClick={() => onEdit(item)} disabled={busy != null}>
              {busy === "edit" ? <Spinner className="h-4 w-4" /> : <Wand2 className="h-4 w-4" />} Edit this image
            </Button>
            <Button variant="primary" onClick={() => onUseSettings(item)} disabled={!g} title={g ? undefined : "The creator didn't share how this image was made"} data-autofocus>
              <SlidersHorizontal className="h-4 w-4" /> Use these settings
            </Button>
          </>
        )
      }
    >
      {item && (
        <div className="grid gap-4 md:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
          <div className="flex max-h-[60vh] items-center justify-center overflow-hidden rounded-lg bg-neutral-100 dark:bg-neutral-800">
            {src ? <img src={src} alt="" draggable={false} className="max-h-[60vh] w-auto object-contain" /> : <Spinner className="m-16 h-5 w-5 text-neutral-400" />}
          </div>
          <div className="min-w-0 space-y-3 text-sm">
            {g ? (
              <>
                {summary.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {summary.map((s) => (
                      <Badge key={s}>{s}</Badge>
                    ))}
                  </div>
                )}
                {prompt && (
                  <div>
                    <p className="text-xs font-medium text-neutral-500">Prompt</p>
                    <p className="max-h-40 overflow-y-auto break-words whitespace-pre-wrap select-text">{prompt}</p>
                  </div>
                )}
                {negative && (
                  <div>
                    <p className="text-xs font-medium text-neutral-500">Avoid</p>
                    <p className="max-h-24 overflow-y-auto break-words whitespace-pre-wrap text-neutral-600 select-text dark:text-neutral-400">{negative}</p>
                  </div>
                )}
                <p className="text-xs text-neutral-500">"Use these settings" fills in Create with this prompt and settings. Nothing is sent anywhere.</p>
              </>
            ) : (
              <p className="text-neutral-500">The creator didn't share how this image was made. You can still edit the image.</p>
            )}
          </div>
        </div>
      )}
    </Dialog>
  );
}
