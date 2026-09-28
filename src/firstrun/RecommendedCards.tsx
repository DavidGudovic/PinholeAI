// OWNER: frontend B. "Recommended for your GPU / computer" cards (SPEC §6.1), reused by the
// Create / Edit / Describe empty states (frontend A). Keep this export signature.
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Camera, CircleCheck, Download, RotateCw, ScanText, Sparkles, WandSparkles } from "lucide-react";
import { asCoreError, getRecommended, installRecommended, onHardwareReady, onModelsChanged } from "../lib/api";
import type { CoreError, GroupStatus, RecommendedPick } from "../lib/types";
import { formatBytes } from "../lib/format";
import { Button, ErrorNotice, Spinner, VramBadge } from "../components/ui";
import { GroupProgress, Skeleton } from "../tabs/models/controls";
import { cancelGroup, getTagged, tagGroup, useDownloadsVersion, useTaggedGroup } from "../tabs/models/lib/downloads";
import { useHardware, useTauriEvent } from "../tabs/models/lib/hooks";
import { isActive, machinePlain, quantPlain } from "../tabs/models/lib/words";

const ROLE_ICON: Record<string, ReactNode> = {
  realistic: <Camera className="h-4 w-4" />,
  // Optional second Realistic card (Krea 2 Turbo on 12 GB+); absent when it doesn't fit.
  realistic_detail: <Camera className="h-4 w-4" />,
  anime: <Sparkles className="h-4 w-4" />,
  edit: <WandSparkles className="h-4 w-4" />,
  // Optional lighter edit model (FLUX.1 Kontext); absent when it doesn't fit.
  edit_alt: <WandSparkles className="h-4 w-4" />,
  describe: <ScanText className="h-4 w-4" />,
};

const GET_ALL_ROLES = ["realistic", "edit"];

