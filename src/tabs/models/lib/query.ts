// Pure helpers for the Models → Browse filter bar (SPEC §5.4).
// The UI keeps a `BrowseFilters` object; `toBrowseQuery` turns it into the IPC
// `BrowseQuery`. The CivitAI mapping itself (types=, nsfw=, baseModels=…) lives in Rust.
import type { BrowseQuery, CatalogCard, CatalogFilterOptions, CatalogKind, ContentMode, PriceMode, Settings } from "../../../lib/types";

export interface BrowseFilters {
  kind: CatalogKind;
  look: string | null;
  content: ContentMode;
  price: PriceMode;
  sort: string;
  period: string;
  commercialOnly: boolean;
  compatibleOnly: boolean;
  query: string;
}

/** Fallback option lists (used until `catalog_filters` answers, and in tests). Labels mirror catalog-filters.yaml. */
export const FALLBACK_OPTIONS: CatalogFilterOptions = {
  looks: [
    { key: "realistic", label: "Realistic" },
    { key: "anime", label: "Anime" },
    { key: "illustration", label: "Illustration" },
    { key: "three_d", label: "3D" },
    { key: "brand", label: "Brand & product" },
  ],
  sorts: [
    { label: "Top rated", api: "Highest Rated" },
    { label: "Most downloaded", api: "Most Downloaded" },
    { label: "Newest", api: "Newest" },
  ],
  periods: [
    { label: "This week", api: "Week" },
    { label: "This month", api: "Month" },
    { label: "This year", api: "Year" },
    { label: "All time", api: "AllTime" },
  ],
  content: [
    { key: "safe", label: "Safe only" },
    { key: "include_18plus", label: "Include 18+" },
    { key: "only_18plus", label: "18+ only" },
  ],
  price: [
    { key: "free", label: "Free" },
    { key: "include", label: "Include early access (paid)" },
    { key: "paid_only", label: "Early access only" },
  ],
  defaultContent: "safe",
  defaultPrice: "free",
};

export const KIND_OPTIONS: { value: CatalogKind; label: string }[] = [
  { value: "models", label: "Models" },
  { value: "styleAddons", label: "Style add-ons" },
];

export const COMMERCIAL_OPTIONS: { value: "any" | "ok"; label: string }[] = [
  { value: "any", label: "Any" },
  { value: "ok", label: "OK for client work" },
];

export function isAdult(mode: ContentMode): boolean {
  return mode === "include_18plus" || mode === "only_18plus";
}

/**
 * Initial filters. Content: the Settings default, but an 18+ default only applies once the
 * user confirmed 18+ this session (otherwise start Safe and ask). Price: "Include early access"
 * when Settings → Show paid is on, else the catalog default (Free).
 */
export function defaultFilters(
  options: CatalogFilterOptions | null,
  settings: Pick<Settings, "contentMode" | "showPaid"> | null,
  adultConfirmed: boolean,
): BrowseFilters {
  const o = options ?? FALLBACK_OPTIONS;
  const wantedContent = settings?.contentMode ?? o.defaultContent;
  const content = isAdult(wantedContent) && !adultConfirmed ? "safe" : wantedContent;
  const price: PriceMode = settings?.showPaid ? "include" : o.defaultPrice;
  return {
    kind: "models",
    look: null,
    content,
    price,
    sort: o.sorts[0]?.api ?? "Highest Rated",
    period: o.periods.find((p) => p.api === "Month")?.api ?? o.periods[0]?.api ?? "AllTime",
    commercialOnly: false,
    compatibleOnly: true,
    query: "",
  };
}

/** Normalise free text the way we send it: trimmed, inner whitespace collapsed, capped. */
export function normalizeSearch(q: string): string {
  return q.replace(/\s+/g, " ").trim().slice(0, 200);
}

export function toBrowseQuery(f: BrowseFilters, cursor: string | null = null): BrowseQuery {
  return {
    kind: f.kind,
    look: f.look,
    content: f.content,
    price: f.price,
    sort: f.sort,
    period: f.period,
    commercialOnly: f.commercialOnly,
    compatibleOnly: f.compatibleOnly,
    query: normalizeSearch(f.query),
    cursor,
  };
}

/** Stable key: a new key means "start over from the first page". */
export function filtersKey(f: BrowseFilters): string {
  const q = toBrowseQuery(f, null);
  return JSON.stringify([q.kind, q.look, q.content, q.price, q.sort, q.period, q.commercialOnly, q.compatibleOnly, q.query]);
}

/** Filters that differ from the defaults (for a "Clear filters" button). */
export function changedFilterCount(f: BrowseFilters, defaults: BrowseFilters): number {
  const keys: (keyof BrowseFilters)[] = ["look", "content", "price", "sort", "period", "commercialOnly", "compatibleOnly", "query"];
  return keys.filter((k) => (k === "query" ? normalizeSearch(f.query) !== normalizeSearch(defaults.query) : f[k] !== defaults[k])).length;
}

/** Price badges are only shown when paid models can appear (SPEC §5.4 model card). */
export function showPriceBadge(price: PriceMode): boolean {
  return price !== "free";
}

/** Blur previews flagged NSFW whenever 18+ is off (catalog-filters.yaml: blur_nsfw_previews_when_safe). */
export function shouldBlurPreview(card: Pick<CatalogCard, "previewNsfw" | "modelNsfw">, content: ContentMode): boolean {
  return !isAdult(content) && (card.previewNsfw || card.modelNsfw);
}

/** Append a page, dropping duplicates (the API can repeat items across cursor pages). */
export function mergePage(existing: CatalogCard[], incoming: CatalogCard[]): CatalogCard[] {
  const seen = new Set(existing.map((c) => c.versionId));
  const out = existing.slice();
  for (const c of incoming) {
    if (!seen.has(c.versionId)) {
      seen.add(c.versionId);
      out.push(c);
    }
  }
  return out;
}
