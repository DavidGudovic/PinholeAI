// "Also apply to…": the same edit on more pictures (this session's results or files). Each one is
// edited on its own after the shown picture, and the results go to Create's results together.
import { useRef, type ReactNode } from "react";
import { Check, FolderOpen, Images, Trash } from "lucide-react";
import { Button, IconButton, MenuItem, MenuLabel, MenuSeparator, Popover, cx } from "../../components/ui";
import { ALSO_MAX, type Action, type ImgRef } from "../../lib/state/model";

/** Thumbnails shown in the picked row before "+N". */
const ROW_THUMBS = 6;

export function AlsoApply({
  picked,
  choices,
  disabled,
  onFiles,
  dispatch,
}: {
  /** The pictures picked so far, in order. */
  picked: ImgRef[];
  /** Pictures from this session that can be picked (not the one being edited or image 2). */
  choices: ImgRef[];
  disabled: boolean;
  onFiles: (files: File[]) => void;
  dispatch: (a: Action) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const pickedIds = new Set(picked.map((p) => p.id));
  // Files picked earlier aren't among the session's results: they stay in the grid as picked.
  const grid = [...choices, ...picked.filter((p) => !choices.some((c) => c.id === p.id))];
  const full = picked.length >= ALSO_MAX;
  const toggle = (p: ImgRef) =>
    dispatch({
      type: "editSetAlso",
      refs: pickedIds.has(p.id) ? picked.filter((r) => r.id !== p.id) : [...picked, p],
    });

  const fileInput = (
    <input
      ref={input}
      type="file"
      multiple
      accept="image/png,image/jpeg,image/webp,image/*"
      className="hidden"
      tabIndex={-1}
      onChange={(ev) => {
        const files = Array.from(ev.target.files ?? []);
        ev.target.value = "";
        if (files.length) onFiles(files);
      }}
    />
  );

  const menu = (label: ReactNode, title: string) => (
    <Popover
      width={312}
      trigger={(p) => (
        <Button {...p} size="sm" variant="ghost" disabled={disabled} title={title}>
          {label}
        </Button>
      )}
    >
      {(close) => (
        <>
          <MenuItem
            icon={<FolderOpen className="h-4 w-4" />}
            disabled={full}
            onClick={() => {
              close();
              input.current?.click();
            }}
          >
            Choose files…
          </MenuItem>
          {grid.length > 0 && (
            <>
              <MenuSeparator />
              <MenuLabel>From this session</MenuLabel>
              <div role="group" aria-label="Pictures from this session" className="grid grid-cols-5 gap-1.5 px-2 pb-2">
                {grid.map((p) => {
                  const on = pickedIds.has(p.id);
                  return (
                    <button
                      key={p.id}
                      type="button"
                      aria-pressed={on}
                      aria-label={on ? "Don't apply to this picture" : "Also apply to this picture"}
                      disabled={!on && full}
                      onClick={() => toggle(p)}
                      className={cx(
                        "relative aspect-square overflow-hidden rounded-md ring-amber-500 focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed disabled:opacity-40",
                        on ? "ring-2" : "opacity-80 hover:opacity-100",
                      )}
                    >
                      <img src={p.url} alt="" className="h-full w-full object-cover" draggable={false} />
                      {on && (
                        <span className="absolute top-0.5 right-0.5 rounded-full bg-amber-500 p-0.5 text-white">
                          <Check className="h-2.5 w-2.5" />
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </>
          )}
          <p className="px-2.5 pb-2 text-xs text-neutral-500">
            Up to {ALSO_MAX}. Each picture gets the same change after this one, and the results go to Create.
          </p>
        </>
      )}
    </Popover>
  );

  if (!picked.length) {
    return (
      <div>
        {fileInput}
        {menu(
          <>
            <Images className="h-3.5 w-3.5" /> Also apply to…
          </>,
          "Make the same change to more pictures",
        )}
      </div>
    );
  }
  const more = picked.length - ROW_THUMBS;
  return (
    <div className="rounded-xl border border-neutral-200 p-2 dark:border-neutral-800">
      {fileInput}
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-xs text-neutral-500">
          <span className="font-medium text-neutral-700 dark:text-neutral-300">
            Also on {picked.length} more {picked.length === 1 ? "picture" : "pictures"}.
          </span>{" "}
          The results go to Create.
        </p>
        {menu("Change…", "Pick other pictures")}
        <IconButton
          label="Don't apply to other pictures"
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={() => dispatch({ type: "editSetAlso", refs: [] })}
        >
          <Trash className="h-3.5 w-3.5" />
        </IconButton>
      </div>
      <div className="mt-1.5 flex gap-1">
        {picked.slice(0, ROW_THUMBS).map((p) => (
          <img key={p.id} src={p.url} alt="" className="h-9 w-9 rounded-md object-cover" draggable={false} />
        ))}
        {more > 0 && (
          <span className="flex h-9 w-9 items-center justify-center rounded-md bg-neutral-100 text-xs text-neutral-500 tabular-nums dark:bg-neutral-800">
            +{more}
          </span>
        )}
      </div>
    </div>
  );
}
