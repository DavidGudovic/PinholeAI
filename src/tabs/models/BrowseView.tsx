// Models → Browse: CivitAI catalog with plain-language filters (SPEC §5.4).
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { RotateCw, Search, SearchX, WifiOff, X } from "lucide-react";
import { asCoreError, browseCatalog, catalogFilters, listLoras, listModels, onModelsChanged } from "../../lib/api";
import type { CatalogCard, CatalogFilterOptions, ContentMode, CoreError, PriceMode, Settings } from "../../lib/types";
import { Button, ErrorNotice, Segmented, Spinner, Toggle, inputClass } from "../../components/ui";
import { onSettingsChanged } from "../../settings/events";
import { CatalogCardView } from "./CatalogCardView";
import { AdultConfirmDialog, Chip, EmptyState, FilterGroup, Select, Skeleton } from "./controls";
import { InstallDialog } from "./InstallDialog";
import { useDebounced, useTauriEvent } from "./lib/hooks";
import {
  COMMERCIAL_OPTIONS,
  KIND_OPTIONS,
  changedFilterCount,
  defaultFilters,
  filtersKey,
  FALLBACK_OPTIONS,
  isAdult,
  mergePage,
  showPriceBadge,
  toBrowseQuery,
  type BrowseFilters,
} from "./lib/query";
import { confirmAdult, getLastFilters, isAdultConfirmed, rememberFilters } from "./lib/session";

