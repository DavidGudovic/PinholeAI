// The usage guidelines, opened again when the safety check stops something, with the block
// message from Rust ("Pinhole cannot help with this. Reason: …") on top.
import { useEffect, useState } from "react";
import { currentBlocked, dismissBlocked, onBlocked } from "../lib/blocked";
import { UsageGuidelines } from "./UsageGuidelines";

export function BlockedNotice() {
  const [message, setMessage] = useState<string | null>(currentBlocked);
  useEffect(() => onBlocked(setMessage), []);
  return <UsageGuidelines open={message !== null} onClose={dismissBlocked} notice={message} />;
}
