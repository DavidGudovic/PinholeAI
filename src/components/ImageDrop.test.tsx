// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { blockStrayDrops } from "../lib/platform";
import { DropTarget } from "./ImageDrop";

afterEach(cleanup);

/** jsdom has no DragEvent/DataTransfer: a plain cancelable event carrying a fake transfer. */
function drag(type: "dragover" | "drop", target: Element, data: { types: string[]; files?: File[] }) {
  const e = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, "dataTransfer", { value: { types: data.types, files: data.files ?? [], items: [], dropEffect: "copy" } });
  target.dispatchEvent(e);
  return e as Event & { dataTransfer: { dropEffect: string } };
}

describe("drops outside a drop area", () => {
  it("never let the window open a dropped link or file, while drop areas and text fields still work", () => {
    const stop = blockStrayDrops();
    const onFile = vi.fn();
    render(
      <div>
        <p>Models</p>
        <textarea aria-label="Prompt" />
        <DropTarget onFile={onFile}>
          <span>Drop here</span>
        </DropTarget>
      </div>,
    );
    const png = new File(["x"], "a.png", { type: "image/png" });

    // A link or a file dropped on a part of the window with no drop area: refused.
    expect(drag("drop", screen.getByText("Models"), { types: ["text/uri-list"] }).defaultPrevented).toBe(true);
    expect(drag("drop", screen.getByText("Models"), { types: ["Files"], files: [png] }).defaultPrevented).toBe(true);
    const over = drag("dragover", screen.getByText("Models"), { types: ["text/uri-list"] });
    expect(over.defaultPrevented).toBe(true);
    expect(over.dataTransfer.dropEffect).toBe("none");
    // A link dropped on a drop area (which only takes files): refused too.
    expect(drag("drop", screen.getByText("Drop here"), { types: ["text/uri-list"] }).defaultPrevented).toBe(true);

    // The drop area still gets its picture, and keeps its "copy" cursor.
    expect(drag("dragover", screen.getByText("Drop here"), { types: ["Files"] }).dataTransfer.dropEffect).toBe("copy");
    drag("drop", screen.getByText("Drop here"), { types: ["Files"], files: [png] });
    expect(onFile).toHaveBeenCalledWith(png);

    // Text dragged into a text field drops as usual.
    expect(drag("drop", screen.getByLabelText("Prompt"), { types: ["text/plain"] }).defaultPrevented).toBe(false);

    stop();
    expect(drag("drop", screen.getByText("Models"), { types: ["text/uri-list"] }).defaultPrevented).toBe(false);
  });
});

describe("picture in a data: address", () => {
  it("reads PNG, JPEG and WebP only, and keeps the bytes", async () => {
    const { pictureFromDataUrl } = await import("./ImageDrop");
    const f = pictureFromDataUrl("data:image/png;base64,iVBORw0KGgo=");
    expect(f?.type).toBe("image/png");
    expect(Array.from(new Uint8Array(await f!.arrayBuffer())).slice(0, 4)).toEqual([0x89, 0x50, 0x4e, 0x47]);
    expect(pictureFromDataUrl("data:image/jpeg;base64,/9j/")?.name).toBe("dropped.jpg");
    expect(pictureFromDataUrl("data:image/svg+xml;base64,PHN2Zz4=")).toBeNull();
    expect(pictureFromDataUrl("data:text/html;base64,PGI+")).toBeNull();
    expect(pictureFromDataUrl("https://example.com/a.png")).toBeNull();
    expect(pictureFromDataUrl("data:image/png;base64,***")).toBeNull();
  });
});
