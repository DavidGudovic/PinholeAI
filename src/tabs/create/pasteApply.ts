// "Paste from CivitAI": parse → resolve resources (ids/hashes only) → apply to Create.
// PRIVACY: the pasted text and parsed prompt stay in memory; only resource
// ids/hashes/names go to Rust (`resolve_civitai_resources`). Nothing is logged.
import * as api from "../../lib/api";
import { planPaste, type PasteSkip } from "../../lib/paste/map";
import { parseGenerationData, type ParsedGeneration } from "../../lib/paste/parse";
import type { CoreError, LoraUse, ResolvedResource, ResolvedResources } from "../../lib/types";
import type { Actions } from "../../lib/state/actions";
import { createModels, loraCompatible } from "../../lib/state/model";
import type { Store } from "../../lib/state/store";

export interface PasteOutcome {
  /** Kept in memory so settings can be re-applied when a model is installed later. prompt-bearing. */
  parsed: ParsedGeneration;
  applied: string[];
  skipped: PasteSkip[];
  ignoredKeys: string[];
  resolved: ResolvedResources | null;
  resolveError: CoreError | null;
  /** Friendly name of the model the settings were applied for (null = none installed). */
  modelName: string | null;
  /** The pasted checkpoint was found installed and selected. */
  checkpointSelected: boolean;
  /** Installed LoRAs that were added (resource index in resolved.loras). */
  lorasAdded: string[];
  /** Installed LoRAs skipped because they don't fit the model. */
  lorasIncompatible: string[];
}

/** Apply pasted generation data to the Create tab. Throws CoreError when nothing usable was found. */
export async function applyPastedText(text: string, store: Store, actions: Actions): Promise<PasteOutcome> {
  const parsed = parseGenerationData(text);
  if (!parsed || (!parsed.prompt && !parsed.hasSettings)) {
    throw { code: "invalid", message: "That doesn't look like generation data. On a CivitAI image page, click “Copy generation data” and paste again.", details: null } as CoreError;
  }

  let resolved: ResolvedResources | null = null;
  let resolveError: CoreError | null = null;
  if (parsed.resources.length) {
    try {
      resolved = await api.resolveCivitaiResources(parsed.resources);
    } catch (e) {
      resolveError = api.asCoreError(e);
    }
  }
  return applyParsed(parsed, resolved, resolveError, store, actions, { setPrompt: true });
}

/** Apply (or re-apply after installing something) parsed data. */
export async function applyParsed(
  parsed: ParsedGeneration,
  resolved: ResolvedResources | null,
  resolveError: CoreError | null,
  store: Store,
  actions: Actions,
  opts: { setPrompt: boolean; preferModelId?: string | null },
): Promise<PasteOutcome> {
  const s = store.getState();
  const usable = createModels(s.models);
  let modelId = s.create.modelId;
  let checkpointSelected = false;
  const ck = resolved?.checkpoint ?? null;
  const preferred = opts.preferModelId ?? ck?.installedId ?? null;
  if (preferred && usable.some((m) => m.id === preferred)) {
    modelId = preferred;
    checkpointSelected = preferred === ck?.installedId;
  }
  const model = usable.find((m) => m.id === modelId) ?? null;
  const ui = model?.familyId ? await actions.ensureFamilyUi(model.familyId).catch(() => null) : null;
  const plan = planPaste(parsed, ui);

  const loras: LoraUse[] = [];
  const lorasAdded: string[] = [];
  const lorasIncompatible: string[] = [];
  for (const r of resolved?.loras ?? []) {
    if (!r.installedId) continue;
    const l = s.loras.find((x) => x.id === r.installedId);
    if (l && !loraCompatible(l, model?.familyId)) {
      lorasIncompatible.push(r.displayName);
      continue;
    }
    if (!loras.some((u) => u.loraId === r.installedId)) {
      loras.push({ loraId: r.installedId, weight: r.resource.weight ?? 1 });
      lorasAdded.push(r.displayName);
    }
  }

  store.dispatch({
    type: "patchCreate",
    patch: {
      // Settings pasted without a prompt keep the one already typed.
      ...(opts.setPrompt && plan.prompt.trim() ? { prompt: plan.prompt } : {}),
      modelId,
      presetId: null,
      fineTune: plan.fineTune,
      stick: plan.stick,
      ...(plan.shape ? { shape: plan.shape } : {}),
      loras,
    },
  });

  return {
    parsed,
    applied: plan.applied,
    skipped: plan.skipped,
    ignoredKeys: plan.ignoredKeys,
    resolved,
    resolveError,
    modelName: model?.friendlyName ?? null,
    checkpointSelected,
    lorasAdded,
    lorasIncompatible,
  };
}

/** Stable key for a resolved resource (for download tracking in the summary). */
export const resourceKey = (r: ResolvedResource) =>
  `${r.resource.type}:${r.resource.modelVersionId ?? r.resource.hash ?? r.resource.modelName ?? r.displayName}`;
