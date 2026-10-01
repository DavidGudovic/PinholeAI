// Dev builds only (`npm run tauri dev`): every image check score for the shown picture, measured
// on request, so each half of a rule can be tried on ordinary, legal pictures (docs/RELEASE-SPEC.md
// §3.4). Release builds leave this out, and their Rust side answers null.
import { useEffect, useState } from "react";
import * as api from "../lib/api";

export function CheckReadings({ id }: { id: string }) {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setText(null);
    api
      .checkReadings(id)
      .then((t) => live && setText(t))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [id]);
  if (!text) return null;
  return (
    <p className="mx-auto max-w-2xl text-center font-mono text-[10px] break-words text-neutral-400" title="Dev builds only">
      Safety check readings: {text}
    </p>
  );
}
