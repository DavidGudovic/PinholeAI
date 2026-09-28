//! In-memory ring buffer for engine stdout/stderr (CLAUDE.md privacy rule 6).
//! Never written to disk. Every line is redacted before it is stored:
//! * lines containing the current prompt / negative prompt (or a long piece of
//!   it) are replaced by `[redacted]`;
//! * anything after `prompt:` / `prompt=` / `"prompt"` / ` -p ` is cut;
//! * sd.cpp's `json parse failed <body>` error (which would echo the request
//!   body) is cut after the marker.
//!
//! sd-server prints a progress bar with `printf("\r  |====>   | 3/20 - 1.2it/s")`
//! (sampling, `=`) and `|####   | 120/900 - 300MB/s` (loading tensors, `#`)
//! regardless of `--log-level`. Those segments are parsed into
//! [`StepProgress`] and not stored as lines.

use std::collections::VecDeque;
use std::time::Instant;

use parking_lot::Mutex;

/// Default capacity (SPEC §2: last ~200 lines).
pub const DEFAULT_LINES: usize = 200;
const MAX_LINE_CHARS: usize = 400;
/// Pieces of a secret at least this long are matched inside lines.
const WINDOW_CHARS: usize = 20;
pub const REDACTED: &str = "[redacted]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressKind {
    /// Sampling steps (`=` bar).
    Sampling,
    /// Loading model tensors (`#` bar).
    Loading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepProgress {
    pub kind: ProgressKind,
    pub step: u32,
    pub total: u32,
    pub at: Instant,
}

#[derive(Default)]
struct Inner {
    lines: VecDeque<String>,
    cap: usize,
    /// Current prompt texts (lowercased), in memory only, cleared after each job.
    secrets: Vec<String>,
    progress: Option<StepProgress>,
    /// Partial line not yet terminated by `\n` / `\r`.
    partial: String,
}

/// Thread-safe ring buffer. Share as `Arc<LogBuffer>`.
pub struct LogBuffer {
    inner: Mutex<Inner>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_LINES)
    }
}

impl std::fmt::Debug for LogBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogBuffer").field("lines", &self.inner.lock().lines.len()).finish()
    }
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self { inner: Mutex::new(Inner { cap: cap.max(1), ..Default::default() }) }
    }

    /// Texts that must never be stored (current prompt, negative prompt, style).
    /// Kept in memory only; call [`LogBuffer::clear_secrets`] when the job ends.
    pub fn set_secrets<I, S>(&self, secrets: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut g = self.inner.lock();
        g.secrets = secrets
            .into_iter()
            .map(|s| s.as_ref().trim().to_lowercase())
            .filter(|s| s.chars().count() >= 4)
            .collect();
    }

    pub fn clear_secrets(&self) {
        self.inner.lock().secrets.clear();
    }

    /// Feed raw output bytes (may contain partial lines, `\r` progress updates).
    pub fn push_bytes(&self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let mut g = self.inner.lock();
        let mut buf = std::mem::take(&mut g.partial);
        buf.push_str(&text);
        let mut rest = buf.as_str();
        while let Some(pos) = rest.find(['\n', '\r']) {
            let seg = &rest[..pos];
            Self::push_segment(&mut g, seg);
            rest = &rest[pos + 1..];
        }
        // sd.cpp starts each progress update with `\r`, so the newest bar is the
        // unterminated tail: read progress from it right away.
        if let Some((kind, step, total)) = parse_progress(strip_control(rest).trim()) {
            g.progress = Some(StepProgress { kind, step, total, at: Instant::now() });
        }
        // Cap an unterminated segment so a runaway line can't grow forever.
        let keep: String = rest.chars().take(8 * MAX_LINE_CHARS).collect();
        g.partial = keep;
    }

    /// Store one complete line (redacted).
    pub fn push_line(&self, line: &str) {
        let mut g = self.inner.lock();
        Self::push_segment(&mut g, line);
    }

    /// Flush a pending partial line (process exited).
    pub fn flush(&self) {
        let mut g = self.inner.lock();
        let partial = std::mem::take(&mut g.partial);
        if !partial.is_empty() {
            Self::push_segment(&mut g, &partial);
        }
    }

    fn push_segment(g: &mut Inner, seg: &str) {
        let clean = strip_control(seg);
        let trimmed = clean.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some((kind, step, total)) = parse_progress(trimmed) {
            g.progress = Some(StepProgress { kind, step, total, at: Instant::now() });
            return;
        }
        let line = redact_line(trimmed, &g.secrets);
        if g.lines.len() >= g.cap {
            g.lines.pop_front();
        }
        g.lines.push_back(line);
    }

    /// Latest progress bar value seen since the last [`LogBuffer::reset_progress`].
    pub fn progress(&self) -> Option<StepProgress> {
        self.inner.lock().progress
    }

    pub fn reset_progress(&self) {
        self.inner.lock().progress = None;
    }

    /// Last `n` lines (oldest first).
    pub fn tail(&self, n: usize) -> Vec<String> {
        let g = self.inner.lock();
        let skip = g.lines.len().saturating_sub(n);
        g.lines.iter().skip(skip).cloned().collect()
    }

    pub fn tail_text(&self, n: usize) -> String {
        self.tail(n).join("\n")
    }

    pub fn len(&self) -> usize {
        self.inner.lock().lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop everything (Clear session / engine restart).
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.lines.clear();
        g.partial.clear();
        g.progress = None;
    }
}

