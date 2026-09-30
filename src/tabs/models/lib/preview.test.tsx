// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { useRef } from "react";
import { NEAR_MARGIN, ScrollRootContext, useVisibility } from "./preview";

type Recorded = { options: IntersectionObserverInit | undefined; targets: Element[] };
let observers: Recorded[] = [];

class FakeIO {
  rec: Recorded;
  constructor(_cb: IntersectionObserverCallback, options?: IntersectionObserverInit) {
    this.rec = { options, targets: [] };
    observers.push(this.rec);
  }
  observe(el: Element) {
    this.rec.targets.push(el);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}

beforeEach(() => {
  observers = [];
  vi.stubGlobal("IntersectionObserver", FakeIO);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function Card() {
  const ref = useRef<HTMLDivElement>(null);
  useVisibility(ref);
  return <div ref={ref} data-testid="card" />;
}

describe("useVisibility", () => {
  it("observes relative to the scroller it is given, with the near margin", () => {
    const scroller = document.createElement("div");
    const { getByTestId } = render(
      <ScrollRootContext.Provider value={scroller}>
        <Card />
      </ScrollRootContext.Provider>,
    );
    const card = getByTestId("card");
    const watching = observers.filter((o) => o.targets.includes(card));
    expect(watching).toHaveLength(2);
    // Both observers use the real scroller: with the viewport as root, the margin is ignored
    // inside a nested scroller, so "near" would mean the same as "visible".
    expect(watching.every((o) => o.options?.root === scroller)).toBe(true);
    expect(watching.some((o) => o.options?.rootMargin === NEAR_MARGIN)).toBe(true);
  });

  it("keeps using the viewport without a scroller", () => {
    const { getByTestId } = render(<Card />);
    const card = getByTestId("card");
    const watching = observers.filter((o) => o.targets.includes(card));
    expect(watching).toHaveLength(2);
    expect(watching.every((o) => !o.options?.root)).toBe(true);
  });
});
