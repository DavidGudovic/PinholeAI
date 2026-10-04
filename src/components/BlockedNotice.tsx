// The usage guidelines, opened again when the safety check stops something, with the block
// message from Rust (one sentence for the rule) under "Why it was blocked".
import { useEffect, useState } from "react";
import { currentBlocked, dismissBlocked, onBlocked } from "../lib/blocked";
import { UsageGuidelines } from "./UsageGuidelines";

export function BlockedNotice() {
  const [message, setMessage] = useState<string | null>(currentBlocked);
  useEffect(() => onBlocked(setMessage), []);
  const notice = message && (
    <>
      <span className="font-semibold">Why it was blocked:</span> {message}
    </>
  );
  return <UsageGuidelines open={message !== null} onClose={dismissBlocked} notice={notice} />;
}