/// Remove ANSI escape sequences and control characters; cap the length.
fn strip_control(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // CSI: ESC [ ... final byte in @..~
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if ('@'..='~').contains(&c2) {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\t' {
            out.push(' ');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out
}

/// Parse `|=====>   | 3/20 - 1.23it/s` or `|####  | 12/300 - 5.0MB/s`.
pub fn parse_progress(seg: &str) -> Option<(ProgressKind, u32, u32)> {
    let s = seg.trim_start();
    let rest = s.strip_prefix('|')?;
    let bar_end = rest.find('|')?;
    let bar = &rest[..bar_end];
    if bar.is_empty() || !bar.chars().all(|c| matches!(c, '=' | '>' | '#' | ' ')) {
        return None;
    }
    let kind = if bar.contains('#') { ProgressKind::Loading } else { ProgressKind::Sampling };
    let after = rest[bar_end + 1..].trim_start();
    let nums = after.split_whitespace().next()?;
    let (a, b) = nums.split_once('/')?;
    let step: u32 = a.parse().ok()?;
    let total: u32 = b.parse().ok()?;
    if total == 0 || step > total {
        return None;
    }
    Some((kind, step, total))
}

/// Redact a single line (see module docs). `secrets` must be lowercase.
pub fn redact_line(line: &str, secrets: &[String]) -> String {
    let lower = line.to_lowercase();
    for secret in secrets {
        if contains_secret(&lower, secret) {
            return REDACTED.to_string();
        }
    }
    let mut out = line.to_string();
    // Cut anything following markers that introduce prompt text.
    let markers = ["json parse failed", "prompt:", "prompt=", "\"prompt\"", "'prompt'", " -p ", "--prompt"];
    let lower_out = out.to_lowercase();
    let mut cut: Option<usize> = None;
    for m in markers {
        if let Some(pos) = lower_out.find(m) {
            let end = pos + m.len();
            cut = Some(cut.map_or(end, |c: usize| c.min(end)));
        }
    }
    if let Some(end) = cut {
        // `end` is a byte offset into the lowercase copy; lowercase can change
        // byte lengths for non-ASCII text, so fall back to a full redaction then.
        if lower_out.len() == out.len() && out.is_char_boundary(end) {
            out.truncate(end);
            let kept = out.trim_end().len();
            out.truncate(kept);
            out.push(' ');
            out.push_str(REDACTED);
        } else {
            return REDACTED.to_string();
        }
    }
    if out.chars().count() > MAX_LINE_CHARS {
        out = out.chars().take(MAX_LINE_CHARS).collect::<String>() + "…";
    }
    out
}

fn contains_secret(line_lower: &str, secret: &str) -> bool {
    if secret.is_empty() {
        return false;
    }
    if line_lower.contains(secret) {
        return true;
    }
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= WINDOW_CHARS {
        return false;
    }
    // Any 20-character piece of the secret (e.g. a truncated / wrapped echo).
    let step = 4;
    let mut i = 0;
    while i + WINDOW_CHARS <= chars.len() {
        let piece: String = chars[i..i + WINDOW_CHARS].iter().collect();
        if piece.trim().chars().count() >= WINDOW_CHARS / 2 && line_lower.contains(&piece) {
            return true;
        }
        i += step;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "PINHOLE_SENTINEL_7f3a a red fox in the snow";

    #[test]
    fn ring_keeps_last_n() {
        let b = LogBuffer::new(3);
        for i in 0..10 {
            b.push_line(&format!("line {i}"));
        }
        assert_eq!(b.tail(10), vec!["line 7", "line 8", "line 9"]);
        assert_eq!(b.tail(1), vec!["line 9"]);
        b.clear();
        assert!(b.is_empty());
    }

    #[test]
    fn redacts_lines_with_the_prompt() {
        let b = LogBuffer::new(50);
        b.set_secrets([SENTINEL, "blurry, lowres"]);
        b.push_line(&format!("[WARN ] something about {SENTINEL} here"));
        b.push_line("[WARN ] something PINHOLE_SENTINEL_7F3A A RED FOX in caps");
        b.push_line("negative was: BLURRY, LOWRES");
        // A 20+ char fragment of the prompt (e.g. a truncated echo) is also caught.
        b.push_line("token dump: sentinel_7f3a a red fox in th");
        b.push_line("[ERROR] model loader: out of memory");
        let all = b.tail_text(50);
        assert!(!all.to_lowercase().contains("sentinel"), "{all}");
        assert!(!all.to_lowercase().contains("red fox"), "{all}");
        assert!(!all.to_lowercase().contains("lowres"), "{all}");
        assert!(all.contains("out of memory"));
        assert_eq!(b.tail(5).iter().filter(|l| *l == REDACTED).count(), 4);
        b.clear_secrets();
    }

    #[test]
    fn cuts_after_prompt_markers_even_without_secrets() {
        assert_eq!(redact_line("[ERROR] json parse failed {\"prompt\":\"secret\"}", &[]), "[ERROR] json parse failed [redacted]");
        assert_eq!(redact_line("prompt: a cat", &[]), "prompt: [redacted]");
        assert_eq!(redact_line("run sd -p a cat --steps 3", &[]), "run sd -p [redacted]");
        assert_eq!(redact_line("{\"prompt\": \"x\"}", &[]), "{\"prompt\" [redacted]");
        // Plain mentions of the word are fine.
        assert_eq!(
            redact_line("IMPORTANT NOTICE: No text encoders provided, cannot process prompts!", &[]),
            "IMPORTANT NOTICE: No text encoders provided, cannot process prompts!"
        );
        // Non-ASCII where lowercasing changes byte length → whole-line redaction.
        assert_eq!(redact_line("İİİ prompt: x", &[]), REDACTED);
    }

    #[test]
    fn parses_progress_bars_and_does_not_store_them() {
        let b = LogBuffer::new(50);
        b.push_bytes(b"\r  |>                                                 | 1/20 - 0.00it/s\x1b[K");
        b.push_bytes(b"\r  |=====>                                            | 3/20 - 1.23it/s\x1b[K");
        let p = b.progress().unwrap();
        assert_eq!((p.kind, p.step, p.total), (ProgressKind::Sampling, 3, 20));
        b.push_bytes(b"\r  |##########                                        | 120/600 - 300.00MB/s\x1b[K\n");
        let p = b.progress().unwrap();
        assert_eq!((p.kind, p.step, p.total), (ProgressKind::Loading, 120, 600));
        assert!(b.is_empty(), "progress segments are not stored as lines");
        b.push_bytes(b"[WARN ] partial ");
        assert!(b.is_empty());
        b.push_bytes(b"line\n[ERROR] second\r\n");
        assert_eq!(b.tail(5), vec!["[WARN ] partial line", "[ERROR] second"]);
        b.reset_progress();
        assert!(b.progress().is_none());
    }

    #[test]
    fn parse_progress_rejects_other_text() {
        assert_eq!(parse_progress("| not a bar | 1/2"), None);
        assert_eq!(parse_progress("hello 3/20"), None);
        assert_eq!(parse_progress("|===| 30/20 - x"), None);
        assert_eq!(parse_progress("|===| 3/20 - 1it/s"), Some((ProgressKind::Sampling, 3, 20)));
    }

    #[test]
    fn strips_ansi_and_caps_length() {
        let b = LogBuffer::new(5);
        b.push_line("\x1b[35;1m[WARN   ]\x1b[0m hello");
        assert_eq!(b.tail(1), vec!["[WARN   ] hello"]);
        let long = "x".repeat(5000);
        b.push_line(&long);
        assert!(b.tail(1)[0].chars().count() <= MAX_LINE_CHARS + 1);
    }
}
