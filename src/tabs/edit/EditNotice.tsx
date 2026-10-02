// Edit's one-time notice about editing photos of real people.
import { useState } from "react";
import { Button } from "../../components/ui";
import * as api from "../../lib/api";
import { useAppState, useDispatch } from "../../lib/state/store";

export function EditNotice({ rootId }: { rootId: string | undefined }) {
  const dispatch = useDispatch();
  // One-time notice the first time a picture from the computer is edited (RELEASE-SPEC §7).
  const rootImported = useAppState(
    (s) =>
      !!rootId &&
      s.results.find((r) => r.id === rootId)?.origin !== "generated",
  );
  const editNoticeSeen = useAppState(
    (s) => s.settings?.editNoticeSeen ?? true,
  );
  const [editNoticeClosed, setEditNoticeClosed] = useState(false);
  const showEditNotice = rootImported && !editNoticeSeen && !editNoticeClosed;
  const closeEditNotice = () => {
    setEditNoticeClosed(true);
    void api
      .getSettings()
      .then((s) => api.setSettings({ ...s, editNoticeSeen: true }))
      .then((s) => dispatch({ type: "setSettings", settings: s }))
      .catch(() => undefined);
  };
  if (!showEditNotice) return null;
  return (
    <div
      role="note"
      className="flex items-start gap-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2.5 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-200"
    >
      <p className="flex-1">
        Only edit photos of people who have agreed to it. Making sexual
        or humiliating images of real people without their consent is a
        crime in many countries.
      </p>
      <Button size="sm" variant="ghost" onClick={closeEditNotice}>
        OK
      </Button>
    </div>
  );
}
