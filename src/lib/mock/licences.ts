// Mock model licences (RELEASE-SPEC §6), mirroring `license_accept` in config/models.yaml and
// Rust `licence::require`: a download of a family or helper with a licence the user hasn't
// accepted fails with "license_needed" (details = the licence id).
import type { CoreError } from "../types";
import { mockSettings } from "./app";

const LICENCES: Record<string, [id: string, note: string]> = {
  flux1_dev: ["flux1-dev-non-commercial", "Non-commercial license"],
  flux1_kontext: ["flux1-dev-non-commercial", "Non-commercial license"],
  flux2_dev: ["flux2-dev-non-commercial", "Non-commercial license"],
  flux2_klein_9b: ["flux2-klein-9b-non-commercial", "Non-commercial license"],
  krea2_turbo: ["krea2-community", "Krea 2 Community License: commercial use only under $1M yearly revenue"],
  qwen_image_21: ["qwen-research", "Qwen Research License"],
  // The small Describe helper (Qwen2.5-VL 3B).
  describe: ["qwen-research", "Qwen Research License"],
};

/** Throws "license_needed" unless the licence of `familyOrHelper` (if any) was accepted. */
export function requireLicence(familyOrHelper: string | null | undefined) {
  const l = familyOrHelper ? LICENCES[familyOrHelper] : undefined;
  if (!l || (mockSettings().acceptedLicenses ?? []).includes(l[0])) return;
  const e: CoreError = { code: "license_needed", message: `This model comes with its own licence: ${l[1]}.`, details: l[0] };
  throw e;
}

/** Like Rust `accept_license`: only ids from the shipped list are accepted. */
export function isKnownLicence(id: string): boolean {
  return Object.values(LICENCES).some(([l]) => l === id);
}
