// After a pick that feeds the prompt (model, style, preset, reference picture, add-on) or a jump to
// another tab, the cursor goes to that tab's text field so typing can go on without the mouse.
import type { TabId } from "./state/model";

/** The text field each tab's main action reads. Edit shows one of these, by mode. */
const FIELDS: Partial<Record<TabId, string>> = {
  create: "#prompt",
  edit: "#edit-instruction, #edit-extend, #edit-fix, #edit-restyle",
};

function isTextField(el: Element | null): boolean {
  if (!el) return false;
  if (el.tagName === "INPUT") return !["range", "checkbox", "radio", "button", "file"].includes((el as HTMLInputElement).type);
  return el.tagName === "TEXTAREA" || el.tagName === "SELECT" || (el as HTMLElement).isContentEditable;
}

/**
 * Focus the tab's text field, caret at the end. Runs after the current render and after the focus
 * moves popovers and dialogs make when they close; does nothing while a dialog is open or while
 * another text field has focus (e.g. one clicked while a picture was loading).
 */
export function focusTabField(tab: TabId) {
  const fields = FIELDS[tab];
  if (!fields) return;
  requestAnimationFrame(() =>
    setTimeout(() => {
      if (document.querySelector('[role="dialog"]')) return;
      const el = document.querySelector<HTMLTextAreaElement | HTMLInputElement>(`#tab-${tab}:not([hidden]) :is(${fields})`);
      if (!el || el.disabled || (document.activeElement !== el && isTextField(document.activeElement))) return;
      el.focus();
      const end = el.value.length;
      el.setSelectionRange(end, end);
    }, 0),
  );
}
