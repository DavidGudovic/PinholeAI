import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { runShortcut, shortcutFor, useShortcuts } from "./shortcuts";

afterEach(cleanup);

const key = (key: string, extra: Partial<KeyboardEventInit> = {}) => new KeyboardEvent("keydown", { key, bubbles: true, ...extra });

function Probe({ onSave, onEdit, tab = "create" }: { onSave: () => void; onEdit?: () => void; tab?: "create" | "edit" }) {
  useShortcuts(tab, { save: onSave, edit: onEdit });
  return <textarea data-testid="t" />;
}

describe("shortcutFor", () => {
  it("maps keys and ignores unrelated or modified ones", () => {
    const e = (k: string, o: object = {}) => shortcutFor({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...o });
    expect(e("e")).toBe("edit");
    expect(e("S")).toBe("save");
    expect(e("s", { ctrlKey: true, shiftKey: true })).toBe("saveAs");
    expect(e("s", { metaKey: true })).toBe("save");
    expect(e("e", { ctrlKey: true })).toBeNull();
    expect(e("d", { altKey: true })).toBeNull();
    expect(e("x")).toBeNull();
  });
});

describe("runShortcut", () => {
  it("runs the active tab's handler, only for keys it provides", () => {
    const save = vi.fn();
    render(<Probe onSave={save} />);
    expect(runShortcut(key("s"), "create")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    expect(runShortcut(key("e"), "create")).toBe(false);
    expect(runShortcut(key("s"), "edit")).toBe(false);
  });

  it("does nothing for letters while typing, or while a dialog is open, or on key repeat", () => {
    const save = vi.fn();
    const { getByTestId } = render(<Probe onSave={save} />);
    const field = getByTestId("t");
    const typed = key("s");
    Object.defineProperty(typed, "target", { value: field });
    expect(runShortcut(typed, "create")).toBe(false);
    // Ctrl+S still works in a text box.
    const mod = key("s", { ctrlKey: true });
    Object.defineProperty(mod, "target", { value: field });
    expect(runShortcut(mod, "create")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    expect(runShortcut(key("s", { repeat: true }), "create")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    const dlg = document.createElement("div");
    dlg.setAttribute("role", "dialog");
    document.body.appendChild(dlg);
    expect(runShortcut(key("s"), "create")).toBe(false);
    dlg.remove();
  });

  it("the newest registration with a handler wins", () => {
    const a = vi.fn();
    const b = vi.fn();
    render(<Probe onSave={a} />);
    render(<Probe onSave={b} />);
    runShortcut(key("s"), "create");
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).not.toHaveBeenCalled();
  });
});
