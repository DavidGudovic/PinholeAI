// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/react";
import { useImagePaste } from "./ImageDrop";
import { clipboardImage } from "../lib/api";

vi.mock("../lib/mock", async (orig) => ({ ...(await orig<typeof import("../lib/mock")>()), isTauri: () => true }));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), clipboardImage: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.mocked(clipboardImage).mockReset();
});

function PasteProbe({ onFile }: { onFile: (f: File) => void }) {
  useImagePaste(true, onFile);
  return <textarea aria-label="text" />;
}

/** What the Linux WebView sends: a paste event whose clipboardData is empty. */
function emptyPaste(types: string[] = []) {
  const e = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(e, "clipboardData", { value: { types, files: [], items: [] } });
  return e;
}

describe("image paste with an empty clipboardData (Linux WebView)", () => {
  it("reads the clipboard picture through the app", async () => {
    const pic = new File([new Uint8Array(4)], "pasted.png", { type: "image/png" });
    vi.mocked(clipboardImage).mockResolvedValue(pic);
    const onFile = vi.fn();
    render(<PasteProbe onFile={onFile} />);
    document.body.dispatchEvent(emptyPaste());
    await waitFor(() => expect(onFile).toHaveBeenCalledWith(pic));
    expect(clipboardImage).toHaveBeenCalledWith(false);
  });

  it("in a text box, asks only for a picture without text", async () => {
    vi.mocked(clipboardImage).mockResolvedValue(null);
    const onFile = vi.fn();
    const { getByLabelText } = render(<PasteProbe onFile={onFile} />);
    const e = emptyPaste();
    getByLabelText("text").dispatchEvent(e);
    await waitFor(() => expect(clipboardImage).toHaveBeenCalledWith(true));
    await new Promise((r) => setTimeout(r, 0));
    expect(onFile).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it("does nothing when the clipboard holds no picture", async () => {
    vi.mocked(clipboardImage).mockResolvedValue(null);
    const onFile = vi.fn();
    render(<PasteProbe onFile={onFile} />);
    document.body.dispatchEvent(emptyPaste());
    await waitFor(() => expect(clipboardImage).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
    expect(onFile).not.toHaveBeenCalled();
  });

  it("reads the picture after a browser's Copy image, where the WebView only hands over the page markup", async () => {
    // Measured in WebKitGTK 2.x with image/png + text/html on the clipboard (as Chrome puts them).
    const pic = new File([new Uint8Array(4)], "pasted.png", { type: "image/png" });
    vi.mocked(clipboardImage).mockResolvedValue(pic);
    const onFile = vi.fn();
    const { getByLabelText } = render(<PasteProbe onFile={onFile} />);
    document.body.dispatchEvent(emptyPaste(["text/html"]));
    await waitFor(() => expect(onFile).toHaveBeenCalledWith(pic));
    expect(clipboardImage).toHaveBeenCalledWith(false);
    getByLabelText("text").dispatchEvent(emptyPaste(["text/html"]));
    await waitFor(() => expect(clipboardImage).toHaveBeenLastCalledWith(true));
  });

  it("leaves a text paste in a text box alone", async () => {
    const { getByLabelText } = render(<PasteProbe onFile={vi.fn()} />);
    getByLabelText("text").dispatchEvent(emptyPaste(["text/plain", "text/html"]));
    await new Promise((r) => setTimeout(r, 0));
    expect(clipboardImage).not.toHaveBeenCalled();
  });
});
