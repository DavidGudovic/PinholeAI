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

interface Pending {
  id: string;
  message: string;
  answer: Promise<boolean>;
  settle: (accepted: boolean) => void;
}
const queue: Pending[] = [];

function showNext() {
  const next = queue[0];
  set(next ? { id: next.id, message: next.message, resolve: next.settle } : null);
}

/** Shows the prompt; resolves true on "I accept". Asks for the same licence at once (e.g. "Get
 *  all" starting Realistic and Edit with one licence) share one prompt; other licences wait. */
export function askLicence(id: string, message: string): Promise<boolean> {
  const same = queue.find((p) => p.id === id);
  if (same) return same.answer;
  let settle!: (accepted: boolean) => void;
  const answer = new Promise<boolean>((resolve) => {
    settle = (accepted) => {
      queue.splice(queue.indexOf(pending), 1);
      resolve(accepted);
      showNext();
    };
  });
  const pending: Pending = { id, message, answer, settle };
  queue.push(pending);
  if (queue.length === 1) showNext();
  return answer;
}

/** The error an install gives when the licence wasn't accepted. */
export const LICENCE_DECLINED: CoreError = {
  code: "cancelled",
  message: "Nothing was downloaded: the model's licence wasn't accepted.",
  details: null,
};
