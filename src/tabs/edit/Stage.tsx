// The picture being edited, with the mask canvas, Compare and Side by side views, Extend frame and full-screen viewer.
import { useRef, useState, type RefObject } from "react";
import { Maximize2 } from "lucide-react";
import { ImageViewer } from "../../components/ImageViewer";
import { SideBySide, type SidePicture } from "../../components/SideBySide";
import { IconButton, cx } from "../../components/ui";
import type { ExtendCanvas } from "../../lib/types";
import { useShortcuts } from "../../lib/shortcuts";
import { CompareView } from "./CompareView";
import { MaskCanvas, type MaskHandle } from "./MaskCanvas";
import { useFitBox } from "./useFitBox";

export function Stage({
  current,
  before,
  pair,
  beforeLabel,
  afterLabel,
  maskOn,
  maskRef,
  brush,
  erase,
  onPainted,
  canvas,
}: {
  current: { id: string; url: string; width: number; height: number };
  before?: { id: string; url: string; width: number; height: number };
  /** Two pictures shown whole next to each other, in place of the picture. */
  pair?: { first: SidePicture; second: SidePicture };
  beforeLabel: string;
  afterLabel: string;
  maskOn: boolean;
  maskRef: RefObject<MaskHandle | null>;
  brush: number;
  erase: boolean;
  onPainted: (b: boolean) => void;
  /** Extend's new canvas (source pixels): shown as a dashed frame around the picture. */
  canvas: ExtendCanvas | null;
}) {
  const container = useRef<HTMLDivElement>(null);
  const box = useFitBox(
    container,
    canvas?.width ?? current.width,
    canvas?.height ?? current.height,
  );
  // With a canvas, `box` is the canvas; the picture sits inside it.
  const pic = canvas
    ? {
        left: (canvas.left / canvas.width) * box.width,
        top: (canvas.top / canvas.height) * box.height,
        width: (current.width / canvas.width) * box.width,
        height: (current.height / canvas.height) * box.height,
      }
    : { left: 0, top: 0, width: box.width, height: box.height };
  const [viewing, setViewing] = useState(false);
  useShortcuts("edit", {
    fullscreen: maskOn ? undefined : () => setViewing(true),
  });
  return (
    <div
      ref={container}
      className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-6"
    >
      {pair && <SideBySide first={pair.first} second={pair.second} />}
      {box.width > 0 && before && !pair && (
        <CompareView
          before={before}
          after={current}
          width={box.width}
          height={box.height}
          beforeLabel={beforeLabel}
          afterLabel={afterLabel}
        />
      )}
      {canvas && !before && !pair && box.width > 0 && (
        <div
          aria-label="New space"
          className="absolute rounded-lg border-2 border-dashed border-amber-500/80 bg-amber-500/10"
          style={{ width: box.width, height: box.height }}
        />
      )}
      {/* Kept mounted while comparing so a painted mask isn't lost. */}
      <div
        hidden={!!before || !!pair || box.width === 0}
        className={cx(
          "relative overflow-hidden shadow-lg ring-1 ring-black/5 dark:ring-white/10",
          canvas ? "rounded-sm" : "rounded-lg",
        )}
        style={{
          width: pic.width,
          height: pic.height,
          transform: canvas
            ? `translate(${pic.left + pic.width / 2 - box.width / 2}px, ${pic.top + pic.height / 2 - box.height / 2}px)`
            : undefined,
        }}
      >
        <img
          src={current.url}
          alt={`Image being edited (${afterLabel})`}
          className="absolute inset-0 h-full w-full"
          draggable={false}
        />
        {!maskOn && (
          <IconButton
            label="View full screen"
            size="sm"
            className="absolute top-2 right-2 z-10 bg-black/40! text-white! hover:bg-black/60!"
            onClick={() => setViewing(true)}
          >
            <Maximize2 className="h-4 w-4" />
          </IconButton>
        )}
        <MaskCanvas
          ref={maskRef}
          width={current.width}
          height={current.height}
          displayWidth={pic.width}
          brush={brush}
          erase={erase}
          active={maskOn}
          onPaintedChange={onPainted}
        />
      </div>
      {viewing && (
        <ImageViewer
          images={[
            {
              url: current.url,
              width: current.width,
              height: current.height,
              alt: `Image being edited (${afterLabel})`,
            },
          ]}
          index={0}
          onIndex={() => {}}
          onClose={() => setViewing(false)}
        />
      )}
    </div>
  );
}
