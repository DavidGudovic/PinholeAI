// The full usage guidelines (RELEASE-SPEC §7), linked from the first-launch notice and shown
// again when the check stops something (<BlockedNotice />, with the block message on top).
// Shown in the app, so reading them needs no network.
import type { ReactNode } from "react";
import { Button, Dialog } from "./ui";

export function UsageGuidelines({ open, onClose, notice }: { open: boolean; onClose: () => void; notice?: ReactNode }) {
  return (
    <Dialog open={open} onClose={onClose} title="Usage guidelines" footer={<Button onClick={onClose}>Close</Button>}>
      <div className="space-y-4 text-sm text-neutral-700 dark:text-neutral-300">
        {notice && (
          <div
            role="status"
            className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2.5 text-amber-900 dark:border-amber-800/60 dark:bg-amber-950/40 dark:text-amber-200"
          >
            {notice}
          </div>
        )}
        <p>
          Pinhole is for creative work: art, illustration, design, concepts and photo edits. These guidelines apply to
          everything you make with it.
        </p>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">Not allowed</h3>
          <p className="mt-1.5">Don't use Pinhole to create or share content that:</p>
          <ul className="mt-1.5 list-disc space-y-1 pl-5">
            <li>sexualizes minors, or anyone who appears to be under 18</li>
            <li>shows a real person in a sexual or intimate way without their consent, including edits of their photos</li>
            <li>impersonates, deceives, bullies or harasses real people</li>
            <li>forges documents, IDs, receipts or evidence</li>
            <li>is otherwise illegal</li>
          </ul>
        </section>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">Sharing</h3>
          <p className="mt-1.5">
            Saved pictures are marked as made with AI, in the file details and with an invisible watermark. Don't
            present a made or edited picture as a real photo in a way that could mislead people.
          </p>
        </section>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">Model licences</h3>
          <p className="mt-1.5">
            Each model has its own licence, set by the people who made it. Pinhole shows it before the download; some
            ask you to accept it first.
          </p>
        </section>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">The safety check</h3>
          <p className="mt-1.5">
            Pinhole checks prompts and pictures on your computer against these guidelines, for example content that
            sexualizes minors or intimate edits of photos of real people. It also keeps models marked for safe images
            only to safe images. The check works offline and can't be turned off. Like any automatic
            check, it can sometimes stop something harmless.
          </p>
        </section>
      </div>
    </Dialog>
  );
}
