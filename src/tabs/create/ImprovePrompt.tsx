// "Improve my prompt": a short idea → a fuller prompt, written by the small helper model that
// also powers Describe (runs on this computer). The result replaces the box text; Undo puts it back.
// Used by Create's prompt box and Edit's "What should change?" box (a change instruction).
// PRIVACY: the prompt only travels to the local helper and back, through React state.
import { useEffect, useRef, useState } from "react";
import { Download, Undo2, WandSparkles } from "lucide-react";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { Button, Spinner, cx, focusRing } from "../../components/ui";
import * as api from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { useActions } from "../../lib/state/AppProvider";
import { useModel } from "../../lib/state/hooks";
import { isActiveDownload } from "../../lib/state/model";
import { activeLoras } from "../../lib/state/request";
import type { AppState } from "../../lib/state/model";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import type { CoreError, ImproveTarget } from "../../lib/types";
import { GroupProgress } from "../models/controls";

const toolbarButton = cx(
  "inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900 disabled:pointer-events-none disabled:opacity-50 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white",
  focusRing,
);

/** The box text Improve works on. */
const boxText = (s: AppState, target: ImproveTarget) => (target === "edit" ? s.edit.instruction : s.create.prompt);

/** The Improve / Undo button for the prompt box toolbar, and the notice that goes under the box.
 *  `target` = which box: Create's prompt or Edit's change instruction (`modelId` = the edit model). */
export function useImprovePrompt(familyId: string | null | undefined, target: ImproveTarget = "create", editModelId: string | null = null) {
  const prompt = useAppState((s) => boxText(s, target));
  const modelId = useAppState((s) => (target === "edit" ? editModelId : s.create.modelId));
  const model = useModel(modelId);
  const dispatch = useDispatch();
  const setText = (text: string) => dispatch(target === "edit" ? { type: "patchEdit", patch: { instruction: text } } : { type: "patchCreate", patch: { prompt: text } });
  const store = useStore();
  const actions = useActions();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  // The helper's answer was unusable: the prompt stays as it was.
  const [note, setNote] = useState<string | null>(null);
  // The helper model isn't installed: offer it (bytes to download).
  const [needHelper, setNeedHelper] = useState<number | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installGroup, setInstallGroup] = useState<string | null>(null);
  // What the box held before the last improve, while it still holds the improved text.
  const [undo, setUndo] = useState<{ before: string; after: string } | null>(null);
  const runId = useRef(0);
  const waitingForHelper = useRef(false);

  const dl = useAppState(
    (s) => (installGroup ? s.downloads.find((x) => x.groupId === installGroup) : undefined) ?? [...s.downloads].reverse().find((x) => x.kind === "captioner" && isActiveDownload(x)),
  );

  const run = async () => {
    const s = store.getState();
    const base = boxText(s, target);
    if (!base.trim() || busy) return;
    const id = ++runId.current;
    const nonce = s.sessionNonce;
    setError(null);
    setNote(null);
    setBusy(true);
    try {
      const status = await api.captionerStatus("improve").catch(() => null);
      if (status && !status.available) {
        setNeedHelper(status.downloadBytes);
        return;
      }
      setNeedHelper(null);
      const loras = store.getState().loras;
      const params = target === "edit" ? store.getState().edit : store.getState().create;
      const avoid = activeLoras(params, loras, model, s.settings?.addTriggerWords ?? true).flatMap((u) => u.words ?? []);
      const answer = await api.improvePrompt(base, familyId ?? null, avoid, target);
      const text = answer.text.trim();
      const now = store.getState();
      // Dropped when Reset was pressed or the prompt was edited meanwhile: the answer no longer fits.
      if (id === runId.current && now.sessionNonce === nonce && boxText(now, target) === base && text) {
        if (answer.note) {
          setNote(answer.note);
          return;
        }
        setText(text);
        setUndo({ before: base, after: text });
      }
    } catch (e) {
      const err = api.asCoreError(e);
      if (id === runId.current && err.code !== "cancelled") setError(err);
    } finally {
      if (id === runId.current) setBusy(false);
    }
  };

  const install = async () => {
    setInstalling(true);
    setError(null);
    try {
      const started = await api.installCaptioner();
      setInstallGroup(started.groupId);
      waitingForHelper.current = true;
      void actions.refreshDownloads().catch(() => undefined);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setInstalling(false);
    }
  };

  // The helper finished downloading: improve right away.
  useEffect(() => {
    if (dl?.state === "done" && waitingForHelper.current) {
      waitingForHelper.current = false;
      setNeedHelper(null);
      setInstallGroup(null);
      void run();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dl?.state]);

  const undoable = undo && prompt === undo.after ? undo : null;
  const empty = !prompt.trim();

  const button = undoable ? (
    <button
      type="button"
      className={toolbarButton}
      onClick={() => {
        setText(undoable.before);
        setUndo(null);
      }}
      title="Put back what you wrote"
    >
      <Undo2 className="h-3.5 w-3.5" /> Undo
    </button>
  ) : (
    <button
      type="button"
      className={toolbarButton}
      disabled={empty || busy}
      onClick={() => void run()}
      title={
        target === "edit"
          ? empty
            ? "Type the change first, then Improve adds how it should look and what to keep"
            : "Add how the change should look and what to keep. Runs on this computer."
          : empty
            ? "Type a few words first, then Improve turns them into a fuller prompt"
            : "Turn your idea into a fuller prompt. Runs on this computer."
      }
    >
      {busy ? <Spinner className="h-3.5 w-3.5 text-amber-500" /> : <WandSparkles className="h-3.5 w-3.5" />} {busy ? "Improving…" : "Improve"}
    </button>
  );

  const installingNow = !!dl && isActiveDownload(dl);
  const failedNow = dl?.state === "failed";
  const notice =
    needHelper !== null || error || note ? (
      <div className="space-y-2">
        {needHelper !== null && (
          <div className="pinhole-pop space-y-2 rounded-xl border border-neutral-200 px-3 py-2.5 text-sm dark:border-neutral-800" role="status">
            <p>
              Improve uses a small helper model that runs on this computer.
              {needHelper > 0 ? ` It is a one-time ${formatBytes(needHelper)} download.` : ""}
            </p>
            {installingNow && dl ? (
              <GroupProgress group={dl} compact onCancel={() => void api.cancelDownload(dl.groupId).finally(() => void actions.refreshDownloads().catch(() => undefined))} />
            ) : (
              <div className="flex flex-wrap gap-2">
                {failedNow && (
                  <p className="w-full text-xs text-red-600 dark:text-red-400" role="alert">
                    {dl.error ?? "The download failed."} Try again.
                  </p>
                )}
                <Button size="sm" variant="primary" disabled={installing} onClick={() => void install()}>
                  {installing ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />} {failedNow ? "Try again" : "Get the helper"}
                  {needHelper ? ` (${formatBytes(needHelper)})` : ""}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setNeedHelper(null)}>
                  Not now
                </Button>
              </div>
            )}
          </div>
        )}
        {note && (
          <p className="px-1 text-xs text-neutral-500" role="status">
            {note}
          </p>
        )}
        {error && <ErrorWithFix error={error} onDismiss={() => setError(null)} onRetry={() => void run()} />}
      </div>
    ) : busy ? (
      <p className="px-1 text-xs text-neutral-500" role="status">
        Writing… the first time takes longer while the helper starts.
      </p>
    ) : null;

  return { button, notice };
}
