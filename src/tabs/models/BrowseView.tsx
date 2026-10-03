// Models → Browse: CivitAI catalog with plain-language filters (SPEC §5.4).
//
// Speed: pages are cached in RAM per filters (going back to filters you used shows the grid
// at once), the next page is fetched ahead while you look at this one, filter clicks are
// debounced, cards are memoised, only the rows near the screen are in the page (VirtualGrid),
// and previews load on-screen first (lib/preview.ts). Nothing is written to disk.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, RotateCw, Search, SearchX, WifiOff, X } from "lucide-react";
import { asCoreError, browseCatalog, catalogFilters, getSettings, listLoras, listModels, onModelsChanged, setSettings } from "../../lib/api";
import type { BrowsePage, CatalogCard, CatalogFilterOptions, ContentMode, CoreError, PriceMode, Settings } from "../../lib/types";
import { Button, ErrorNotice, Segmented, Toggle, cx, focusRing, inputClass } from "../../components/ui";
import { emitSettingsChanged, onSettingsChanged } from "../../settings/events";
import { createModels } from "../../lib/state/model";
import { useAppState } from "../../lib/state/store";
import { CatalogCardView } from "./CatalogCardView";
import { Chip, EmptyState, FilterGroup, SafeModeOffDialog, ScrollRow, Select, Skeleton } from "./controls";
import { InstallDialog } from "./InstallDialog";
import { ModelDetails } from "./ModelDetails";
import { VirtualGrid } from "./VirtualGrid";
import { useDebounced, useTauriEvent } from "./lib/hooks";
import { PageStore } from "./lib/pageStore";
import { measureSince } from "./lib/perf";
import { useScrollRoot } from "./lib/preview";
import {
  COMMERCIAL_OPTIONS,
  KIND_OPTIONS,
  NO_TOTALS,
  addTotals,
  changedFilterCount,
  defaultFilters,
  filtersKey,
  forModelOf,
  FALLBACK_OPTIONS,
  isSafeModeOff,
  mergePage,
  moreFiltersCount,
  REFERENCE_TAG,
  resultsSummary,
  showPriceBadge,
  tagsWithSafeMode,
  toBrowseQuery,
  toggleTag,
  visibleTags,
  withSettingsSafeMode,
  type BrowseFilters,
  type BrowseTotals,
} from "./lib/query";
import { confirmAdult, getLastFilters, isAdultConfirmed, onAddonRequest, rememberFilters, takeAddonRequest } from "./lib/session";

/** Filter clicks settle for this long before CivitAI is asked (search text waits longer). */
const FILTER_DEBOUNCE_MS = 200;
const SEARCH_DEBOUNCE_MS = 400;
/** Ask for the next page this far before the end of the grid comes into view. */
const SCROLL_AHEAD = "1600px 0px";
// Automatic "keep looking" rounds when nothing matches yet (about 6 CivitAI requests each).
const AUTO_LOOK_ROUNDS = 8;

/** Recent pages for this session (RAM only). */
const pages = new PageStore();
/**
 * Cached cards carry an `installed` flag, so an install or delete must empty the cache even
 * while Browse isn't shown (e.g. a delete in Installed). Subscribed once, for the whole session.
 */
let watchingModels: Promise<unknown> | null = null;
function clearPagesOnModelsChanged() {
  watchingModels ??= onModelsChanged(() => pages.clear()).catch(() => {
    watchingModels = null;
  });
}

/** Settings that change what cards say (Offline, GPU/VRAM → fit badges). */
function cardSettingsKey(s: Settings | null): string {
  return s ? JSON.stringify([s.offline, s.gpu, s.vramOverrideGb, s.engineBackend]) : "";
}

