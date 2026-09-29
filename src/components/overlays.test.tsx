// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { Dialog, Sheet } from "./ui";
import { useImagePaste } from "./ImageDrop";

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
