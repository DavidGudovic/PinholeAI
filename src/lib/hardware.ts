import type { Settings } from "./types";

/** The Settings fields that change the effective hardware (and so every fit badge). */
export const hardwareKey = (s: Settings) => JSON.stringify([s.gpu, s.vramOverrideGb, s.engineBackend]);