export function BrowseView({ settings, onShowInstalled }: { settings: Settings | null; onShowInstalled: () => void }) {
  const [options, setOptions] = useState<CatalogFilterOptions>(FALLBACK_OPTIONS);
  const [filters, setFilters] = useState<BrowseFilters>(() => {
    const last = getLastFilters();
    const f = last ? withSettingsSafeMode(last, settings?.contentMode, FALLBACK_OPTIONS) : defaultFilters(null, settings, isAdultConfirmed());
    const forModel = takeAddonRequest();
    return forModel ? addonsFor(f, forModel) : f;
  });
  const [search, setSearch] = useState(filters.query);
  // "More filters" starts open whenever one of them is set, so nothing set is hidden.
  const [moreOpen, setMoreOpen] = useState(() => moreFiltersCount(filters, defaultFilters(null, settings, isAdultConfirmed())) > 0);
  const debouncedSearch = useDebounced(search, SEARCH_DEBOUNCE_MS);
  const [pendingContent, setPendingContent] = useState<ContentMode | null>(() =>
    !getLastFilters() && settings && isSafeModeOff(settings.contentMode) && !isAdultConfirmed() ? settings.contentMode : null,
  );

  const [items, setItems] = useState<CatalogCard[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [partial, setPartial] = useState(false);
  const [totals, setTotals] = useState<BrowseTotals>(NO_TOTALS);
  const [offline, setOffline] = useState(false);
  const [phase, setPhase] = useState<"loading" | "more" | "idle">("loading");
  const [error, setError] = useState<CoreError | null>(null);
  const [reloadTick, setReloadTick] = useState(0);
  const [installFor, setInstallFor] = useState<CatalogCard | null>(null);
  const [detailsFor, setDetailsFor] = useState<CatalogCard | null>(null);
  const [installedVersions, setInstalledVersions] = useState<Set<number>>(new Set());

  const optionsRef = useRef(options);
  optionsRef.current = options;
  const filtersRef = useRef(filters);
  filtersRef.current = filters;
  // Installed models: "For" choices, and the family sent for "add-ons for this model".
  const models = useAppState((s) => s.models);
  const createModelId = useAppState((s) => s.create.modelId);
  const forChoices = createModels(models).filter((m) => m.familyId);
  const modelsRef = useRef(models);
  modelsRef.current = models;
  const familyOf = useCallback((f: BrowseFilters) => {
    const id = forModelOf(f);
    return (id && modelsRef.current?.find((m) => m.id === id)?.familyId) || null;
  }, []);
  // The family actually sent is part of the key: pages asked before the models list loaded
  // (or after the model was deleted) aren't narrowed, and mustn't be reused as if they were.
  const keyOf = useCallback((f: BrowseFilters) => `${filtersKey(f)}|${familyOf(f) ?? ""}`, [familyOf]);
  const reqId = useRef(0);
  /** First visit this session, and the user hasn't touched a filter yet. */
  const pristine = useRef(!getLastFilters());
  /** Filters key of the cards in the grid. */
  const shownKey = useRef<string | null>(null);

  useEffect(() => {
    catalogFilters()
      .then((o) => {
        setOptions(o);
        // First visit this session: open on the catalog's own defaults (catalog-filters.yaml).
        if (pristine.current) setFilters((f) => ({ ...defaultFilters(o, settings, isAdultConfirmed()), kind: f.kind, forModel: f.forModel, query: f.query }));
      })
      .catch(() => undefined);
    // Once per mount; `settings` is only read for the opening defaults.
  }, []);

  useEffect(() => {
    rememberFilters(filters);
  }, [filters]);

  // "Find style add-ons" from Create or Installed while this view is open.
  useEffect(
    () =>
      onAddonRequest((id) => {
        takeAddonRequest();
        setDetailsFor(null);
        setSearch("");
        setFilters((f) => addonsFor(f, id));
      }),
    [],
  );

  useEffect(() => {
    setFilters((f) => (f.query === debouncedSearch ? f : { ...f, query: debouncedSearch }));
  }, [debouncedSearch]);

  // Offline mode / GPU changes in Settings → cached cards are stale: reload.
  const cardSettings = useRef(cardSettingsKey(settings));
  useEffect(
    () =>
      onSettingsChanged((s) => {
        setFilters((f) => withSettingsSafeMode(f, s.contentMode, optionsRef.current));
        const k = cardSettingsKey(s);
        if (k !== cardSettings.current) {
          cardSettings.current = k;
          pages.clear();
          shownKey.current = null;
          setReloadTick((t) => t + 1);
        }
      }),
    [],
  );

  const refreshInstalled = useCallback(async () => {
    try {
      const [m, l] = await Promise.all([listModels(), listLoras()]);
      setInstalledVersions(new Set([...m.map((x) => x.civitaiVersionId), ...l.map((x) => x.civitaiVersionId)].filter((x): x is number => x != null)));
    } catch {
      /* the cards' own `installed` flag still applies */
    }
  }, []);
  useEffect(() => {
    clearPagesOnModelsChanged();
    void refreshInstalled();
  }, [refreshInstalled]);
  useTauriEvent(onModelsChanged, () => {
    // Cached cards carry an `installed` flag: don't reuse them after an install/delete.
    pages.clear();
    void refreshInstalled();
  });

  const showPages = useCallback((key: string, list: BrowsePage[]) => {
    const last = list[list.length - 1];
    setItems(list.slice(1).reduce((acc, p) => mergePage(acc, p.items), list[0].items));
    setTotals(list.reduce(addTotals, NO_TOTALS));
    setOffline(last.offline);
    // Offline pages echo the request cursor (Rust): don't keep paging against it.
    setNextCursor(last.offline ? null : last.nextCursor);
    setPartial(last.partial);
    shownKey.current = key;
  }, []);

  // Rounds of "nothing matched yet, look further" run without a click (one round = up to
  // a few CivitAI requests); after that the "Keep looking" button takes over.
  const autoRounds = useRef(0);

  const fetchPage = useCallback(
    async (cursor: string | null, retry = true) => {
      const f = filtersRef.current;
      const key = keyOf(f);
      const id = ++reqId.current;
      if (!cursor) autoRounds.current = 0;
      setPhase(cursor ? "more" : "loading");
      setError(null);
      const started = performance.now();
      try {
        const page = await pages.load(key, cursor, () => browseCatalog(toBrowseQuery(f, cursor), familyOf(f)));
        measureSince(cursor ? "pinhole:browse-more" : "pinhole:browse-first", started);
        if (id !== reqId.current) return;
        if (!cursor) showPages(key, [page]);
        else {
          setItems((prev) => mergePage(prev, page.items));
          setTotals((t) => addTotals(t, page));
          setNextCursor(page.offline ? null : page.nextCursor);
          setPartial(page.partial);
          setOffline(page.offline);
        }
      } catch (e) {
        if (id !== reqId.current) return;
        const ce = asCoreError(e);
        // Rust stopped it because another Browse request came in; ask again once.
        if (ce.code === "cancelled" && retry) {
          void fetchPage(cursor, false);
          return;
        }
        setError(ce);
      } finally {
        if (id === reqId.current) setPhase("idle");
      }
    },
    [showPages, familyOf, keyOf],
  );

  // Filters changed: show the cached grid right away when there is one…
  const liveKey = keyOf(filters);
  useEffect(() => {
    const chain = pages.chain(liveKey);
    if (!chain) return;
    reqId.current += 1;
    autoRounds.current = 0;
    showPages(liveKey, chain.pages);
    setError(null);
    setPhase("idle");
  }, [liveKey, showPages]);

  // …otherwise ask CivitAI once the clicks settle.
  const key = useDebounced(liveKey, FILTER_DEBOUNCE_MS);
  useEffect(() => {
    if (key !== keyOf(filtersRef.current)) return; // still changing
    if (shownKey.current === key) return; // restored from the cache
    void fetchPage(null);
  }, [key, reloadTick, fetchPage, keyOf]);

  // Fetch the next page ahead, while the user looks at this one.
  useEffect(() => {
    if (phase !== "idle" || !nextCursor || partial || error || offline) return;
    const k = shownKey.current;
    const f = filtersRef.current;
    if (!k || k !== keyOf(f) || pages.has(k, nextCursor)) return;
    const t = setTimeout(() => void pages.load(k, nextCursor, () => browseCatalog(toBrowseQuery(f, nextCursor), familyOf(f))).catch(() => undefined), 250);
    return () => clearTimeout(t);
  }, [phase, nextCursor, partial, error, offline, familyOf, keyOf]);

  // Nothing matched in the pages checked so far: keep looking on our own.
  useEffect(() => {
    if (phase !== "idle" || items.length > 0 || !nextCursor || !partial || error || offline) return;
    if (autoRounds.current >= AUTO_LOOK_ROUNDS) return;
    autoRounds.current += 1;
    void fetchPage(nextCursor);
  }, [phase, items.length, nextCursor, partial, error, offline, fetchPage]);

  // Infinite scroll (not when the backend hit its extra-request cap: then "Load more").
  const sentinel = useRef<HTMLDivElement>(null);
  // Observe relative to the tab's own scroller: with the viewport as root the margin is ignored.
  const scrollRoot = useScrollRoot();
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !nextCursor || partial || phase !== "idle" || error || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((es) => es.some((e) => e.isIntersecting) && void fetchPage(nextCursor), {
      root: scrollRoot,
      rootMargin: SCROLL_AHEAD,
    });
    io.observe(el);
    return () => io.disconnect();
  }, [nextCursor, partial, phase, error, fetchPage, scrollRoot]);

  const update = (patch: Partial<BrowseFilters>) => {
    pristine.current = false;
    setFilters((f) => ({ ...f, ...patch }));
  };
  const setContent = (m: ContentMode) => {
    if (isSafeModeOff(m) && !isAdultConfirmed()) setPendingContent(m);
    else if (isSafeModeOff(m)) update({ content: m });
    else update({ content: m, tags: tagsWithSafeMode(filters.tags, options) });
  };
  // "Hide anime" is remembered across launches (Settings); the list reacts at once.
  const setHideAnime = (hideAnime: boolean) => {
    update(hideAnime && filters.look === "anime" ? { hideAnime, look: null } : { hideAnime });
    void getSettings()
      .then((cur) => setSettings({ ...cur, hideAnime }))
      .then(emitSettingsChanged)
      .catch(() => undefined); // not remembered; this session's switch still applies
  };
  const defaults = defaultFilters(options, settings, isAdultConfirmed());
  const changed = changedFilterCount(filters, { ...defaults, kind: filters.kind });
  const moreCount = moreFiltersCount(filters, defaults);
  const clearFilters = () => {
    pristine.current = false;
    setSearch("");
    setFilters({ ...defaults, kind: filters.kind, forModel: filters.forModel });
  };
  const setKind = (kind: BrowseFilters["kind"]) =>
    // Style add-ons open on the model picked in Create (the user can pick "Any model").
    update(kind === "styleAddons" && filters.forModel === null && createModelId ? { kind, forModel: createModelId } : { kind });
  const forModel = forModelOf(filters);
  const forValue = forModel && forChoices.some((m) => m.id === forModel) ? forModel : "";
  const reload = () => {
    pages.clear();
    shownKey.current = null;
    setReloadTick((t) => t + 1);
  };

  const showPrice = showPriceBadge(filters.price);
  const onInstall = useCallback((c: CatalogCard) => setInstallFor(c), []);
  const onOpen = useCallback((c: CatalogCard) => setDetailsFor(c), []);
  const settling = liveKey !== shownKey.current && phase === "idle" && items.length > 0;
  const summary = resultsSummary(filters, items.length, totals);
  const noun = filters.kind === "models" ? "models" : "style add-ons";

  return (
    <div className="space-y-4">
      {/* ------------------------------------------------ filter bar */}
      <div className="space-y-3 rounded-xl border border-neutral-200 bg-white p-3 shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-56 flex-1">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 h-4 w-4 -translate-y-1/2 text-neutral-400" />
            <input
              type="search"
              spellCheck={false}
              autoComplete="off"
              value={search}
              onChange={(e) => {
                pristine.current = false;
                setSearch(e.target.value);
              }}
              onKeyDown={(e) => e.key === "Enter" && update({ query: search })}
              placeholder={filters.kind === "models" ? "Search models on CivitAI" : "Search style add-ons on CivitAI"}
              aria-label="Search CivitAI"
              className={`${inputClass} h-9 py-1 pl-8`}
            />
          </div>
          <Segmented ariaLabel="Kind" options={KIND_OPTIONS} value={filters.kind} onChange={setKind} />
          {filters.kind === "styleAddons" && forChoices.length > 0 && (
            <Select
              label="For"
              value={forValue}
              onChange={(v) => update({ forModel: v })}
              options={[{ value: "", label: "Any model" }, ...forChoices.map((m) => ({ value: m.id, label: `For ${m.friendlyName}` }))]}
            />
          )}
          <Select label="Sort" value={filters.sort} onChange={(sort) => update({ sort })} options={options.sorts.map((s) => ({ value: s.api, label: s.label }))} />
          <Select label="Time" value={filters.period} onChange={(period) => update({ period })} options={options.periods.map((p) => ({ value: p.api, label: p.label }))} />
        </div>

        <ScrollRow label="Look" className="gap-1.5">
          <Chip active={filters.look === null} onClick={() => update({ look: null })}>
            Any
          </Chip>
          {options.looks.map((l) => (
            <Chip
              key={l.key}
              active={filters.look === l.key}
              disabled={filters.hideAnime && l.key === "anime"}
              title={filters.hideAnime && l.key === "anime" ? "Turn off Hide anime to use this look" : undefined}
              onClick={() => update({ look: filters.look === l.key ? null : l.key })}
            >
              {l.label}
            </Chip>
          ))}
        </ScrollRow>

        {/* One line: scrolls sideways in a narrow window instead of folding. */}
        <div className="border-t border-neutral-100 pt-3 dark:border-neutral-800">
          <ScrollRow className="gap-x-4">
            <FilterGroup label="Safe mode">
              <Segmented ariaLabel="Safe mode" options={options.content.map((c) => ({ value: c.key, label: c.label }))} value={filters.content} onChange={setContent} />
            </FilterGroup>
            <Toggle checked={filters.compatibleOnly} onChange={(v) => update({ compatibleOnly: v })} label={<span className="text-sm whitespace-nowrap">Works with Pinhole</span>} />
            {filters.kind === "models" && (
              <Toggle checked={filters.runsOnMyCard} onChange={(v) => update({ runsOnMyCard: v })} label={<span className="text-sm whitespace-nowrap">Runs on my card</span>} />
            )}
            {options.tags.some((t) => t.key === REFERENCE_TAG) && (
              <Toggle
                checked={filters.tags.includes(REFERENCE_TAG)}
                onChange={(v) => update({ tags: v ? [...filters.tags, REFERENCE_TAG] : filters.tags.filter((t) => t !== REFERENCE_TAG) })}
                label={<span className="text-sm whitespace-nowrap">Reference picture</span>}
              />
            )}
            <button
              type="button"
              aria-expanded={moreOpen}
              aria-controls="browse-more-filters"
              onClick={() => setMoreOpen((o) => !o)}
              className={cx("inline-flex shrink-0 items-center gap-1 rounded text-sm font-medium whitespace-nowrap text-neutral-700 hover:text-neutral-950 dark:text-neutral-300 dark:hover:text-white", focusRing)}
            >
              More filters
              {!moreOpen && moreCount > 0 && (
                <span className="rounded-full bg-amber-500 px-1.5 text-xs leading-5 font-semibold text-neutral-950">{moreCount}</span>
              )}
              <ChevronDown className={cx("h-4 w-4 transition-transform", moreOpen && "rotate-180")} aria-hidden />
            </button>
            {changed > 0 && (
              <button
                type="button"
                onClick={clearFilters}
                className={cx("ml-auto inline-flex items-center gap-1 rounded text-xs whitespace-nowrap text-neutral-500 hover:text-neutral-900 hover:underline dark:hover:text-neutral-100", focusRing)}
              >
                <X className="h-3.5 w-3.5" /> Clear filters
              </button>
            )}
          </ScrollRow>
        </div>

        {moreOpen && (
          <div id="browse-more-filters" className="space-y-3 border-t border-neutral-100 pt-3 dark:border-neutral-800">
            <ScrollRow label="Tags" className="gap-1.5">
              {visibleTags(options.tags, filters.content)
                .filter((t) => t.key !== REFERENCE_TAG)
                .map((t) => (
                  <Chip key={t.key} active={filters.tags.includes(t.key)} onClick={() => update({ tags: toggleTag(filters.tags, t.key) })}>
                    {t.label}
                  </Chip>
                ))}
            </ScrollRow>
            <ScrollRow className="gap-x-4">
              <FilterGroup label="Price">
                <Select<PriceMode> label="Price" className="[&>select]:[field-sizing:content]" value={filters.price} onChange={(price) => update({ price })} options={options.price.map((p) => ({ value: p.key, label: p.label }))} />
              </FilterGroup>
              <FilterGroup label="Commercial use">
                <Segmented
                  ariaLabel="Commercial use"
                  options={COMMERCIAL_OPTIONS}
                  value={filters.commercialOnly ? "ok" : "any"}
                  onChange={(v) => update({ commercialOnly: v === "ok" })}
                />
              </FilterGroup>
              <Toggle checked={filters.hideAnime} onChange={setHideAnime} label={<span className="text-sm whitespace-nowrap">Hide anime</span>} />
            </ScrollRow>
          </div>
        )}
      </div>

      {/* ------------------------------------------------ results */}
      {offline ? (
        <EmptyState
          icon={<WifiOff className="h-6 w-6" />}
          title="Offline — only installed models are shown"
          actions={
            <Button variant="primary" onClick={onShowInstalled}>
              Show installed models
            </Button>
          }
        >
          Offline mode is on, so Pinhole doesn't contact CivitAI. Turn it off in Settings to browse and download new models.
        </EmptyState>
      ) : error && items.length === 0 ? (
        <div className="mx-auto max-w-lg space-y-3 py-10">
          <ErrorNotice error={error} />
          <Button onClick={reload}>
            <RotateCw className="h-4 w-4" /> Try again
          </Button>
        </div>
      ) : (phase === "loading" || phase === "more") && items.length === 0 ? (
        <Grid>
          <SkeletonCards count={12} />
        </Grid>
      ) : items.length === 0 ? (
        nextCursor && partial ? (
          <EmptyState
            icon={<SearchX className="h-6 w-6" />}
            title={`No matching ${noun} yet`}
            actions={
              <>
                <Button variant="primary" onClick={() => void fetchPage(nextCursor)}>
                  Keep looking
                </Button>
                {changed > 0 && <Button onClick={clearFilters}>Clear filters</Button>}
              </>
            }
          >
            Pinhole looked through {totals.checked.toLocaleString("en-US")} {noun} and none matched your filters yet. Keep looking to check more.
          </EmptyState>
        ) : (
          <EmptyState
            icon={<SearchX className="h-6 w-6" />}
            title={filters.kind === "models" ? "No models match these filters" : "No style add-ons match these filters"}
            actions={changed > 0 ? <Button onClick={clearFilters}>Clear filters</Button> : undefined}
          >
            {filters.period === "AllTime" ? "Try another look, fewer tags, or a different search." : 'Try another look, fewer tags, a longer time range ("All time"), or a different search.'}
          </EmptyState>
        )
      ) : (
        <div className={phase === "loading" || settling ? "opacity-60 transition-opacity" : "transition-opacity"} aria-busy={phase !== "idle"}>
          <div className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1 text-xs text-neutral-500 dark:text-neutral-400" aria-live="polite">
            <span className="font-medium text-neutral-700 dark:text-neutral-300">{summary.count}</span>
            {summary.hints.map((h) => (
              <span key={h}>{h}</span>
            ))}
          </div>
          <VirtualGrid
            items={items}
            itemKey={cardKey}
            scrollRoot={scrollRoot}
            renderItem={(c) => (
              <CatalogCardView
                card={c}
                content={filters.content}
                showPrice={showPrice}
                installed={c.installed || installedVersions.has(c.versionId)}
                onInstall={onInstall}
                onOpen={onOpen}
              />
            )}
            after={phase === "more" ? <SkeletonCards count={4} /> : undefined}
          />
          <div ref={sentinel} className="flex flex-col items-center gap-2 py-6">
            {error && <ErrorNotice error={error} />}
            {phase === "more" ? null : nextCursor ? (
              <>
                {partial && (
                  <span className="text-xs text-neutral-500">
                    Pinhole checked {totals.checked.toLocaleString("en-US")} {noun} so far and hid the ones your filters leave out. There may be more.
                  </span>
                )}
                <Button onClick={() => void fetchPage(nextCursor)}>Load more</Button>
              </>
            ) : (
              <span className="text-xs text-neutral-400">That's everything for these filters.</span>
            )}
          </div>
        </div>
      )}

      {detailsFor && (
        <ModelDetails
          card={detailsFor}
          content={filters.content}
          installed={detailsFor.installed || installedVersions.has(detailsFor.versionId)}
          onInstall={onInstall}
          onClose={() => setDetailsFor(null)}
        />
      )}
      <InstallDialog versionId={installFor?.versionId ?? null} title={installFor?.name} onClose={() => setInstallFor(null)} />
      <SafeModeOffDialog
        open={pendingContent !== null}
        onCancel={() => setPendingContent(null)}
        onConfirm={() => {
          confirmAdult();
          if (pendingContent) update({ content: pendingContent });
          setPendingContent(null);
        }}
      />
    </div>
  );
}

function Grid({ children }: { children: ReactNode }) {
  return <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-4">{children}</div>;
}

/** Placeholder cards with the same shape as real ones (no layout jump when they arrive). */
function SkeletonCards({ count }: { count: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <div key={`skeleton-${i}`} aria-hidden className="pinhole-card overflow-hidden rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
          <Skeleton className="aspect-[4/5] rounded-none" />
          <div className="space-y-2 p-3">
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-3 w-1/2" />
            <Skeleton className="h-3 w-2/3" />
            <Skeleton className="h-8 w-full" />
          </div>
        </div>
      ))}
    </>
  );
}

const cardKey = (c: CatalogCard) => c.versionId;

/** Browse style add-ons for one installed model (a fresh search). */
function addonsFor(f: BrowseFilters, modelId: string): BrowseFilters {
  return { ...f, kind: "styleAddons", forModel: modelId, query: "" };
}
