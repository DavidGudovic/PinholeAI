// One CivitAI model card in Browse (SPEC §5.4 "Model card").
import { memo, useRef, type ReactNode } from "react";
import { Check, Download, EyeOff, Film, HardDrive, ImageOff, RotateCw, ShieldAlert, ThumbsUp } from "lucide-react";
import type { CatalogCard, ContentMode } from "../../lib/types";
import { formatBytes, formatCount } from "../../lib/format";
import { Badge, Button } from "../../components/ui";
import { GroupProgress, VramLine } from "./controls";
import { cancelGroup, useTaggedGroup } from "./lib/downloads";
import { usePreviewBlob, useVisibility } from "./lib/preview";
import { isVideoFile, shouldBlurPreview } from "./lib/query";
import { isActive, ratioPercent } from "./lib/words";
import { UseAddonButton, useInstalledLoraId } from "./UseAddon";

function Overlay({ tone = "dark", children }: { tone?: "dark" | "amber" | "green"; children: ReactNode }) {
  const tones = {
    dark: "bg-black/55 text-white",
    amber: "bg-amber-400 text-neutral-950",
    green: "bg-emerald-500 text-white",
  };
  return <span className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium backdrop-blur-sm ${tones[tone]}`}>{children}</span>;
}

function Placeholder({ icon, text }: { icon: ReactNode; text: string }) {
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
      {icon}
      {text}
    </div>
  );
}

export const CatalogCardView = memo(function CatalogCardView({
  card,
  content,
  showPrice,
  installed,
  onInstall,
  onOpen,
}: {
  card: CatalogCard;
  content: ContentMode;
  showPrice: boolean;
  installed: boolean;
  onInstall: (card: CatalogCard) => void;
  /** Open the model's details page. */
  onOpen: (card: CatalogCard) => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const visibility = useVisibility(ref);
  const blur = shouldBlurPreview(card, content);
  // Video previews arrive as a still frame (Rust rewrites the URL); a bare video file is never fetched.
  const videoFile = isVideoFile(card.previewUrl);
  const preview = usePreviewBlob(videoFile ? null : card.previewUrl, visibility);
  const group = useTaggedGroup(`civitai:${card.versionId}`);
  const downloading = !!group && isActive(group);
  const isLora = card.type.toUpperCase() === "LORA";
  const ratio = ratioPercent(card.thumbsUpRatio);
  const loraId = useInstalledLoraId(isLora && installed ? card.versionId : null);

  let action: ReactNode;
  if (card.blockedReason)
    action = (
      <p className="flex items-start gap-1.5 rounded-lg bg-red-50 p-2 text-xs text-red-800 dark:bg-red-950/40 dark:text-red-300">
        <ShieldAlert className="mt-px h-3.5 w-3.5 shrink-0" />
        {card.blockedReason}
      </p>
    );
  else if (downloading && group) action = <GroupProgress group={group} compact onCancel={() => void cancelGroup(group.groupId)} />;
  else if (installed)
    action = (
      <div className="flex items-center gap-1.5">
        <div className="flex h-8 flex-1 items-center justify-center gap-1.5 rounded-lg bg-emerald-50 text-xs font-medium text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-400">
          <Check className="h-3.5 w-3.5" /> Installed
        </div>
        {loraId && <UseAddonButton loraId={loraId} name={card.name} className="h-8" />}
      </div>
    );
  else if (group?.state === "failed")
    action = (
      <div className="space-y-1.5">
        <p className="line-clamp-2 text-xs text-red-600 dark:text-red-400">{group.error ?? "The download failed."}</p>
        <Button size="sm" className="w-full" onClick={() => onInstall(card)}>
          <RotateCw className="h-3.5 w-3.5" /> Try again
        </Button>
      </div>
    );
  else
    action = (
      <Button variant="primary" size="sm" className="h-8 w-full" onClick={() => onInstall(card)} aria-label={`Install ${card.name}`}>
        <Download className="h-3.5 w-3.5" /> Install
      </Button>
    );

  return (
    // content-visibility: the browser skips layout/paint for cards far off screen.
    // The border and background are drawn by a frame behind the content, and only the picture
    // is clipped to the rounded corners, not the whole card (on Linux the card itself has
    // square corners and no shadow, see lib/platform.ts: WebKitGTK repaints every card on
    // screen for each scroll frame, and a rounded clip around a card is slow to repaint).
    <article className="pinhole-card relative isolate flex flex-col rounded-xl p-px shadow-sm transition-shadow [contain-intrinsic-size:auto_440px] [content-visibility:auto] hover:shadow-md">
      <div aria-hidden className="absolute inset-0 -z-10 rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900" />
      <button
        ref={ref}
        type="button"
        onClick={() => onOpen(card)}
        aria-label={`Show ${card.name} details`}
        className="relative block aspect-[4/5] w-full cursor-pointer overflow-hidden rounded-t-[11px] bg-neutral-100 focus-visible:ring-2 focus-visible:ring-amber-500/70 focus-visible:outline-none focus-visible:ring-inset dark:bg-neutral-800"
      >
        {preview.src ? (
          <img src={preview.src} alt="" draggable={false} decoding="async" className={`h-full w-full object-cover ${blur ? "scale-125 blur-2xl" : ""}`} />
        ) : card.previewIsVideo && (videoFile || preview.failed) ? (
          <Placeholder icon={<Film className="h-6 w-6" />} text="Video preview" />
        ) : preview.failed || !card.previewUrl ? (
          <Placeholder icon={<ImageOff className="h-6 w-6" />} text="No preview" />
        ) : (
          <div className="absolute inset-0 animate-pulse bg-neutral-200 dark:bg-neutral-800" />
        )}
        {blur && preview.src && (
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="inline-flex items-center gap-1.5 rounded-full bg-black/60 px-2.5 py-1 text-xs font-medium text-white">
              <EyeOff className="h-3.5 w-3.5" /> Preview hidden by Safe mode
            </span>
          </div>
        )}
        <div className="absolute top-2 left-2 flex flex-wrap gap-1">
          {card.styleBadge && <Overlay>{card.styleBadge}</Overlay>}
          {isLora && <Overlay>Style add-on</Overlay>}
          {card.sfwOnly && <Overlay>Safe images only</Overlay>}
          {card.previewIsVideo && preview.src && (
            <Overlay>
              <Film className="h-3 w-3" /> Video
            </Overlay>
          )}
        </div>
        <div className="absolute top-2 right-2 flex flex-col items-end gap-1">
          {showPrice && card.earlyAccess && <Overlay tone="amber">Early access · paid</Overlay>}
          {installed && (
            <Overlay tone="green">
              <Check className="h-3 w-3" /> Installed
            </Overlay>
          )}
        </div>
      </button>

      <div className="flex flex-1 flex-col gap-2 p-3">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-neutral-900 dark:text-neutral-50" title={card.name}>
            <button type="button" onClick={() => onOpen(card)} className="block max-w-full truncate text-left hover:underline focus-visible:underline focus-visible:outline-none">
              {card.name}
            </button>
          </h3>
          <p className="truncate text-[11px] text-neutral-500" title={`${card.baseModel} · ${card.versionName}`}>
            {card.baseModel} · {card.versionName}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-neutral-600 dark:text-neutral-400">
          {ratio && (
            <span className="inline-flex items-center gap-1" title="Share of thumbs-up ratings">
              <ThumbsUp className="h-3.5 w-3.5" /> {ratio}
            </span>
          )}
          <span className="inline-flex items-center gap-1" title="Downloads">
            <Download className="h-3.5 w-3.5" /> {formatCount(card.downloadCount)}
          </span>
          {card.downloadBytes != null && (
            <span className="inline-flex items-center gap-1" title="Download size">
              <HardDrive className="h-3.5 w-3.5" /> {formatBytes(card.downloadBytes)}
            </span>
          )}
        </div>

        {card.vram ? (
          <div>
            <VramLine vram={card.vram} fit={card.fit} />
            {card.smallerFile && <p className="mt-0.5 text-[11px] text-neutral-500">{card.smallerFile} version, so it fits your card</p>}
          </div>
        ) : isLora ? (
          <span className="text-xs text-neutral-500">Adds a look to {card.baseModel} models</span>
        ) : null}

        <div className="flex flex-wrap items-center gap-1">
          {card.commercialOk ? <Badge tone="green">OK for client work</Badge> : <Badge>Not for client work</Badge>}
          {card.takesReference && (
            <Badge tone="blue" title="In Create, this model can follow a picture you add under the prompt">
              Reference picture
            </Badge>
          )}
        </div>
        {card.licenseNote && (
          <p className="truncate text-[11px] text-neutral-500" title={card.licenseNote}>
            License: {card.licenseNote}
          </p>
        )}

        <div className="mt-auto pt-1">{action}</div>
      </div>
    </article>
  );
});
