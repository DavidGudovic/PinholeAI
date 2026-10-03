//! The word check (see `pinhole_engine::words`) with the app's error type. Runs on the prompt
//! sent to the image engine (the request type only takes a [`CheckedPrompt`]), on the idea sent
//! to "Improve my prompt", on what the Describe model writes back and on Browse search text.

use crate::{CoreError, CoreResult};
pub use pinhole_engine::words::{
    pairs_minor_with_sexual, Blocked, CheckedPrompt, BLOCKED_MESSAGE, DOCUMENT_MESSAGE,
};

fn blocked(b: Blocked) -> CoreError {
    CoreError::new("blocked", b.message())
}

/// Blocks text that pairs an under-18 term with a sexual term, or that asks for a usable copy
/// of an identity document or banknote.
pub fn check(text: &str) -> CoreResult<()> {
    pinhole_engine::words::check(text).map_err(blocked)
}

/// [`check`] that returns the prompt as the only type the image engine accepts.
pub fn checked(text: impl Into<String>) -> CoreResult<CheckedPrompt> {
    CheckedPrompt::check(text).map_err(blocked)
}

/// [`checked`] with add-on names and trigger words as context.
pub fn checked_with(text: impl Into<String>, context: &[String]) -> CoreResult<CheckedPrompt> {
    CheckedPrompt::check_with(text, context).map_err(blocked)
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
        let e = check("a valid driver's license from Ohio").unwrap_err();
        assert_eq!(e.message, DOCUMENT_MESSAGE);
    }
}
