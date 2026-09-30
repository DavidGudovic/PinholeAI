// The usage guidelines, opened again when the safety check stops something. The message is the
// block message from Rust as is: it never says what triggered the block.
import { useEffect, useState } from "react";
import { currentBlocked, dismissBlocked, onBlocked } from "../lib/blocked";
import { UsageGuidelines } from "./UsageGuidelines";

export function BlockedNotice() {
  const [message, setMessage] = useState<string | null>(currentBlocked);
  useEffect(() => onBlocked(setMessage), []);
  return <UsageGuidelines open={message !== null} onClose={dismissBlocked} notice={message} />;
}
