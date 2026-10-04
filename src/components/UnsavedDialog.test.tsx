// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";

const { installMocks } = await import("../lib/mock");
const { AppProvider } = await import("../lib/state/AppProvider");
const { createStore } = await import("../lib/state/store");
const { UnsavedDialog } = await import("./UnsavedDialog");

beforeAll(async () => {
  await installMocks();
});
afterEach(cleanup);

function open(what: "close" | "clear" | "edit") {
  const store = createStore();
  render(
    <AppProvider store={store}>
      <UnsavedDialog />
    </AppProvider>,
  );
  act(() => store.dispatch({ type: "askLeave", what }));
}

describe("UnsavedDialog", () => {
  it("warns in red that Reset deletes unsaved images, the prompt and Fine-tune settings", () => {
    open("clear");
    const warning = screen.getByText("Unsaved images, the prompt and Fine-tune settings are permanently deleted.");
    expect(warning.className).toContain("text-red-700");
    expect(warning.className).toContain("dark:text-red-400");
    expect(screen.getByText("Nothing is saved until you press Save.")).toBeTruthy();
    expect(screen.queryByText(/for good/)).toBeNull();
  });

  it("names only images when closing or editing another image", () => {
    open("close");
    expect(screen.getByText("Unsaved images are permanently deleted.")).toBeTruthy();
    cleanup();
    open("edit");
    expect(screen.getByText("Unsaved edited images are permanently deleted.")).toBeTruthy();
  });
});
