import { useState } from "react";
import { Save } from "lucide-react";
import * as api from "../lib/api";
import { canSaveAs } from "../lib/state/platform";
import { useActions } from "../lib/state/AppProvider";
import { unsavedEditIds, unsavedIds } from "../lib/state/model";
import { useAppState, useDispatch } from "../lib/state/store";
import type { CoreError } from "../lib/types";
import { Button, Dialog, ErrorNotice } from "./ui";

/** Shown before closing the window, Reset, or replacing Edit's history while some pictures were never saved. */
export function UnsavedDialog() {
  const what = useAppState((s) => s.leave);
  // Loading another image into Edit only drops the edit results.
  const count = useAppState((s) => (s.leave === "edit" ? unsavedEditIds(s) : unsavedIds(s)).length);
  const actions = useActions();
  const dispatch = useDispatch();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const closing = what === "close";
  const replacing = what === "edit";
  const cancel = () => {
    setError(null);
    dispatch({ type: "askLeave", what: null });
  };

  const saveAllThenLeave = async () => {
    setBusy(true);
    setError(null);
    try {
      // Cancelling the folder picker (or a failed save) keeps the dialog open.
      if (await actions.saveAll()) await actions.finishLeave(what!);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={what !== null}
      onClose={() => !busy && cancel()}
      title={count === 1 ? "You have 1 picture that isn't saved" : `You have ${count} pictures that aren't saved`}
      description={`Nothing is saved until you press Save. ${closing ? "Closing Pinhole" : replacing ? "Editing another image" : "Reset"} removes them for good.`}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={cancel}>
            Go back
          </Button>
          <Button variant="secondary" disabled={busy} onClick={() => void actions.finishLeave(what!)}>
            {closing ? "Close without saving" : replacing ? "Continue without saving" : "Reset without saving"}
          </Button>
          {canSaveAs() && (
            <Button variant="primary" disabled={busy} onClick={() => void saveAllThenLeave()}>
              <Save className="h-4 w-4" /> Save all…
            </Button>
          )}
        </>
      }
    >
      <p className="text-sm text-neutral-600 dark:text-neutral-400">
        {canSaveAs() ? "Save all puts every unsaved picture into a folder you choose." : `Save pictures one by one with the Save button first, or ${replacing ? "continue" : "leave"} without saving.`}
      </p>
      {error && (
        <div className="mt-3">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </Dialog>
  );
}
