// Other tabs → Create: "Use these settings" on a model's details page sends
// generation text here; the Create tab applies it through Paste from CivitAI.
// PRIVACY: held in memory only until Create takes it.

type Listener = (text: string) => void;

let pending: string | null = null;
let listener: Listener | null = null;

/** Hand generation text to the Create tab (applied now if it is mounted, else when it mounts). */
export function sendGenerationToCreate(text: string): void {
  if (listener) listener(text);
  else pending = text;
}

/** Reset: forget text that Create hasn't taken yet. */
export function clearGenerationHandoff(): void {
  pending = null;
}

/** Create tab: receive handed-over text. Returns an unsubscribe function. */
export function onGenerationHandoff(cb: Listener): () => void {
  listener = cb;
  if (pending != null) {
    const t = pending;
    pending = null;
    cb(t);
  }
  return () => {
    if (listener === cb) listener = null;
  };
}
