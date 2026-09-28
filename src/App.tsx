// App shell: top bar, tabs (kept mounted so their state survives switching),
// Settings sheet, first run, theme, Ctrl/Cmd+Enter.
import { useEffect, useState } from "react";
import { Logo } from "./components/Logo";
import { Toasts } from "./components/Toasts";
import { TopBar } from "./components/TopBar";
import { Spinner } from "./components/ui";
import { FirstRun } from "./firstrun/FirstRun";
import type { Settings } from "./lib/types";
import { AppProvider, runPrimaryAction, useActions } from "./lib/state/AppProvider";
import type { TabId } from "./lib/state/model";
import { useAppState, useDispatch } from "./lib/state/store";
import { SettingsSheet } from "./settings/SettingsSheet";
import { onSettingsChanged } from "./settings/events";
import { CreateTab } from "./tabs/create/CreateTab";
import { DescribeTab } from "./tabs/describe/DescribeTab";
import { EditTab } from "./tabs/edit/EditTab";
import { ModelsTab } from "./tabs/models/ModelsTab";

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}

function useTheme(theme: Settings["theme"] | undefined) {
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || ((theme ?? "system") === "system" && mq.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    if ((theme ?? "system") !== "system") return;
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme]);
}

function Shell() {
  const settings = useAppState((s) => s.settings);
  const tab = useAppState((s) => s.tab);
  const nonce = useAppState((s) => s.sessionNonce);
  const dispatch = useDispatch();
  const actions = useActions();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [firstRunClosed, setFirstRunClosed] = useState(false);
  useTheme(settings?.theme);

  // Settings saved anywhere in the UI (sheet, first run) → apply immediately (theme, trigger words…).
  useEffect(() => onSettingsChanged((s) => dispatch({ type: "setSettings", settings: s })), [dispatch]);

  // Ctrl/Cmd+Enter → the current tab's main action (Generate / Edit / Describe).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
        if (document.querySelector('[role="dialog"]')) return;
        if (runPrimaryAction(tab)) e.preventDefault();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab]);

  if (!settings) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 text-neutral-500">
        <Logo className="h-12 w-12" />
        <Spinner />
      </div>
    );
  }

  if (!settings.firstRunDone && !firstRunClosed) {
    return (
      <div className="h-full overflow-auto">
        <FirstRun
          onDone={() => {
            setFirstRunClosed(true);
            void actions.refreshSettings().catch(() => undefined);
            void actions.refreshModels().catch(() => undefined);
            void actions.refreshEngine().catch(() => undefined);
          }}
        />
        <Toasts />
      </div>
    );
  }

  const pane = (id: TabId) => ({ id: `tab-${id}`, role: "tabpanel", hidden: tab !== id, className: "h-full" }) as const;

  return (
    <div className="flex h-full min-w-[900px] flex-col">
      <TopBar onOpenSettings={() => setSettingsOpen(true)} />
      <main className="relative min-h-0 flex-1">
        <div {...pane("create")}>
          <CreateTab key={`create-${nonce}`} />
        </div>
        <div {...pane("edit")}>
          <EditTab key={`edit-${nonce}`} />
        </div>
        <div {...pane("describe")}>
          <DescribeTab key={`describe-${nonce}`} />
        </div>
        <div {...pane("models")}>
          <ModelsTab />
        </div>
      </main>
      <SettingsSheet
        open={settingsOpen}
        onClose={() => {
          setSettingsOpen(false);
          void actions.refreshSettings().catch(() => undefined);
        }}
      />
      <Toasts />
    </div>
  );
}