export function RecommendedCards(props: {
  /** Subset of roles to show (realistic | realistic_detail | anime | edit | edit_alt | describe); default all. */
  roles?: string[];
  /**
   * Only picks you can still get: "all" = anything not installed; "tightInstalled" = only
   * smaller versions offered because the installed one is a tight fit.
   * Renders nothing when there is none.
   */
  offers?: "all" | "tightInstalled";
  /** Shown above the cards, only when there are cards to show. */
  heading?: ReactNode;
  /** Smaller layout for empty states inside a tab. */
  compact?: boolean;
  /** Show the "Get all" button (first run). */
  showGetAll?: boolean;
}) {
  const { roles, compact = false, showGetAll = false, offers, heading } = props;
  const [picks, setPicks] = useState<RecommendedPick[] | null>(null);
  const [loadError, setLoadError] = useState<CoreError | null>(null);
  const [errors, setErrors] = useState<Record<string, CoreError | null>>({});
  const [starting, setStarting] = useState<Record<string, boolean>>({});
  const hw = useHardware();

  const load = useCallback(async () => {
    try {
      setPicks(await getRecommended());
      setLoadError(null);
    } catch (e) {
      setLoadError(asCoreError(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);
  useTauriEvent(onModelsChanged, () => void load());
  useTauriEvent(onHardwareReady, () => void load());

  const shown = !picks
    ? null
    : !roles?.length
      ? picks
      : roles.map((r) => picks.find((p) => p.role === r)).filter((p): p is RecommendedPick => !!p);
  const offered = shown && offers ? shown.filter((p) => isOffer(p) && p.fit === "fits" && (offers === "all" || !!p.replacesInstalled)) : shown;

  const get = useCallback(async (role: string) => {
    setErrors((e) => ({ ...e, [role]: null }));
    setStarting((s) => ({ ...s, [role]: true }));
    try {
      const { groupId } = await installRecommended(role);
      tagGroup(`rec:${role}`, groupId);
    } catch (e) {
      setErrors((x) => ({ ...x, [role]: asCoreError(e) }));
    } finally {
      setStarting((s) => ({ ...s, [role]: false }));
    }
  }, []);

  // Offers are extras next to something that works: no error box or skeleton for them.
  if (offers && (loadError || !offered?.length)) return null;

  if (loadError)
    return (
      <div className="space-y-2">
        <ErrorNotice error={loadError} />
        <Button size="sm" onClick={() => void load()}>
          <RotateCw className="h-3.5 w-3.5" /> Try again
        </Button>
      </div>
    );

  if (!offered)
    return (
      <div className="@container" aria-busy="true">
        <div className={compact ? "grid gap-3" : "grid gap-4 @xl:grid-cols-2"}>
          {(roles?.length ? roles : ["realistic", "anime", "edit", "describe"]).map((r) => (
            <div key={r} className="rounded-xl border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900">
              <Skeleton className="h-3 w-20" />
              <Skeleton className="mt-3 h-5 w-40" />
              <Skeleton className="mt-2 h-3 w-full" />
              {!compact && <Skeleton className="mt-6 h-8 w-full" />}
            </div>
          ))}
        </div>
      </div>
    );

  const getAllTargets = (picks ?? []).filter((p) => GET_ALL_ROLES.includes(p.role) && p.title && !p.installed && !p.unavailableReason);

  return (
    // Container queries: the same cards sit in wide pages and in narrow side panels.
    <div className={`@container ${compact ? "space-y-2" : "space-y-4"}`}>
      {heading}
      {showGetAll && (
        <GetAllBar picks={getAllTargets} onGet={(role) => void get(role)} starting={starting} allPicks={picks ?? []} />
      )}
      <div className={compact ? (offered.length > 1 ? "grid gap-3 @lg:grid-cols-2" : "grid max-w-xl gap-3") : "grid gap-4 @xl:grid-cols-2"}>
        {offered.map((p) => (
          <PickCard key={p.role} pick={p} compact={compact} machine={machinePlain(hw)} starting={!!starting[p.role]} error={errors[p.role] ?? null} onGet={() => void get(p.role)} onDismissError={() => setErrors((x) => ({ ...x, [p.role]: null }))} />
        ))}
      </div>
    </div>
  );
}

/** A pick the user can still get (not installed, and something is available). */
function isOffer(p: RecommendedPick): boolean {
  return !p.installed && !!p.title && !p.unavailableReason;
}

function useRoleGroup(pick: RecommendedPick): GroupStatus | null {
  // The describer may have been started from the Describe tab: match its group by kind
  // (its label is "Describe model", not the pick title).
  return useTaggedGroup(`rec:${pick.role}`, (g) => (pick.role === "describe" ? g.kind === "captioner" : !!pick.title && g.label === pick.title));
}

function GetAllBar({
  picks,
  allPicks,
  onGet,
  starting,
}: {
  picks: RecommendedPick[];
  allPicks: RecommendedPick[];
  onGet: (role: string) => void;
  starting: Record<string, boolean>;
}) {
  useDownloadsVersion();
  const busy = (role: string) => {
    const g = getTagged(`rec:${role}`);
    return !!starting[role] || (!!g && isActive(g));
  };
  const todo = picks.filter((p) => !busy(p.role));
  const total = todo.reduce((a, p) => a + p.downloadBytes, 0);
  const names = GET_ALL_ROLES.map((r) => allPicks.find((p) => p.role === r)?.roleLabel ?? r).join(" + ");
  const todoNames = todo.map((p) => p.roleLabel).join(" + ");
  const allDone = GET_ALL_ROLES.every((r) => allPicks.find((p) => p.role === r)?.installed);
  // Nothing to get (e.g. no GPU: no realistic/edit pick fits) — not "Downloading…".
  if (!picks.length && !allDone) return null;
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-amber-200 bg-amber-50/70 px-4 py-3 dark:border-amber-900/60 dark:bg-amber-500/5">
      <div className="text-sm">
        <div className="font-medium text-neutral-900 dark:text-neutral-100">Get the essentials in one go</div>
        <div className="text-neutral-600 dark:text-neutral-400">
          {allDone
            ? "Your realistic and edit models are ready."
            : todo.length
              ? `${todoNames} — about ${formatBytes(total)} in total. Shared parts are downloaded once.`
              : "Downloading… you can keep going; downloads continue in the background."}
        </div>
      </div>
      <Button variant="primary" disabled={!todo.length} onClick={() => todo.forEach((p) => onGet(p.role))}>
        {allDone ? <CircleCheck className="h-4 w-4" /> : <Download className="h-4 w-4" />}
        {allDone ? "All set" : `Get all (${names})`}
      </Button>
    </div>
  );
}

function PickCard({
  pick,
  compact,
  machine,
  starting,
  error,
  onGet,
  onDismissError,
}: {
  pick: RecommendedPick;
  compact: boolean;
  /** "your graphics card" | "your computer" */
  machine: string;
  starting: boolean;
  error: CoreError | null;
  onGet: () => void;
  onDismissError: () => void;
}) {
  const group = useRoleGroup(pick);
  const downloading = !!group && isActive(group);
  const failed = group?.state === "failed" && !pick.installed;
  const unavailable = !pick.title || !!pick.unavailableReason;
  const quant = quantPlain(pick.quant);
  const sizeText = pick.installed ? null : pick.downloadBytes > 0 ? `${formatBytes(pick.downloadBytes)} download` : "Nothing to download";

  const action = pick.installed ? (
    <span className="inline-flex items-center gap-1.5 text-sm font-medium text-emerald-700 dark:text-emerald-400">
      <CircleCheck className="h-4 w-4" /> Installed
    </span>
  ) : unavailable ? null : downloading ? null : (
    <Button variant="primary" size={compact ? "sm" : "md"} onClick={onGet} disabled={starting} aria-label={`Get ${pick.title}`}>
      {starting ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-4 w-4" />}
      {failed ? "Try again" : "Get"}
    </Button>
  );

  return (
    <article
      className={`flex flex-col rounded-xl border bg-white shadow-sm dark:bg-neutral-900 ${
        pick.installed ? "border-emerald-200 dark:border-emerald-900/60" : "border-neutral-200 dark:border-neutral-800"
      } ${compact ? "p-3.5" : "p-4"}`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center gap-1.5 text-xs font-semibold tracking-wide text-amber-600 uppercase dark:text-amber-400">
          {ROLE_ICON[pick.role] ?? null}
          {pick.roleLabel}
        </span>
        {compact && <span className="text-xs text-neutral-500">Best for {machine}</span>}
      </div>

      {unavailable ? (
        <div className="mt-2 text-sm text-neutral-600 dark:text-neutral-400">
          <div className="font-medium text-neutral-800 dark:text-neutral-200">{pick.title ?? "Nothing fits yet"}</div>
          <p className="mt-1">{pick.unavailableReason ?? `There's no model for this that fits ${machine} yet.`}</p>
        </div>
      ) : (
        <>
          <h3 className={`mt-1.5 font-semibold text-neutral-900 dark:text-neutral-50 ${compact ? "text-sm" : "text-base"}`}>{pick.title}</h3>
          {pick.goodAt && <p className={`mt-1 text-neutral-600 dark:text-neutral-400 ${compact ? "text-xs" : "text-sm"}`}>{pick.goodAt}</p>}
          {pick.note && !pick.installed && <p className="mt-1.5 text-xs text-amber-800 dark:text-amber-300">{pick.note}</p>}
          {!compact && (quant || pick.licenseNote) && (
            <p className="mt-2 text-xs text-neutral-500">
              {[quant, pick.licenseNote].filter(Boolean).join(" · ")}
            </p>
          )}
        </>
      )}

      <div className={`mt-auto ${compact ? "pt-3" : "pt-4"}`}>
        {!unavailable && (
          <div className={`flex flex-wrap items-center justify-between gap-x-3 gap-y-2 ${compact ? "" : "border-t border-neutral-100 pt-3 dark:border-neutral-800"}`}>
            <div className="flex min-w-0 flex-col gap-1">
              {sizeText && <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">{sizeText}</span>}
              <VramBadge vram={pick.vram} fit={pick.fit} />
            </div>
            {action}
          </div>
        )}
        {downloading && group && (
          <div className="mt-3">
            <GroupProgress group={group} compact={compact} onCancel={() => void cancelGroup(group.groupId)} />
          </div>
        )}
        {failed && group?.error && !error && <p className="mt-2 text-xs text-red-600 dark:text-red-400">{group.error}</p>}
        {error && (
          <div className="mt-3">
            <ErrorNotice error={error} onDismiss={onDismissError} />
          </div>
        )}
        {compact && pick.licenseNote && !unavailable && <p className="mt-2 text-[11px] text-neutral-500">{pick.licenseNote}</p>}
      </div>
    </article>
  );
}