export function BrowseView({ settings, onShowInstalled }: { settings: Settings | null; onShowInstalled: () => void }) {
  const [options, setOptions] = useState<CatalogFilterOptions>(FALLBACK_OPTIONS);
  const [filters, setFilters] = useState<BrowseFilters>(() => getLastFilters() ?? defaultFilters(null, settings, isAdultConfirmed()));
  const [search, setSearch] = useState(filters.query);
  const debouncedSearch = useDebounced(search, 400);
  const [pendingContent, setPendingContent] = useState<ContentMode | null>(() =>
    !getLastFilters() && settings && isAdult(settings.contentMode) && !isAdultConfirmed() ? settings.contentMode : null,
  );

  const [items, setItems] = useState<CatalogCard[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [partial, setPartial] = useState(false);
  const [offline, setOffline] = useState(false);
  const [phase, setPhase] = useState<"loading" | "more" | "idle">("loading");
  const [error, setError] = useState<CoreError | null>(null);
  const [reloadTick, setReloadTick] = useState(0);
  const [installFor, setInstallFor] = useState<CatalogCard | null>(null);
  const [installedVersions, setInstalledVersions] = useState<Set<number>>(new Set());

  const filtersRef = useRef(filters);
  filtersRef.current = filters;
  const reqId = useRef(0);

  useEffect(() => {
    catalogFilters()
      .then(setOptions)
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    rememberFilters(filters);
  }, [filters]);

  useEffect(() => {
    setFilters((f) => (f.query === debouncedSearch ? f : { ...f, query: debouncedSearch }));
  }, [debouncedSearch]);

  // Offline mode toggled in Settings → reload.
  const offlineSetting = useRef(settings?.offline);
  useEffect(
    () =>
      onSettingsChanged((s) => {
        if (s.offline !== offlineSetting.current) {
          offlineSetting.current = s.offline;
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
    void refreshInstalled();
  }, [refreshInstalled]);
  useTauriEvent(onModelsChanged, () => void refreshInstalled());

  const fetchPage = useCallback(async (cursor: string | null) => {
    const id = ++reqId.current;
    setPhase(cursor ? "more" : "loading");
    setError(null);
    try {
      const page = await browseCatalog(toBrowseQuery(filtersRef.current, cursor));
      if (id !== reqId.current) return;
      setOffline(page.offline);
      setItems((prev) => (cursor ? mergePage(prev, page.items) : page.items));
      setNextCursor(page.nextCursor);
      setPartial(page.partial);
    } catch (e) {
      if (id === reqId.current) setError(asCoreError(e));
    } finally {
      if (id === reqId.current) setPhase("idle");
    }
  }, []);

  const key = filtersKey(filters);
  useEffect(() => {
    void fetchPage(null);
  }, [key, reloadTick, fetchPage]);

  // Infinite scroll (not when the backend hit its extra-request cap: then "Load more").
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !nextCursor || partial || phase !== "idle" || error || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((es) => es.some((e) => e.isIntersecting) && void fetchPage(nextCursor), { rootMargin: "600px" });
    io.observe(el);
    return () => io.disconnect();
  }, [nextCursor, partial, phase, error, fetchPage]);

  const update = (patch: Partial<BrowseFilters>) => setFilters((f) => ({ ...f, ...patch }));
  const setContent = (m: ContentMode) => {
    if (isAdult(m) && !isAdultConfirmed()) setPendingContent(m);
    else update({ content: m });
  };
  const defaults = defaultFilters(options, settings, isAdultConfirmed());
  const changed = changedFilterCount(filters, { ...defaults, kind: filters.kind });
  const clearFilters = () => {
    setSearch("");
    setFilters({ ...defaults, kind: filters.kind });
  };

  const showPrice = showPriceBadge(filters.price);
  const onInstall = useCallback((c: CatalogCard) => setInstallFor(c), []);

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
              onChange={(e) => setSearch(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && update({ query: search })}
              placeholder={filters.kind === "models" ? "Search models on CivitAI" : "Search style add-ons on CivitAI"}
              aria-label="Search CivitAI"
              className={`${inputClass} h-9 py-1 pl-8`}
            />
          </div>
          <Segmented ariaLabel="Kind" options={KIND_OPTIONS} value={filters.kind} onChange={(kind) => update({ kind })} />
          <Select label="Sort" value={filters.sort} onChange={(sort) => update({ sort })} options={options.sorts.map((s) => ({ value: s.api, label: s.label }))} />
          <Select label="Time" value={filters.period} onChange={(period) => update({ period })} options={options.periods.map((p) => ({ value: p.api, label: p.label }))} />
        </div>

        <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Look">
          <span className="mr-1 text-xs font-medium text-neutral-500 dark:text-neutral-400">Look</span>
          <Chip active={filters.look === null} onClick={() => update({ look: null })}>
            Any
          </Chip>
          {options.looks.map((l) => (
            <Chip key={l.key} active={filters.look === l.key} onClick={() => update({ look: filters.look === l.key ? null : l.key })}>
              {l.label}
            </Chip>
          ))}
        </div>

        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 border-t border-neutral-100 pt-3 dark:border-neutral-800">
          <FilterGroup label="Content">
            <Segmented ariaLabel="Content" options={options.content.map((c) => ({ value: c.key, label: c.label }))} value={filters.content} onChange={setContent} />
          </FilterGroup>
          <FilterGroup label="Price">
            <Select<PriceMode> label="Price" value={filters.price} onChange={(price) => update({ price })} options={options.price.map((p) => ({ value: p.key, label: p.label }))} />
          </FilterGroup>
          <FilterGroup label="Commercial use">
            <Segmented
              ariaLabel="Commercial use"
              options={COMMERCIAL_OPTIONS}
              value={filters.commercialOnly ? "ok" : "any"}
              onChange={(v) => update({ commercialOnly: v === "ok" })}
            />
          </FilterGroup>
          <Toggle checked={filters.compatibleOnly} onChange={(v) => update({ compatibleOnly: v })} label={<span className="text-sm">Works with Pinhole</span>} />
          {changed > 0 && (
            <button type="button" onClick={clearFilters} className="ml-auto inline-flex items-center gap-1 text-xs text-neutral-500 hover:text-neutral-900 hover:underline dark:hover:text-neutral-100">
              <X className="h-3.5 w-3.5" /> Clear filters
            </button>
          )}
        </div>
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
          <Button onClick={() => setReloadTick((t) => t + 1)}>
            <RotateCw className="h-4 w-4" /> Try again
          </Button>
        </div>
      ) : phase === "loading" && items.length === 0 ? (
        <Grid>
          {Array.from({ length: 10 }, (_, i) => (
            <div key={i} className="overflow-hidden rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
              <Skeleton className="aspect-[4/5] rounded-none" />
              <div className="space-y-2 p-3">
                <Skeleton className="h-4 w-3/4" />
                <Skeleton className="h-3 w-1/2" />
                <Skeleton className="h-8 w-full" />
              </div>
            </div>
          ))}
        </Grid>
      ) : items.length === 0 ? (
        <EmptyState
          icon={<SearchX className="h-6 w-6" />}
          title={filters.kind === "models" ? "No models match these filters" : "No style add-ons match these filters"}
          actions={changed > 0 ? <Button onClick={clearFilters}>Clear filters</Button> : undefined}
        >
          Try another look, a longer time range ("All time"), or a different search.
        </EmptyState>
      ) : (
        <div className={phase === "loading" ? "opacity-60 transition-opacity" : "transition-opacity"} aria-busy={phase !== "idle"}>
          <Grid>
            {items.map((c) => (
              <CatalogCardView
                key={c.versionId}
                card={c}
                content={filters.content}
                showPrice={showPrice}
                installed={c.installed || installedVersions.has(c.versionId)}
                onInstall={onInstall}
              />
            ))}
          </Grid>
          <div ref={sentinel} className="flex flex-col items-center gap-2 py-6">
            {error && <ErrorNotice error={error} />}
            {phase === "more" ? (
              <span className="inline-flex items-center gap-2 text-sm text-neutral-500">
                <Spinner className="h-4 w-4 text-amber-500" /> Loading more…
              </span>
            ) : nextCursor ? (
              <>
                {partial && <span className="text-xs text-neutral-500">Some results were filtered out. There may be more.</span>}
                <Button onClick={() => void fetchPage(nextCursor)}>Load more</Button>
              </>
            ) : (
              <span className="text-xs text-neutral-400">That's everything for these filters.</span>
            )}
          </div>
        </div>
      )}

      <InstallDialog versionId={installFor?.versionId ?? null} title={installFor?.name} onClose={() => setInstallFor(null)} />
      <AdultConfirmDialog
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
