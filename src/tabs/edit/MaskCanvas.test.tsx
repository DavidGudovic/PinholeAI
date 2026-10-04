// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MaskCanvas } from "./MaskCanvas";

// jsdom has no 2D canvas: a stand-in where a paint stroke fills every pixel and an
// eraser stroke clears them all.
let alpha = 0;
beforeEach(() => {
  alpha = 0;
  const ctx = {
    globalCompositeOperation: "source-over",
    beginPath() {},
    moveTo() {},
    lineTo() {},
    stroke() {
      alpha = this.globalCompositeOperation === "destination-out" ? 0 : 255;
    },
    clearRect() {
      alpha = 0;
    },
    getImageData(_x: number, _y: number, w: number, h: number) {
      const data = new Uint8ClampedArray(w * h * 4);
      for (let i = 3; i < data.length; i += 4) data[i] = alpha;
      return { data };
    },
  };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx as never);
  HTMLCanvasElement.prototype.setPointerCapture ??= () => undefined;
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const props = { width: 8, height: 8, displayWidth: 8, brush: 4 };

describe("MaskCanvas", () => {
  it("hides the paint while the brush is off and keeps it for when it is back on", () => {
    const onPainted = vi.fn();
    const { rerender } = render(<MaskCanvas {...props} erase={false} active onPaintedChange={onPainted} />);
    const canvas = screen.getByLabelText("Paint where the image may change");
    fireEvent.pointerDown(canvas, { button: 0 });
    fireEvent.pointerUp(canvas);
    expect(canvas.className).not.toContain("invisible");
    rerender(<MaskCanvas {...props} erase={false} active={false} onPaintedChange={onPainted} />);
    expect(canvas.className).toContain("invisible");
    expect(canvas.className).toContain("pointer-events-none");
    rerender(<MaskCanvas {...props} erase={false} active onPaintedChange={onPainted} />);
    expect(canvas.className).not.toContain("invisible");
    expect(alpha).toBe(255);
    expect(onPainted).toHaveBeenLastCalledWith(true);
  });

  it("counts as not painted once the eraser has removed all the paint", () => {
    const onPainted = vi.fn();
    const { rerender } = render(<MaskCanvas {...props} erase={false} active onPaintedChange={onPainted} />);
    const canvas = screen.getByLabelText("Paint where the image may change");
    fireEvent.pointerDown(canvas, { button: 0 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(true);
    rerender(<MaskCanvas {...props} erase active onPaintedChange={onPainted} />);
    fireEvent.pointerDown(canvas, { button: 0 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(false);
    // Painting again counts again.
    rerender(<MaskCanvas {...props} erase={false} active onPaintedChange={onPainted} />);
    fireEvent.pointerDown(canvas, { button: 0 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(true);
  });
});
