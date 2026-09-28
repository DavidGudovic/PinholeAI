// Tiny external store (useSyncExternalStore) so components subscribe to slices
// and typing in the prompt box doesn't re-render the image grid.
import { createContext, useContext, useSyncExternalStore } from "react";
import { initialState, reducer, type Action, type AppState, type ImgRef } from "./model";

export interface Store {
  getState(): AppState;
  dispatch(a: Action): void;
  subscribe(fn: () => void): () => void;
}

/**
 * `onDropped` is called with image refs that nothing references any more
 * (after the state update), so their blob: URLs can be revoked and the Rust
 * session copy discarded.
 */
export function createStore(onDropped?: (refs: ImgRef[], action: Action) => void, init: AppState = initialState()): Store {
  let state = init;
  const subs = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(a) {
      const prev = state;
      const next = reducer(prev, a);
      if (next === prev) return;
      state = next;
      for (const f of [...subs]) f();
      if (onDropped && prev.images !== next.images) {
        const dropped = Object.values(prev.images).filter((r) => !next.images[r.id]);
        if (dropped.length) onDropped(dropped, a);
      }
    },
    subscribe(fn) {
      subs.add(fn);
      return () => {
        subs.delete(fn);
      };
    },
  };
}

export const StoreContext = createContext<Store | null>(null);

export function useStore(): Store {
  const s = useContext(StoreContext);
  if (!s) throw new Error("StoreContext missing");
  return s;
}

/** Subscribe to a slice. The selector must return a stable value (a slice or a primitive). */
export function useAppState<T>(selector: (s: AppState) => T): T {
  const store = useStore();
  return useSyncExternalStore(store.subscribe, () => selector(store.getState()));
}

export function useDispatch(): (a: Action) => void {
  return useStore().dispatch;
}
