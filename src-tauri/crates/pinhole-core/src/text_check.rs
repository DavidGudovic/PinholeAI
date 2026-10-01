//! The word check (see `pinhole_engine::words`) with the app's error type. Runs on the prompt
//! sent to the image engine (the request type only takes a [`CheckedPrompt`]), on the idea sent
//! to "Improve my prompt", on what the Describe model writes back and on Browse search text.
//! A prompt for a picture made from pictures Pinhole made in this session is also checked
//! together with the prompts that made them ([`check_with_inputs`]).

use std::sync::Arc;

use crate::session::SessionImage;
use crate::{CoreError, CoreResult};
pub use pinhole_engine::words::{pairs_minor_with_sexual, CheckedPrompt, BLOCKED_MESSAGE};

fn blocked() -> CoreError {
    CoreError::new("blocked", BLOCKED_MESSAGE)
}

/// Blocks text that pairs an under-18 term with a sexual term, or that asks for a usable copy
/// of an identity document or banknote.
pub fn check(text: &str) -> CoreResult<()> {
    pinhole_engine::words::check(text).map_err(|_| blocked())
}

/// [`check`] that returns the prompt as the only type the image engine accepts.
pub fn checked(text: impl Into<String>) -> CoreResult<CheckedPrompt> {
    CheckedPrompt::check(text).map_err(|_| blocked())
}

/// [`checked`] with add-on names and trigger words as context.
pub fn checked_with(text: impl Into<String>, context: &[String]) -> CoreResult<CheckedPrompt> {
    CheckedPrompt::check_with(text, context).map_err(|_| blocked())
}

/// The prompts (with add-on names and trigger words) of every step that made a picture in this
/// session, each once. Memory only: never shown, logged, saved or sent anywhere; gone on Reset.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MadeWith(Arc<[Arc<str>]>);

impl MadeWith {
    /// `step`'s text followed by what made each of `inputs`.
    pub fn joined<'a>(step: Option<&str>, inputs: impl IntoIterator<Item = &'a MadeWith>) -> Self {
        let mut parts: Vec<Arc<str>> = Vec::new();
        let step = step.filter(|s| !s.trim().is_empty()).map(Arc::from);
        for p in step
            .into_iter()
            .chain(inputs.into_iter().flat_map(|m| m.0.iter().cloned()))
        {
            if !parts.contains(&p) {
                parts.push(p);
            }
        }
        Self(Arc::from(parts))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn text(&self) -> String {
        self.0.join("\n")
    }
}

impl std::fmt::Debug for MadeWith {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MadeWith([redacted])")
    }
}

/// [`check`] on `step` (the new prompt with its add-on names and trigger words) together with
/// the prompts that made `inputs`. Pictures brought in from outside carry none.
pub fn check_with_inputs(step: &str, inputs: &[SessionImage]) -> CoreResult<()> {
    if inputs.iter().all(|i| i.made_with.is_empty()) {
        return Ok(());
    }
    check(&MadeWith::joined(Some(step), inputs.iter().map(|i| &i.made_with)).text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_returns_a_plain_error_without_the_text() {
        let e = check("loli, nude, PINHOLE_SENTINEL_7f3a").unwrap_err();
        assert_eq!(e.code, "blocked");
        assert_eq!(e.message, BLOCKED_MESSAGE);
        assert!(e.details.is_none());
        assert!(check("a nude woman, oil painting").is_ok());
        assert!(checked("loli, nude").is_err());
    }

    #[test]
    fn made_with_keeps_each_prompt_once_and_hides_it_from_debug() {
        let a = MadeWith::joined(Some("a red barn"), []);
        let b = MadeWith::joined(Some("at night"), [&a]);
        let c = MadeWith::joined(Some("  "), [&b, &a]);
        assert_eq!(c.text(), "at night\na red barn");
        assert!(MadeWith::joined(None, []).is_empty());
        assert_eq!(format!("{c:?}"), "MadeWith([redacted])");
    }
}
