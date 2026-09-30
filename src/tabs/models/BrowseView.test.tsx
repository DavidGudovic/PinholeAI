// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { installMocks } from "../../lib/mock";
import { createStore, StoreContext } from "../../lib/state/store";
import { ScrollRootContext } from "./lib/preview";
import { BrowseView } from "./BrowseView";

const observers: { options: IntersectionObserverInit | undefined; targets: Element[] }[] = [];
class FakeIO {
  rec: (typeof observers)[number];
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

beforeAll(async () => {
  vi.stubGlobal("IntersectionObserver", FakeIO);
  await installMocks();
});
afterAll(() => vi.unstubAllGlobals());
afterEach(cleanup);

describe("BrowseView infinite scroll", () => {
  it("looks ahead relative to the Models tab's own scroller", async () => {
    const scroller = document.createElement("div");
    render(
      <StoreContext.Provider value={createStore()}>
        <ScrollRootContext.Provider value={scroller}>
          <BrowseView settings={null} onShowInstalled={() => undefined} />
        </ScrollRootContext.Provider>
      </StoreContext.Provider>,
    );
    const more = await screen.findByRole("button", { name: "Load more" }, { timeout: 5000 });
    const sentinel = more.parentElement!;
    await waitFor(() => expect(observers.some((o) => o.targets.includes(sentinel))).toBe(true));
    const io = observers.filter((o) => o.targets.includes(sentinel)).at(-1)!;
    // With the viewport as root, the look-ahead margin does nothing inside a nested scroller.
    expect(io.options?.root).toBe(scroller);
    expect(io.options?.rootMargin).toBe("1600px 0px");
  }, 10_000);
});
