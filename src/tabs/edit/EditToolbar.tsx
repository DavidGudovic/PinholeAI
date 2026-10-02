// The bar above the picture: undo, redo, delete, Try again, Compare, Side by side, and the image actions.
import {
  Columns2,
  Copy,
  ImagePlus,
  PanelsLeftRight,
  Redo2,
  RefreshCw,
  ScanText,
  Trash2,
  Undo2,
} from "lucide-react";
import { SaveButton, UpscaleMenu } from "../../components/ImageActions";
import { Button, IconButton, Segmented } from "../../components/ui";
import type {
  Action,
  EditNode,
  EditParams,
  ImgRef,
} from "../../lib/state/model";

export function EditToolbar({
  e,
  current,
  node,
  before,
  locked,
  job,
  importing,
  canTryAgain,
  onTryAgain,
  compare,
  setCompare,
  canSideBySide,
  sideBySide,
  setSideBySide,
  compareWith,
  setCompareWith,
  onPickImage,
  onUpscale,
  runAction,
  onCopy,
  onDescribe,
  dispatch,
}: {
  e: EditParams;
  current: ImgRef;
  node: EditNode | null;
  before: ImgRef | undefined;
  locked: boolean;
  job: boolean;
  importing: boolean;
  canTryAgain: boolean;
  onTryAgain: () => void;
  compare: boolean;
  setCompare: (on: boolean) => void;
  /** The shown edit combined a second image. */
  canSideBySide: boolean;
  sideBySide: boolean;
  setSideBySide: (on: boolean) => void;
  compareWith: "previous" | "original";
  setCompareWith: (w: "previous" | "original") => void;
  onPickImage: () => void;
  onUpscale: (factor: 2 | 4) => void;
  runAction: (f: () => Promise<unknown>) => Promise<void>;
  onCopy: () => void;
  onDescribe: () => void;
  dispatch: (a: Action) => void;
}) {
  return (
    <div className="flex shrink-0 items-center gap-1.5 border-b border-neutral-200 bg-white/60 px-4 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
      <IconButton
        label="Undo"
        disabled={e.index === 0 || locked}
        onClick={() =>
          dispatch({ type: "editGoto", index: e.index - 1 })
        }
      >
        <Undo2 className="h-4 w-4" />
      </IconButton>
      <IconButton
        label="Redo"
        disabled={e.index >= e.chain.length - 1 || locked}
        onClick={() =>
          dispatch({ type: "editGoto", index: e.index + 1 })
        }
      >
        <Redo2 className="h-4 w-4" />
      </IconButton>
      <IconButton
        label="Delete this edit"
        disabled={e.index === 0 || job || locked}
        onClick={() => dispatch({ type: "editDelete", index: e.index })}
      >
        <Trash2 className="h-4 w-4" />
      </IconButton>
      <span className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />
      <Button
        size="sm"
        variant="ghost"
        disabled={!canTryAgain}
        onClick={onTryAgain}
        title={
          e.index < e.chain.length - 1
            ? "Make this edit again from the step before, with a new seed. Replaces it and the edits after it."
            : "Make this edit again from the step before, with a new seed. Replaces it."
        }
        aria-label="Try again"
      >
        <RefreshCw className="h-3.5 w-3.5" />{" "}
        <span className="hidden xl:inline">Try again</span>
      </Button>
      <Button
        size="sm"
        variant={compare && before ? "secondary" : "ghost"}
        disabled={!before && e.index === 0}
        aria-pressed={compare && !!before}
        onClick={() => setCompare(!compare)}
      >
        <Columns2 className="h-3.5 w-3.5" /> Compare
      </Button>
      {compare && e.index > 1 && (
        <Segmented
          size="sm"
          ariaLabel="Compare with"
          value={compareWith}
          onChange={setCompareWith}
          options={[
            { value: "previous", label: "Previous" },
            { value: "original", label: "Original" },
          ]}
        />
      )}
      {canSideBySide && (
        <Button
          size="sm"
          variant={sideBySide ? "secondary" : "ghost"}
          aria-pressed={sideBySide}
          title="Show image 2 and this edit next to each other"
          onClick={() => setSideBySide(!sideBySide)}
        >
          <PanelsLeftRight className="h-3.5 w-3.5" /> Side by side
        </Button>
      )}
      <div className="ml-auto flex items-center gap-1.5">
        <Button
          size="sm"
          variant="ghost"
          disabled={locked}
          onClick={onPickImage}
          title="Pick another image to edit"
          aria-label="Another image"
        >
          <ImagePlus className="h-3.5 w-3.5" />{" "}
          <span className="hidden xl:inline">Another image</span>
        </Button>
        <UpscaleMenu
          size="sm"
          width={current.width}
          height={current.height}
          disabled={job || importing}
          onPick={onUpscale}
        />
        <SaveButton
          tab="edit"
          size="sm"
          id={current.id}
          seed={node?.meta?.seed ?? null}
          run={runAction}
        />
        <IconButton
          label="Copy image"
          size="sm"
          variant="secondary"
          onClick={onCopy}
        >
          <Copy className="h-3.5 w-3.5" />
        </IconButton>
        <IconButton
          label="Describe this image"
          size="sm"
          variant="secondary"
          onClick={onDescribe}
        >
          <ScanText className="h-3.5 w-3.5" />
        </IconButton>
      </div>
    </div>
  );
}
