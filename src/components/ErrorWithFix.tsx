// ErrorNotice plus a one-click fix where there is one:
//   engine_missing → "Set up engine" (installEngine, progress in Downloads) → "Try again".
//   not_found for a model, model_load (the model file couldn't be loaded) → "Open Models".
import { useState } from "react";
import { Download, Layers, RotateCcw } from "lucide-react";
import * as api from "../lib/api";
import type { CoreError } from "../lib/types";
import { useActions } from "../lib/state/AppProvider";
import { Button, ErrorNotice, Spinner } from "./ui";

export function ErrorWithFix({ error, onDismiss, onRetry }: { error: CoreError; onDismiss?: () => void; onRetry?: () => void }) {
  const actions = useActions();
  const [state, setState] = useState<"idle" | "installing" | "ready">("idle");
  const [installError, setInstallError] = useState<CoreError | null>(null);

  let action = null;
  if (error.code === "engine_missing") {
    action =
      state === "ready" ? (
        onRetry ? (
          <Button size="sm" variant="primary" onClick={onRetry}>
            <RotateCcw className="h-3.5 w-3.5" /> Try again
          </Button>
        ) : (
          <span className="text-xs font-medium">Engine ready — try again.</span>
        )
      ) : (
        <Button
          size="sm"
          variant="primary"
          disabled={state === "installing"}
          onClick={async () => {
            setState("installing");
            setInstallError(null);
            void actions.refreshDownloads().catch(() => undefined);
            try {
              actions.onEngine(await api.installEngine());
              setState("ready");
            } catch (e) {
              setInstallError(api.asCoreError(e));
              setState("idle");
            }
          }}
        >
          {state === "installing" ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />}
          {state === "installing" ? "Setting up… (see Downloads)" : "Set up engine"}
        </Button>
      );
  } else if (error.code === "model_load" || (error.code === "not_found" && /model/i.test(error.message))) {
    action = (
      <Button size="sm" onClick={() => actions.setTab("models")}>
        <Layers className="h-3.5 w-3.5" /> Open Models
      </Button>
    );
  }

  return (
    <div className="space-y-2">
      <ErrorNotice error={error} onDismiss={onDismiss} action={action} />
      {installError && <ErrorNotice error={installError} onDismiss={() => setInstallError(null)} />}
    </div>
  );
}
