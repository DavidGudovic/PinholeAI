// Wires the store to the backend: initial loads, events, image cleanup.
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import * as api from "../api";
import { makeActions, type Actions } from "./actions";
import { releaseRefs } from "./images";
import type { TabId } from "./model";
import { StoreContext, createStore, type Store } from "./store";

const ActionsContext = createContext<Actions | null>(null);

export function useActions(): Actions {
  const a = useContext(ActionsContext);
  if (!a) throw new Error("ActionsContext missing");
  return a;
}

export function AppProvider({ children, store: given }: { children: ReactNode; store?: Store }) {
  const [store] = useState<Store>(
    () => given ?? createStore((refs, action) => releaseRefs(refs, action.type !== "clearSession")),
  );
  const [actions] = useState(() => makeActions(store));

  useEffect(() => {
    let alive = true;
    const unlisten: (() => void)[] = [];
    const keep = (p: Promise<() => void>) =>
      p.then((u) => (alive ? unlisten.push(u) : u())).catch(() => undefined);

    keep(api.onDownload((st) => store.dispatch({ type: "download", status: st })));
    keep(
      api.onGeneration((p) => {
        if (store.getState().job) store.dispatch({ type: "jobProgress", progress: p });
      }),
    );
    keep(api.onEngine((e) => store.dispatch({ type: "setEngine", engine: e })));
    keep(api.onModelsChanged(() => void actions.refreshModels().catch(() => undefined)));

    void actions.refreshSettings().catch(() =>
      // No settings → don't trap the user in a broken first run.
      store.dispatch({
        type: "setSettings",
        settings: {
          offline: false,
          gpu: "auto",
          vramOverrideGb: null,
          contentMode: "safe",
          showPaid: false,
          hideAnime: false,
          savedMetadata: "none",
          theme: "system",
          addTriggerWords: true,
          soundOnDone: false,
          firstRunDone: true,
          engineBackend: "auto",
          textEncoderOnCpu: "auto",
          modelsFolder: null,
        describeModel: "auto",
        improveModel: "auto",
        },
      }),
    );
    void actions.refreshModels().catch(() => store.dispatch({ type: "setModels", models: [] }));
    void actions.refreshStyles().catch(() => undefined);
    void actions.refreshPresets().catch(() => undefined);
    void actions.refreshEngine().catch(() => undefined);
    void actions.refreshDownloads().catch(() => undefined);

    return () => {
      alive = false;
      for (const u of unlisten) u();
    };
  }, [store, actions]);

  return (
    <StoreContext.Provider value={store}>
      <ActionsContext.Provider value={actions}>{children}</ActionsContext.Provider>
    </StoreContext.Provider>
  );
}

// ---------------------------------------------------------------- Ctrl/Cmd+Enter

const primary = new Map<TabId, () => void>();

/** Register the tab's primary action (Generate / Edit / Describe) for Ctrl/Cmd+Enter. */
export function usePrimaryAction(tab: TabId, fn: () => void) {
  const ref = useRef(fn);
  useEffect(() => {
    ref.current = fn;
  });
  useEffect(() => {
    const run = () => ref.current();
    primary.set(tab, run);
    return () => {
      if (primary.get(tab) === run) primary.delete(tab);
    };
  }, [tab]);
}

export function runPrimaryAction(tab: TabId): boolean {
  const f = primary.get(tab);
  if (!f) return false;
  f();
  return true;
}
