// The Pinhole Licence, shown in the app from the LICENSE file bundled at build time (no network).
import licence from "../../LICENSE?raw";
import { Button, Dialog } from "./ui";

export function LicenceText({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} onClose={onClose} title="Pinhole Licence" footer={<Button onClick={onClose}>Close</Button>}>
      <pre className="text-xs leading-relaxed whitespace-pre-wrap text-neutral-700 dark:text-neutral-300">{licence.trim()}</pre>
    </Dialog>
  );
}
