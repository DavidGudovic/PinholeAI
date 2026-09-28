// OWNER: frontend B. "Recommended for your GPU" cards (SPEC §6.1), reused by the
// Create / Edit / Describe empty states (frontend A). Keep this export signature.
export function RecommendedCards(props: {
  /** Subset of roles to show (realistic | anime | edit | describe); default all. */
  roles?: string[];
  /** Smaller layout for empty states inside a tab. */
  compact?: boolean;
  /** Show the "Get all" button (first run). */
  showGetAll?: boolean;
}) {
  void props;
  return null;
}
