// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MaskCanvas } from "./MaskCanvas";

// jsdom has no 2D canvas: a stand-in where a paint stroke fills every pixel and an
// eraser stroke clears them all.
let alpha = 0;
// The canvas's size on screen (CSS px), for turning pointer positions into image pixels.
let shown = 8;
beforeEach(() => {
  alpha = 0;
  shown = 8;
  vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockImplementation(
    () => ({ left: 0, top: 0, width: shown, height: shown, right: shown, bottom: shown, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect,
  );
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

  it("after an eraser stroke reads back only the area that was painted, not the whole mask", () => {
    shown = 400;
    const big = { width: 4096, height: 4096, displayWidth: 400, brush: 4 };
    const read = vi.spyOn(HTMLCanvasElement.prototype.getContext("2d") as unknown as { getImageData: () => unknown }, "getImageData");
    const onPainted = vi.fn();
    const { rerender } = render(<MaskCanvas {...big} erase={false} active onPaintedChange={onPainted} />);
    const canvas = screen.getByLabelText("Paint where the image may change");
    fireEvent.pointerDown(canvas, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(true);
    rerender(<MaskCanvas {...big} erase active onPaintedChange={onPainted} />);
    fireEvent.pointerDown(canvas, { button: 0, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(false);
    expect(read).toHaveBeenCalled();
    for (const [x, y, w, h] of read.mock.calls as unknown as number[][]) {
      // The dab: 4 CSS px at 4096/400 image px each, around (102, 102).
      expect(x).toBeGreaterThan(50);
      expect(y).toBeGreaterThan(50);
      expect(w).toBeLessThan(64);
      expect(h).toBeLessThan(64);
    }
  });

  it("reads back a mask painted all over in blocks of at most 256 px a side", () => {
    shown = 400;
    const big = { width: 4096, height: 4096, displayWidth: 400, brush: 40 };
    const read = vi.spyOn(HTMLCanvasElement.prototype.getContext("2d") as unknown as { getImageData: () => unknown }, "getImageData");
    const onPainted = vi.fn();
    const { rerender } = render(<MaskCanvas {...big} erase={false} active onPaintedChange={onPainted} />);
    const canvas = screen.getByLabelText("Paint where the image may change");
    fireEvent.pointerDown(canvas, { button: 0, clientX: 0, clientY: 0 });
    fireEvent.pointerMove(canvas, { clientX: 400, clientY: 400 });
    fireEvent.pointerMove(canvas, { clientX: 0, clientY: 400 });
    fireEvent.pointerMove(canvas, { clientX: 400, clientY: 0 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(true);
    rerender(<MaskCanvas {...big} erase active onPaintedChange={onPainted} />);
    fireEvent.pointerDown(canvas, { button: 0, clientX: 200, clientY: 200 });
    fireEvent.pointerUp(canvas);
    expect(onPainted).toHaveBeenLastCalledWith(false);
    // Nothing left: every block of the 4096 px mask was read, none larger than 256 px.
    expect(read).toHaveBeenCalledTimes(16 * 16);
    for (const [, , w, h] of read.mock.calls as unknown as number[][]) {
      expect(w).toBeLessThanOrEqual(256);
      expect(h).toBeLessThanOrEqual(256);
    }
  });
});
