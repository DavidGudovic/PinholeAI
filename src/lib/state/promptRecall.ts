// Up/Down in the prompt box walks through the prompts sent earlier this session.
// PRIVACY: the list lives in memory only (AppState.promptHistory), never on disk.

/** Where the user is in the history. `index` is into the history array; `draft` is what they had typed before browsing. */
export interface Browse {
  index: number;
  draft: string;
}

export interface Step {
  browse: Browse | null;
  /** New prompt text, or null to leave the box alone. */
  text: string | null;
}

/** One Up (`-1`) or Down (`+1`) press. */
export function recallStep(history: string[], browse: Browse | null, prompt: string, dir: -1 | 1): Step {
  if (!history.length) return { browse, text: null };
  if (dir === -1) {
    const index = browse ? Math.max(0, browse.index - 1) : history.length - 1;
    return { browse: { index, draft: browse ? browse.draft : prompt }, text: history[index] };
  }
  if (!browse) return { browse: null, text: null };
  if (browse.index >= history.length - 1) return { browse: null, text: browse.draft };
  const index = browse.index + 1;
  return { browse: { index, draft: browse.draft }, text: history[index] };
}

/** Should this Up/Down press recall a prompt (rather than just move the caret inside multi-line text)? */
export function shouldRecall(value: string, caretStart: number, caretEnd: number, dir: -1 | 1, browsing: boolean): boolean {
  if (caretStart !== caretEnd) return false;
  if (dir === -1) {
    if (value.slice(0, caretStart).includes("\n")) return false;
    return browsing || caretStart === 0;
  }
  return browsing && !value.slice(caretEnd).includes("\n");
}
