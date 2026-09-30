// The one-time "I accept" for a model licence (RELEASE-SPEC §6), opened by the install wrappers
// in lib/api.ts. Only the licence id is stored once accepted.
import { useEffect, useState } from "react";
import { FileText } from "lucide-react";
import { currentLicenceRequest, onLicenceRequest, type LicenceRequest } from "../lib/licence";
import { Button, Dialog } from "./ui";

export function LicencePrompt() {
  const [req, setReq] = useState<LicenceRequest | null>(currentLicenceRequest);
  useEffect(() => onLicenceRequest(setReq), []);
  if (!req) return null;
  return (
    <Dialog
      open
      onClose={() => req.resolve(false)}
      title={
        <span className="flex items-center gap-2">
          <FileText className="h-4 w-4" /> Model licence
        </span>
      }
      footer={
        <>
          <Button variant="ghost" onClick={() => req.resolve(false)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={() => req.resolve(true)}>
            I accept
          </Button>
        </>
      }
    >
      <p className="text-sm">{req.message}</p>
      <p className="mt-2 text-sm text-neutral-600 dark:text-neutral-400">
        The people who made this model set its terms, such as no commercial use.
        Accept the licence to download it; Pinhole asks once per licence.
      </p>
    </Dialog>
  );
}
