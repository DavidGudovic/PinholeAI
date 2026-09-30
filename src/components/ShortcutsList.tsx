// The keyboard shortcuts list: a section in Settings and the window the ? key opens.
import { Fragment } from "react";
import { SHORTCUT_LIST } from "../lib/shortcuts";
import { modKey } from "../lib/state/platform";
import { Kbd } from "./ui";

export function ShortcutsList() {
  return (
    <dl className="space-y-2 text-sm">
      {SHORTCUT_LIST.map((s) => (
        <div key={s.what} className="flex items-center justify-between gap-4">
          <dt className="text-neutral-700 dark:text-neutral-300">{s.what}</dt>
          <dd className="flex shrink-0 items-center gap-1">
            {s.keys.map((k, i) => (
              <Fragment key={k}>
                {i > 0 && <span className="text-xs text-neutral-400">+</span>}
                <Kbd>{k === "Mod" ? modKey : k}</Kbd>
              </Fragment>
            ))}
          </dd>
        </div>
      ))}
      <p className="pt-1 text-xs text-neutral-500">Letter keys only work when you are not typing in a text box.</p>
    </dl>
  );
}
