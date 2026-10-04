// Models tab: Browse (CivitAI) · Installed, plus the downloads list.
// Keep this export signature.
import { useEffect, useRef, useState } from "react";
import { getSettings } from "../../lib/api";
import { useAppState } from "../../lib/state/store";
import type { Settings } from "../../lib/types";
import { Segmented, Spinner } from "../../components/ui";
import { onSettingsChanged } from "../../settings/events";
import { BrowseView } from "./BrowseView";
import { DownloadsPanel } from "./DownloadsPanel";
import { InstalledView } from "./InstalledView";
import { ScrollRootContext, useIsVisible } from "./lib/preview";
import { getLastView, hasAddonRequest, onAddonRequest, onHelpersRequest, onInstalledRequest, rememberView } from "./lib/session";
import { HelpersView } from "./HelpersView";

type View = "browse" | "helpers" | "installed";

export function ModelsTab() {
  const [view, setViewState] = useState<View>(() => (hasAddonRequest() ? "browse" : (getLastView() ?? "browse")));
  const [settings, setLocalSettings] = useState<Settings | null>(null);
  const [settingsReady, setSettingsReady] = useState(false);
  // The app store already holds the installed models and add-ons; null until the first load.
  const installedCount = useAppState((s) => (s.models == null ? null : s.models.length + s.loras.length));
  // The shell keeps every tab mounted. Don't contact CivitAI (or pop the Safe mode question)
  // until the Models tab has actually been opened.
  const rootRef = useRef<HTMLDivElement>(null);
  const visible = useIsVisible(rootRef);
  const [opened, setOpened] = useState(false);
  // The element that scrolls the cards: previews and paging observe relative to it.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  useEffect(() => setScroller(rootRef.current), []);
  useEffect(() => {
    if (visible) setOpened(true);
  }, [visible]);

  const setView = (v: View) => {
    rememberView(v);
    setViewState(v);
  };

  // "Find style add-ons" from Create or Installed opens Browse.
  useEffect(
    () =>
      onAddonRequest(() => {
        rememberView("browse");
        setViewState("browse");
      }),
    [],
  );

  // "Get more models…" in a Describe / Improve picker opens Helpers.
  useEffect(
    () =>
      onHelpersRequest(() => {
        rememberView("helpers");
        setViewState("helpers");
      }),
    [],
  );

  // "Get missing parts" in Create opens Installed.
  useEffect(
    () =>
      onInstalledRequest(() => {
        rememberView("installed");
        setViewState("installed");
      }),
    [],
  );

  useEffect(() => {
    getSettings()
      .then(setLocalSettings)
      .catch(() => undefined)
      .finally(() => setSettingsReady(true));
    return onSettingsChanged(setLocalSettings);
  }, []);

  return (
    <div ref={rootRef} className="h-full overflow-y-auto">
      <ScrollRootContext.Provider value={scroller}>
        <div className="mx-auto max-w-[1600px] space-y-4 px-6 py-5">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h1 className="text-xl font-semibold tracking-tight">Models</h1>
              <p className="text-sm text-neutral-500">
                {view === "browse"
                  ? "Find models and style add-ons on CivitAI. Everything downloads to this computer."
                  : view === "helpers"
                    ? "Small language models for Describe and Improve. They run on this computer."
                    : "Everything on this computer, ready to use offline."}
              </p>
            </div>
            <Segmented
              ariaLabel="View"
              options={[
                { value: "browse" as View, label: "Browse" },
                { value: "helpers" as View, label: "Helpers" },
                { value: "installed" as View, label: installedCount != null ? `Installed (${installedCount})` : "Installed" },
              ]}
              value={view}
              onChange={setView}
            />
          </div>

          <DownloadsPanel />

          {view === "browse" ? (
            settingsReady && opened ? (
              <BrowseView settings={settings} onShowInstalled={() => setView("installed")} />
            ) : (
              <div className="flex justify-center py-16">
                <Spinner className="h-5 w-5 text-neutral-400" />
              </div>
            )
          ) : view === "helpers" ? (
            <HelpersView />
          ) : (
            <InstalledView onBrowse={() => setView("browse")} />
          )}
        </div>
      </ScrollRootContext.Provider>
    </div>
  );
}
