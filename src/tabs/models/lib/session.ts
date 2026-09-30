// Per-session UI state for the Models tab, kept in RAM only (never in storage):
// the one-time confirmation for turning Safe mode off (SPEC §5.4) and the last Browse filters / sub-view so
// switching tabs doesn't reset them. Everything is gone when the app closes.
import type { BrowseFilters } from "./query";

let adultConfirmed = false;
let lastFilters: BrowseFilters | null = null;
let lastView: "browse" | "installed" | null = null;

export const isAdultConfirmed = () => adultConfirmed;
export const confirmAdult = () => {
  adultConfirmed = true;
};

export const getLastFilters = () => lastFilters;
export const rememberFilters = (f: BrowseFilters) => {
  lastFilters = f;
};

/** "Find style add-ons for this model" from Create or Installed: Browse picks it up. */
let pendingAddons: string | null = null;
const addonListeners = new Set<(modelId: string) => void>();

export const requestAddonBrowse = (modelId: string) => {
  pendingAddons = modelId;
  for (const l of addonListeners) l(modelId);
};
export const hasAddonRequest = () => pendingAddons !== null;
/** The model id asked for (once), or null. */
export const takeAddonRequest = () => {
  const id = pendingAddons;
  pendingAddons = null;
  return id;
};
export const onAddonRequest = (cb: (modelId: string) => void) => {
  addonListeners.add(cb);
  return () => void addonListeners.delete(cb);
};

export const getLastView = () => lastView;
export const rememberView = (v: "browse" | "installed") => {
  lastView = v;
};
