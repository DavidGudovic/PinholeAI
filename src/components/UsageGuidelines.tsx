// The full usage guidelines (RELEASE-SPEC §7), linked from the first-launch notice and Settings.
// Shown in the app, so reading them needs no network.
import { Button, Dialog } from "./ui";

export function UsageGuidelines({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Dialog open={open} onClose={onClose} title="Usage guidelines" footer={<Button onClick={onClose}>Close</Button>}>
      <div className="space-y-4 text-sm text-neutral-700 dark:text-neutral-300">
        <p>
          Pinhole is for creative work: art, illustration, design, concepts and photo edits. These guidelines apply to
          everything you make with it.
        </p>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">Not allowed</h3>
          <ul className="mt-1.5 list-disc space-y-1 pl-5">
            <li>Sexual content involving anyone under 18, or anyone who looks under 18.</li>
            <li>Sexual or intimate images of a real person without their consent, including edits of their photos.</li>
            <li>Pictures of real people made to deceive, embarrass or harass them.</li>
            <li>Fake documents, IDs, receipts or evidence.</li>
          </ul>
        </section>
        <section>
          <h3 className="font-semibold text-neutral-900 dark:text-neutral-100">Sharing</h3>
          <p className="mt-1.5">
            Saved pictures carry a small "made with AI" note. Don't present a made or edited picture as a real photo in
            a way that could mislead people.
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
            A small checker runs on your computer and looks at each picture before it's shown. It stops sexual images
            of anyone who looks under 18, nude or intimate edits of photos of real people, and adult images from models
            marked for safe images only. It works offline, can't be turned off and keeps no record. It can make
            mistakes; when it stops a picture, your prompt and settings are kept.
          </p>
        </section>
      </div>
    </Dialog>
  );
}
