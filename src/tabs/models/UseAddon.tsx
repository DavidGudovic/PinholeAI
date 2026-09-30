// "Use" for an installed style add-on: adds it to Create (SPEC §5.4) and switches there.
import { Plus } from "lucide-react";
import { Button } from "../../components/ui";
import { useActions } from "../../lib/state/AppProvider";
import { useAppState } from "../../lib/state/store";

/** The installed add-on for a CivitAI version, if Pinhole has it. */
export function useInstalledLoraId(versionId: number | null): string | null {
  return useAppState((s) => (versionId == null ? null : (s.loras.find((l) => l.civitaiVersionId === versionId)?.id ?? null)));
}

export function UseAddonButton({ loraId, name, size = "sm", className }: { loraId: string; name: string; size?: "sm" | "md"; className?: string }) {
  const actions = useActions();
  const inUse = useAppState((s) => s.create.loras.some((u) => u.loraId === loraId));
  return (
    <Button size={size} className={className} onClick={() => actions.addLora(loraId)} aria-label={`Use ${name} in Create`} title={inUse ? "Already added in Create" : "Add to Create"}>
      <Plus className="h-3.5 w-3.5" /> {inUse ? "In use" : "Use"}
    </Button>
  );
}
