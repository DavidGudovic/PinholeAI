// Describe a change: "Add another image" (a file or a picture from this session), or the second
// image with its Remove button.
import type { ReactNode } from "react";
import { ImagePlus, Trash } from "lucide-react";
import { PictureButton } from "../../components/SessionPictures";
import { IconButton } from "../../components/ui";
import type { Action, ImgRef } from "../../lib/state/model";

export function SecondImage({
  second,
  pickerInput,
  onPick,
  sessionPictures,
  onPickSession,
  myJob,
  locked,
  importing,
  dispatch,
}: {
  second: ImgRef | undefined;
  pickerInput: ReactNode;
  onPick: () => void;
  /** This session's pictures (not image 1), offered next to choosing a file. */
  sessionPictures: ImgRef[];
  onPickSession: (p: ImgRef) => void;
  myJob: boolean;
  locked: boolean;
  importing: boolean;
  dispatch: (a: Action) => void;
}) {
  return (
    <div>
      {pickerInput}
      {second ? (
        <div className="flex items-center gap-3 rounded-xl border border-neutral-200 p-2 dark:border-neutral-800">
          <img
            src={second.url}
            alt="Image 2"
            className="h-12 w-12 shrink-0 rounded-md object-cover"
            draggable={false}
          />
          <p className="min-w-0 flex-1 text-xs text-neutral-500">
            <span className="font-medium text-neutral-700 dark:text-neutral-300">
              Image 2.
            </span>{" "}
            Call the picture you're editing “image 1” and this one
            “image 2”.
          </p>
          <IconButton
            label="Remove image 2"
            size="sm"
            variant="ghost"
            disabled={myJob}
            onClick={() =>
              dispatch({ type: "editSetSecond", ref: null })
            }
          >
            <Trash className="h-3.5 w-3.5" />
          </IconButton>
        </div>
      ) : (
        <PictureButton
          disabled={locked || importing}
          onFile={onPick}
          onPick={onPickSession}
          pictures={sessionPictures}
          pickTitle="Use as image 2"
          title="Use something from another picture, like an object or a logo"
        >
          <ImagePlus className="h-3.5 w-3.5" /> Add another image
        </PictureButton>
      )}
    </div>
  );
}
