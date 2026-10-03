// Choices in braces: `a {red|blue|green} car` makes one picture per combination.
// PRIVACY: the expanded prompts are memory only, like the prompt they come from.

/** Most pictures one Generate makes from choices. */
export const MAX_CHOICE_PICTURES = 16;

/** A `{…|…}` group: braces with at least one `|` and no braces inside. */
const GROUP = /\{([^{}]*\|[^{}]*)\}/g;

export interface Choices {
  /** The prompts to make, in order (at most `cap`, never empty text). Just the prompt when it has no choices. */
  prompts: string[];
  /** How many different prompts the choices make before the cap. */
  total: number;
}

const tidy = (s: string) => s.replace(/\s+/g, " ").replace(/\s+([,.;:!?])/g, "$1").trim();

/**
 * Every combination of the prompt's `{a|b}` groups, first group changing slowest. Braces
 * without a `|` stay as they are; repeated options count once.
 */
export function expandChoices(prompt: string, cap = MAX_CHOICE_PICTURES): Choices {
  const parts: string[] = [];
  const groups: string[][] = [];
  let last = 0;
  for (const m of prompt.matchAll(GROUP)) {
    parts.push(prompt.slice(last, m.index));
    groups.push([...new Set(m[1].split("|").map((o) => o.trim()))]);
    last = m.index + m[0].length;
  }
  parts.push(prompt.slice(last));
  if (!groups.length) return { prompts: [prompt], total: 1 };

  const combinations = groups.reduce((n, g) => n * g.length, 1);
  const prompts: string[] = [];
  // Bounded: options that repeat each other's text ("{x|}{x|}") make fewer distinct prompts.
  const tries = Math.min(combinations, cap * 64);
  let i = 0;
  for (; i < tries && prompts.length < cap; i++) {
    // Mixed-radix digits of i, last group fastest.
    let rest = i;
    const picks = new Array<string>(groups.length);
    for (let g = groups.length - 1; g >= 0; g--) {
      picks[g] = groups[g][rest % groups[g].length];
      rest = Math.floor(rest / groups[g].length);
    }
    const text = tidy(parts.map((p, k) => p + (k < picks.length ? picks[k] : "")).join(""));
    if (text && !prompts.includes(text)) prompts.push(text);
  }
  // Every combination looked at: the count is exact. Else (cut by the cap) the combinations.
  return { prompts, total: i === combinations ? prompts.length : combinations };
}

/** How many pictures one Generate makes from `prompt` (null when it has no choices). */
export function choiceCount(prompt: string): { count: number; total: number } | null {
  const c = expandChoices(prompt);
  return c.prompts.length > 1 ? { count: c.prompts.length, total: c.total } : null;
}
