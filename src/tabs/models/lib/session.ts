// Per-session UI state for the Models tab, kept in RAM only (never in storage):
// the one-time 18+ confirmation (SPEC §5.4) and the last Browse filters / sub-view so
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

export const getLastView = () => lastView;
export const rememberView = (v: "browse" | "installed") => {
  lastView = v;
};
