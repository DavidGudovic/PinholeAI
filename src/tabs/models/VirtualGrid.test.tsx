// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { VirtualGrid } from "./VirtualGrid";

// jsdom has no layout: give every element a width, and rows a fixed height.
const ROW = 500;
const resizeCallbacks: ResizeObserverCallback[] = [];
class FakeRO {
  constructor(cb: ResizeObserverCallback) {
    resizeCallbacks.push(cb);
  }
  observe() {}
  unobserve() {}
  disconnect() {}
}

beforeAll(() => {
  vi.stubGlobal("ResizeObserver", FakeRO);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1552); // 6 columns
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(1000);
});
afterAll(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
afterEach(cleanup);

function scroller(scrollTop: number) {
  const el = document.createElement("div");
  // The list starts at the top of the scroller; scrolling moves it up.
  el.getBoundingClientRect = () => ({ top: 0, bottom: 1000, height: 1000, left: 0, right: 1552, width: 1552, x: 0, y: 0, toJSON: () => ({}) });
  return Object.assign(el, { scrollTop });
}

const items = Array.from({ length: 480 }, (_, i) => i);

function show(root: HTMLElement) {
  return render(
    <VirtualGrid items={items} itemKey={(i) => i} scrollRoot={root} renderItem={(i) => <article style={{ height: ROW }}>card {i}</article>} />,
  );
}

describe("VirtualGrid", () => {
  it("puts only the rows near the screen in the page", () => {
    const root = scroller(0);
    const { container } = show(root);
    const list = container.firstElementChild as HTMLElement;
    list.getBoundingClientRect = () => ({ top: 0, bottom: 0, height: 0, left: 0, right: 0, width: 0, x: 0, y: 0, toJSON: () => ({}) });
    act(() => root.dispatchEvent(new Event("scroll")));
    const shown = screen.getAllByRole("article").length;
    // 480 cards in 80 rows; the screen plus the margin around it holds a handful of rows.
    expect(shown).toBeGreaterThanOrEqual(6);
    expect(shown).toBeLessThanOrEqual(60);
    expect(screen.getByText("card 0")).toBeTruthy();
    expect(screen.queryByText("card 479")).toBeNull();
  });

  it("keeps the full height, so the scrollbar and paging work as before", () => {
    const { container } = show(scroller(0));
    const list = container.firstElementChild as HTMLElement;
    const pad = parseFloat(list.style.paddingTop || "0") + parseFloat(list.style.paddingBottom || "0");
    // The rows not shown are stood in for by padding.
    expect(pad).toBeGreaterThan(70 * 300);
  });

  it("shows the rows further down after scrolling there", async () => {
    const root = scroller(0);
    const { container } = show(root);
    const list = container.firstElementChild as HTMLElement;
    // Scrolled 20 estimated rows down: the list's top is far above the scroller's top.
    const top = -(20 * (((1552 - 5 * 16) / 6) * 1.25 + 190 + 16));
    list.getBoundingClientRect = () => ({ top, bottom: 0, height: 0, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({}) });
    await act(async () => {
      root.dispatchEvent(new Event("scroll"));
      await new Promise((r) => requestAnimationFrame(() => r(null)));
    });
    expect(screen.queryByText("card 0")).toBeNull();
    expect(screen.getByText("card 120")).toBeTruthy();
  });

  it("keeps its rows while the tab is hidden (width 0)", () => {
    resizeCallbacks.length = 0;
    const { container } = show(scroller(0));
    expect(container.querySelectorAll("[data-row]").length).toBeGreaterThan(0);
    const width = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(0);
    act(() => resizeCallbacks.forEach((cb) => cb([], {} as ResizeObserver)));
    expect(container.querySelectorAll("[data-row]").length).toBeGreaterThan(0);
    expect(screen.getAllByRole("article").length).toBeLessThanOrEqual(60);
    width.mockReturnValue(1552);
  });
});
