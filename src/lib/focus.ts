// After a pick that feeds the prompt (model, style, preset, reference picture, add-on) or a jump to
// another tab, the cursor goes to that tab's text field so typing can go on without the mouse.
import type { TabId } from "./state/model";

/** The text field each tab's main action reads. Edit shows one of these, by mode. */
const FIELDS: Partial<Record<TabId, string>> = {
  create: "#prompt",
  edit: "#edit-instruction, #edit-extend, #edit-fix, #edit-restyle",
};

/**
 * Focus the tab's text field, caret at the end. Runs after the current render and after the focus
 * moves popovers and dialogs make when they close; does nothing while a dialog is open.
 */
export function focusTabField(tab: TabId) {
  const fields = FIELDS[tab];
  if (!fields) return;
  requestAnimationFrame(() =>
    setTimeout(() => {
      if (document.querySelector('[role="dialog"]')) return;
      const el = document.querySelector<HTMLTextAreaElement | HTMLInputElement>(`#tab-${tab}:not([hidden]) :is(${fields})`);
      if (!el || el.disabled) return;
      el.focus();
      const end = el.value.length;
      el.setSelectionRange(end, end);
    }, 0),
  );
}
