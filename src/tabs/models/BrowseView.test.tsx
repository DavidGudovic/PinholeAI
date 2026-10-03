// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { installMocks } from "../../lib/mock";
import { createStore, StoreContext } from "../../lib/state/store";
import { ScrollRootContext } from "./lib/preview";
import { BrowseView } from "./BrowseView";
import * as api from "../../lib/api";
import { defaultFilters } from "./lib/query";
import { getLastFilters, rememberFilters } from "./lib/session";
import { emitSettingsChanged } from "../../settings/events";

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

describe("BrowseView cached pages", () => {
  const show = () =>
    render(
      <StoreContext.Provider value={createStore()}>
        <BrowseView settings={null} onShowInstalled={() => undefined} />
      </StoreContext.Provider>,
    );
  const card = async (name: string) => (await screen.findByText(name, undefined, { timeout: 5000 })).closest("article")!;

  it("drops a deleted model's Installed badge even when it was deleted while Browse was closed", async () => {
    show();
    await waitFor(async () => expect((await card("Juggernaut XL")).textContent).toContain("Installed"));
    cleanup(); // Browse closes (e.g. the Installed view opens)
    const m = (await api.listModels()).find((x) => x.civitaiVersionId === 782002)!;
    await api.deleteModel(m.id);
    await new Promise((r) => setTimeout(r, 0)); // let the models-changed event arrive
    show();
    await waitFor(async () => expect((await card("Juggernaut XL")).textContent).not.toContain("Installed"), { timeout: 5000 });
  }, 15_000);
});

describe("BrowseView reference picture filter", () => {
  it("shows only models that take a reference picture, each with the badge", async () => {
    render(
      <StoreContext.Provider value={createStore()}>
        <ScrollRootContext.Provider value={document.createElement("div")}>
          <BrowseView settings={null} onShowInstalled={() => undefined} />
        </ScrollRootContext.Provider>
      </StoreContext.Provider>,
    );
    await screen.findByRole("button", { name: "Show Juggernaut XL details" }, { timeout: 5000 });
    fireEvent.click(screen.getByRole("switch", { name: "Reference picture" }));
    const klein = await screen.findByRole("button", { name: "Show FLUX.2 [klein] 4B details" }, { timeout: 5000 });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Show Juggernaut XL details" })).toBeNull(), { timeout: 5000 });
    const cards = screen.getAllByRole("article");
    expect(cards.length).toBeGreaterThan(0);
    for (const c of cards) expect(within(c).getByText("Reference picture")).toBeTruthy();
    expect(within(klein.closest("article")!).getByText("Reference picture")).toBeTruthy();
  }, 15_000);
});

describe("BrowseView More filters", () => {
  it("keeps Tags, Price, Commercial use and Hide anime folded, with a count of what is set", async () => {
    rememberFilters(defaultFilters(null, null, false));
    render(
      <StoreContext.Provider value={createStore()}>
        <ScrollRootContext.Provider value={document.createElement("div")}>
          <BrowseView settings={null} onShowInstalled={() => undefined} />
        </ScrollRootContext.Provider>
      </StoreContext.Provider>,
    );
    const more = await screen.findByRole("button", { name: "More filters" });
    expect(more.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("button", { name: "Portraits" })).toBeNull();
    expect(screen.queryByRole("switch", { name: "Hide anime" })).toBeNull();
    // Always shown.
    expect(screen.getByRole("switch", { name: "Works with Pinhole" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Runs on my card" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Reference picture" })).toBeTruthy();
    fireEvent.click(more);
    fireEvent.click(await screen.findByRole("button", { name: "Portraits" }));
    fireEvent.click(screen.getByRole("switch", { name: "Hide anime" }));
    // Reference picture is its own switch, not one of the folded tags.
    expect(screen.queryByRole("button", { name: "Reference picture" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "More filters" }));
    expect(screen.getByRole("button", { name: /^More filters\s*2$/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Portraits" })).toBeNull();
  }, 15_000);

  it("starts open when one of its filters is already set", async () => {
    rememberFilters({ ...defaultFilters(null, null, false), hideAnime: true });
    render(
      <StoreContext.Provider value={createStore()}>
        <ScrollRootContext.Provider value={document.createElement("div")}>
          <BrowseView settings={null} onShowInstalled={() => undefined} />
        </ScrollRootContext.Provider>
      </StoreContext.Provider>,
    );
    expect((await screen.findByRole("button", { name: "More filters" })).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("switch", { name: "Hide anime" }).getAttribute("aria-checked")).toBe("true");
  }, 15_000);
});

describe("BrowseView tags with Safe mode on", () => {
  it("doesn't show tags that need Safe mode off", async () => {
    rememberFilters(defaultFilters(null, null, false));
    render(
      <StoreContext.Provider value={createStore()}>
        <BrowseView settings={null} onShowInstalled={() => undefined} />
      </StoreContext.Provider>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "More filters" }, { timeout: 5000 }));
    await screen.findByRole("button", { name: "Portraits" });
    const offOnly = (await api.catalogFilters()).tags.filter((t) => t.needsSafeModeOff);
    expect(offOnly.length).toBeGreaterThan(0);
    for (const t of offOnly) expect(screen.queryByRole("button", { name: t.label })).toBeNull();
  }, 10_000);
});

describe("BrowseView follows Safe mode turned on in Settings", () => {
  const offFilters = async () => {
    const offOnly = (await api.catalogFilters()).tags.filter((t) => t.needsSafeModeOff).map((t) => t.key);
    return { ...defaultFilters(null, null, false), content: "all" as const, tags: offOnly };
  };

  it("opens with Safe mode on when Browse was left with it off", async () => {
    rememberFilters(await offFilters());
    const settings = { ...(await api.getSettings()), contentMode: "safe" as const };
    render(
      <StoreContext.Provider value={createStore()}>
        <BrowseView settings={settings} onShowInstalled={() => undefined} />
      </StoreContext.Provider>,
    );
    await waitFor(() => expect(getLastFilters()).toMatchObject({ content: "safe", tags: [] }));
  });

  it("turns it on while Browse is open", async () => {
    rememberFilters(await offFilters());
    render(
      <StoreContext.Provider value={createStore()}>
        <BrowseView settings={null} onShowInstalled={() => undefined} />
      </StoreContext.Provider>,
    );
    await waitFor(() => expect(getLastFilters()?.content).toBe("all"));
    emitSettingsChanged({ ...(await api.getSettings()), contentMode: "safe" });
    await waitFor(() => expect(getLastFilters()).toMatchObject({ content: "safe", tags: [] }));
  });
});
