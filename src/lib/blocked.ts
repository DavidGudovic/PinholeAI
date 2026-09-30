// When the safety check stops something (error code "blocked"), the usage guidelines open again
// with the block message on top (<BlockedNotice />). api.ts reports every "blocked" error here.

let current: string | null = null;
const listeners = new Set<(message: string | null) => void>();

function set(message: string | null) {
  current = message;
  listeners.forEach((l) => l(message));
}

export function currentBlocked(): string | null {
  return current;
}

export function onBlocked(cb: (message: string | null) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function showBlocked(message: string) {
  set(message);
}

export function dismissBlocked() {
  set(null);
}
