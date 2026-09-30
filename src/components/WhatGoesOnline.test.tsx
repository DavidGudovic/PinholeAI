// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { installMocks } from "../lib/mock";
import { AppProvider } from "../lib/state/AppProvider";
import { createStore } from "../lib/state/store";
import * as api from "../lib/api";
import { onSettingsChanged } from "../settings/events";
import { ONLINE_CALLS, OnlineBadge } from "./WhatGoesOnline";

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

const show = async (offline: boolean) => {
  const store = createStore();
  store.dispatch({ type: "setSettings", settings: { ...(await api.getSettings()), offline } });
  render(
    <AppProvider store={store}>
      <OnlineBadge />
    </AppProvider>,
  );
  return store;
};

describe("Offline / Online badge", () => {
  it("says Online when Offline mode is off, Offline when it is on", async () => {
    await show(false);
    expect(screen.getByRole("button", { name: /Online/ })).toBeTruthy();
    cleanup();
    await show(true);
    expect(screen.getByRole("button", { name: /Offline/ })).toBeTruthy();
  });

  it("opens the list of everything that goes online", async () => {
    await show(false);
    fireEvent.click(screen.getByRole("button", { name: /Online/ }));
    const dialog = await screen.findByRole("dialog", { name: "What goes online" });
    for (const c of ONLINE_CALLS) expect(dialog.textContent).toContain(c.when);
    expect(dialog.textContent).toMatch(/civitai\.com, huggingface\.co and github\.com/);
  });

  it("can switch Offline mode from the sheet", async () => {
    await show(false);
    const seen: boolean[] = [];
    const off = onSettingsChanged((s) => seen.push(s.offline));
    fireEvent.click(screen.getByRole("button", { name: /Online/ }));
    fireEvent.click(await screen.findByRole("button", { name: "Turn Offline mode on" }));
    await waitFor(() => expect(seen).toEqual([true]));
    off();
  });
});
