// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Button, Dialog, MenuItem, Popover, Sheet } from "./ui";
import { useImagePaste } from "./ImageDrop";
import { Toasts } from "./Toasts";
import { StoreContext, createStore } from "../lib/state/store";

afterEach(cleanup);

describe("Escape", () => {
  it("closes only the overlay opened last", () => {
    const closeSheet = vi.fn();
    const closeDialog = vi.fn();
    const { rerender } = render(
      <Sheet open onClose={closeSheet} title="Settings">
        <Dialog open={false} onClose={closeDialog} title="Key">
          x
        </Dialog>
      </Sheet>,
    );
    rerender(
      <Sheet open onClose={closeSheet} title="Settings">
        <Dialog open onClose={closeDialog} title="Key">
          x
        </Dialog>
      </Sheet>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(closeDialog).toHaveBeenCalledTimes(1);
    expect(closeSheet).not.toHaveBeenCalled();
    rerender(
      <Sheet open onClose={closeSheet} title="Settings">
        <Dialog open={false} onClose={closeDialog} title="Key">
          x
        </Dialog>
      </Sheet>,
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(closeSheet).toHaveBeenCalledTimes(1);
  });
});

function PasteProbe({ onFile }: { onFile: (f: File) => void }) {
  useImagePaste(true, onFile);
  return <textarea aria-label="text" />;
}

function pasteEvent(types: string[]) {
  const file = new File([new Uint8Array(4)], "clip.png", { type: "image/png" });
  const e = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(e, "clipboardData", { value: { types, files: [file], items: [] } });
  return e;
}

describe("image paste", () => {
  it("leaves text pasted into a text field alone, even when the clipboard also has a picture", () => {
    const onFile = vi.fn();
    const { getByLabelText } = render(<PasteProbe onFile={onFile} />);
    const e = pasteEvent(["text/plain", "Files"]);
    getByLabelText("text").dispatchEvent(e);
    expect(onFile).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it("takes a file copied in a file manager, even in a text field", () => {
    const onFile = vi.fn();
    const { getByLabelText } = render(<PasteProbe onFile={onFile} />);
    getByLabelText("text").dispatchEvent(pasteEvent(["text/plain", "text/uri-list", "Files"]));
    expect(onFile).toHaveBeenCalledTimes(1);
  });

  it("takes a picture pasted anywhere else", () => {
    const onFile = vi.fn();
    render(<PasteProbe onFile={onFile} />);
    document.body.dispatchEvent(pasteEvent(["Files"]));
    expect(onFile).toHaveBeenCalledTimes(1);
  });
});

describe("keyboard focus", () => {
  const tick = () => act(() => new Promise((r) => setTimeout(r, 0)));

  function Menu({ onDialog }: { onDialog?: () => void }) {
    return (
      <Popover trigger={(p) => <button {...p}>Open menu</button>}>
        {(close) => (
          <>
            <MenuItem onClick={close}>First</MenuItem>
            <MenuItem
              onClick={() => {
                close();
                onDialog?.();
              }}
            >
              Open dialog
            </MenuItem>
          </>
        )}
      </Popover>
    );
  }

  it("moves into an opened menu and back to its button on Escape", async () => {
    render(<Menu />);
    const trigger = screen.getByRole("button", { name: "Open menu" });
    trigger.focus();
    fireEvent.click(trigger);
    await tick();
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "First" }));
    fireEvent.keyDown(window, { key: "Escape" });
    await tick();
    expect(screen.queryByRole("menuitem")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("returns to the menu button after a dialog opened from the menu closes", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <Menu onDialog={() => setOpen(true)} />
          <Dialog open={open} onClose={() => setOpen(false)} title="Save as preset">
            <input aria-label="Name" />
          </Dialog>
        </>
      );
    }
    render(<Harness />);
    const trigger = screen.getByRole("button", { name: "Open menu" });
    trigger.focus();
    fireEvent.click(trigger);
    await tick();
    fireEvent.click(screen.getByRole("menuitem", { name: "Open dialog" }));
    await tick();
    expect(document.activeElement).toBe(screen.getByLabelText("Name"));
    fireEvent.keyDown(window, { key: "Escape" });
    await tick();
    expect(document.activeElement).toBe(trigger);
  });

  it("the Settings sheet takes focus, is named by its title and gives focus back", async () => {
    const view = (open: boolean) => (
      <>
        <button>Gear</button>
        <Sheet open={open} onClose={() => undefined} title="Settings">
          <input aria-label="Field" />
        </Sheet>
      </>
    );
    const { rerender } = render(view(false));
    const gear = screen.getByRole("button", { name: "Gear" });
    gear.focus();
    rerender(view(true));
    await tick();
    expect(document.activeElement).toBe(screen.getByRole("dialog", { name: "Settings" }));
    rerender(view(false));
    await tick();
    expect(document.activeElement).toBe(gear);
  });
});

describe("popover position", () => {
  it("follows its button when a sidebar scrolls", () => {
    render(
      <div data-testid="aside" style={{ overflowY: "auto" }}>
        <Popover trigger={(p) => <button {...p}>Style</button>}>
          <MenuItem>Watercolor</MenuItem>
        </Popover>
      </div>,
    );
    const trigger = screen.getByRole("button", { name: "Style" });
    let top = 100;
    trigger.getBoundingClientRect = () => ({ top, bottom: top + 30, left: 10, right: 110, width: 100, height: 30, x: 10, y: top, toJSON: () => ({}) });
    fireEvent.click(trigger);
    const panel = screen.getByRole("menuitem", { name: "Watercolor" }).parentElement!;
    expect(panel.style.top).toBe("136px");
    top = 40;
    fireEvent.scroll(screen.getByTestId("aside"));
    expect(panel.style.top).toBe("76px");
  });
});

describe("looks", () => {
  it("a disabled primary button dims like the others (no half-transparent amber in dark mode)", () => {
    render(
      <Button variant="primary" disabled>
        Generate
      </Button>,
    );
    expect(screen.getByRole("button", { name: "Generate" }).className).not.toMatch(/disabled:bg-/);
  });

  it("toast text wraps at spaces, not mid-word", () => {
    const store = createStore();
    store.dispatch({ type: "toast", toast: { id: 1, text: "The edit finished after the image changed, so it wasn't added." } });
    render(
      <StoreContext.Provider value={store}>
        <Toasts />
      </StoreContext.Provider>,
    );
    const text = screen.getByText(/The edit finished/);
    expect(text.className).not.toMatch(/break-all/);
    expect(text.className).toMatch(/overflow-wrap:anywhere/);
  });
});
