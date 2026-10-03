// One-time notice about photos of real people, shown in Edit and in Create's reference slot.
import { useState } from "react";
import { Button } from "./ui";
import * as api from "../lib/api";
import { useAppState, useDispatch } from "../lib/state/store";

/** "edit" in Edit, "use" for Create's reference picture. */
export function PhotoNotice({ imageId, verb }: { imageId: string | undefined; verb: "edit" | "use" }) {
  const dispatch = useDispatch();
  // One-time notice the first time a picture from the computer is used (RELEASE-SPEC §7).
  const imported = useAppState(
    (s) =>
      !!imageId &&
      s.results.find((r) => r.id === imageId)?.origin !== "generated",
  );
  const noticeSeen = useAppState(
    (s) => s.settings?.editNoticeSeen ?? true,
  );
  const [closed, setClosed] = useState(false);
  const show = imported && !noticeSeen && !closed;
  const close = () => {
    setClosed(true);
    void api
      .getSettings()
      .then((s) => api.setSettings({ ...s, editNoticeSeen: true }))
      .then((s) => dispatch({ type: "setSettings", settings: s }))
      .catch(() => undefined);
  };
  if (!show) return null;
  return (
    <div
      role="note"
      className="flex items-start gap-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2.5 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-200"
    >
      <p className="flex-1">
        Only {verb} photos of people who have agreed to it. Making sexual
        or humiliating images of real people without their consent is a
        crime in many countries.
      </p>
      <Button size="sm" variant="ghost" onClick={close}>
        OK
      </Button>
    </div>
  );
}
