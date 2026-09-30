// Model licence acceptance (RELEASE-SPEC §6). A download that needs a licence the user hasn't
// accepted fails with code "license_needed" (message = the licence, details = its id). The
// install wrappers in api.ts ask once through <LicencePrompt />, save the acceptance and retry.
import type { CoreError } from "./types";

export const LICENSE_NEEDED = "license_needed";

export interface LicenceRequest {
  id: string;
  /** Plain sentence naming the licence. */
  message: string;
  resolve: (accepted: boolean) => void;
}

let current: LicenceRequest | null = null;
const listeners = new Set<(r: LicenceRequest | null) => void>();

function set(r: LicenceRequest | null) {
  current = r;
  listeners.forEach((l) => l(r));
}

export function currentLicenceRequest(): LicenceRequest | null {
  return current;
}

export function onLicenceRequest(cb: (r: LicenceRequest | null) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** Shows the prompt; resolves true on "I accept". A second request while one is open is declined. */
export function askLicence(id: string, message: string): Promise<boolean> {
  if (current) return Promise.resolve(false);
  return new Promise((resolve) => {
    set({
      id,
      message,
      resolve: (accepted) => {
        set(null);
        resolve(accepted);
      },
    });
  });
}

/** The error an install gives when the licence wasn't accepted. */
export const LICENCE_DECLINED: CoreError = {
  code: "cancelled",
  message: "Nothing was downloaded: the model's licence wasn't accepted.",
  details: null,
};
