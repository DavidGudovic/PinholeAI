// Pictures made this session (Create and Edit results still in memory), offered wherever a
// picture is asked for: a row of thumbnails, and a picture button that offers them next to
// choosing a file. Picking one uses the picture already in the session; nothing is read from disk.
import { useMemo, type ReactNode } from "react";
import { FolderOpen } from "lucide-react";
import { thumbSrc } from "../lib/state/images";
import { sessionPictures, type ImgRef } from "../lib/state/model";
import { useAppState } from "../lib/state/store";
import { Button, MenuItem, MenuLabel, MenuSeparator, Popover, cx } from "./ui";

/** This session's pictures, newest first, without the ids in `exclude` (at most `max`). */
export function useSessionPictures(exclude: (string | null | undefined)[] = [], max = 8): ImgRef[] {
  const results = useAppState((s) => s.results);
  // Only the history: typing in Edit's text boxes doesn't re-render every picture picker.
  const chain = useAppState((s) => s.edit.chain);
  const images = useAppState((s) => s.images);
  const key = exclude.join("|");
  return useMemo(
    () => sessionPictures({ results, edit: { chain }, images }, key.split("|")).slice(0, max),
    [results, chain, images, key, max],
  );
}

/** A row of thumbnails; `title` is each one's tooltip and label. */
export function SessionStrip({
  pictures,
  onPick,
  title,
  size = "sm",
  disabled,
}: {
  pictures: ImgRef[];
  onPick: (p: ImgRef) => void;
  title: string;
  size?: "sm" | "md";
  disabled?: boolean;
}) {
  if (!pictures.length) return null;
  return (
    <div role="group" aria-label="Pictures from this session" className="flex flex-wrap gap-1">
      {pictures.map((p) => (
        <button
          key={p.id}
          type="button"
          title={title}
          aria-label={title}
          disabled={disabled}
          className={cx(
            "overflow-hidden rounded-md opacity-80 ring-amber-500 hover:opacity-100 hover:ring-2 focus-visible:ring-2 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-40",
            size === "sm" ? "h-7 w-7" : "h-12 w-12",
          )}
          onClick={() => onPick(p)}
        >
          <img src={thumbSrc(p)} alt="" className="h-full w-full object-cover" decoding="async" loading="lazy" draggable={false} />
        </button>
      ))}
    </div>
  );
}

/**
 * A button that asks for a picture. With pictures from this session it opens a small menu
 * (choose a file, or one of them); without, it opens the file chooser straight away.
 */
export function PictureButton({
  children,
  onFile,
  onPick,
  pictures,
  pickTitle,
  size = "sm",
  variant = "ghost",
  disabled,
  title,
  ariaLabel,
  align = "start",
  className,
}: {
  children: ReactNode;
  /** Opens the file chooser. */
  onFile: () => void;
  onPick: (p: ImgRef) => void;
  pictures: ImgRef[];
  pickTitle: string;
  size?: "sm" | "md";
  variant?: "ghost" | "primary" | "secondary";
  disabled?: boolean;
  title?: string;
  ariaLabel?: string;
  align?: "start" | "end";
  className?: string;
}) {
  if (!pictures.length) {
    return (
      <Button size={size} variant={variant} disabled={disabled} onClick={onFile} title={title} aria-label={ariaLabel} className={className}>
        {children}
      </Button>
    );
  }
  return (
    <Popover
      align={align}
      width={248}
      trigger={(p) => (
        <Button {...p} size={size} variant={variant} disabled={disabled} title={title} aria-label={ariaLabel} className={className}>
          {children}
        </Button>
      )}
    >
      {(close) => (
        <>
          <MenuItem
            icon={<FolderOpen className="h-4 w-4" />}
            onClick={() => {
              close();
              onFile();
            }}
          >
            Choose a file…
          </MenuItem>
          <MenuSeparator />
          <MenuLabel>From this session</MenuLabel>
          <div className="px-2 pb-2">
            <SessionStrip
              pictures={pictures}
              size="md"
              title={pickTitle}
              onPick={(pic) => {
                close();
                onPick(pic);
              }}
            />
          </div>
        </>
      )}
    </Popover>
  );
}

/** Under an empty drop area: "Or use one from this session" and the thumbnails. */
export function SessionChoices({ pictures, onPick, title, disabled }: { pictures: ImgRef[]; onPick: (p: ImgRef) => void; title: string; disabled?: boolean }) {
  if (!pictures.length) return null;
  return (
    <div className="mt-5 flex flex-col items-center gap-2">
      <p className="text-xs text-neutral-500">Or use one from this session</p>
      <SessionStrip pictures={pictures} onPick={onPick} title={title} size="md" disabled={disabled} />
    </div>
  );
}
