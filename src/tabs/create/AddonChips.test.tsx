// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const api = await import("../../lib/api");
const { installMocks } = await import("../../lib/mock");
const { AppProvider } = await import("../../lib/state/AppProvider");
const { createStore } = await import("../../lib/state/store");
const { AddonChips } = await import("./AddonChips");

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

describe("add-on chips: trigger words", () => {
  it("shows the words a chip adds, lets the user leave one out, and saves their own list", async () => {
    const film = (await api.listLoras()).find((l) => l.trainedWords.includes("film photo"))!;
    const store = createStore();
    store.dispatch({ type: "patchCreate", patch: { loras: [{ loraId: film.id, weight: 0.8 }] } });
    render(
      <AppProvider store={store}>
        <AddonChips model={null} />
      </AppProvider>,
    );
    await waitFor(() => expect(screen.getByText(/\+ film photo/)).toBeTruthy());
    expect(screen.getByText(/\+ film photo/).textContent).toContain("+1");

    fireEvent.click(screen.getByTitle("Change strength"));
    const kodak = screen.getByRole("button", { name: "kodak portra 400" });
    expect(kodak.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(kodak);
    expect(store.getState().create.loras[0].words).toEqual(["film photo"]);
    expect(screen.getByRole("button", { name: "kodak portra 400" }).getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const input = screen.getByLabelText("Trigger words");
    fireEvent.change(input, { target: { value: "analog look,  , Analog Look, grain" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await flush();
    await waitFor(() => expect(store.getState().loras.find((l) => l.id === film.id)?.trainedWords).toEqual(["analog look", "grain"]));
    // A new list starts from the default pick again.
    expect(store.getState().create.loras[0]).toEqual({ loraId: film.id, weight: 0.8 });
    const group = screen.getByRole("group", { name: "Style add-ons" });
    expect(within(group).getByText(/\+ analog look/)).toBeTruthy();
  });

  it("adds nothing by default when automatic trigger words are off", async () => {
    const film = (await api.listLoras()).find((l) => l.trainedWords.length > 0)!;
    const store = createStore();
    render(
      <AppProvider store={store}>
        <AddonChips model={null} />
      </AppProvider>,
    );
    await flush();
    await waitFor(() => expect(store.getState().settings).toBeTruthy());
    act(() => {
      store.dispatch({ type: "setSettings", settings: { ...store.getState().settings!, addTriggerWords: false } });
      store.dispatch({ type: "patchCreate", patch: { loras: [{ loraId: film.id, weight: 0.8 }] } });
    });
    await waitFor(() => expect(screen.getByTitle("Change strength")).toBeTruthy());
    expect(screen.queryByText(/^\+ /)).toBeNull();
    fireEvent.click(screen.getByTitle("Change strength"));
    expect(screen.getByText(/Tap a word to add it/)).toBeTruthy();
  });
});
