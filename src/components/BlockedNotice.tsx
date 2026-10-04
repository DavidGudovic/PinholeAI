// The usage guidelines, opened again when the safety check stops something: "Pinhole can't help
// with this." and the block message from Rust (one sentence for the rule) under "Why it was blocked".
import { useEffect, useState } from "react";
import { currentBlocked, dismissBlocked, onBlocked } from "../lib/blocked";
import { UsageGuidelines } from "./UsageGuidelines";

export const CANT_HELP = "Pinhole can't help with this.";

export function BlockedNotice() {
  const [message, setMessage] = useState<string | null>(currentBlocked);
  useEffect(() => onBlocked(setMessage), []);
  const notice = message && (
    <>
      <p className="font-semibold">{CANT_HELP}</p>
      <p className="mt-1">
        <span className="font-semibold">Why it was blocked:</span> {message}
      </p>
    </>
  );
  return <UsageGuidelines open={message !== null} onClose={dismissBlocked} notice={notice} />;
}
