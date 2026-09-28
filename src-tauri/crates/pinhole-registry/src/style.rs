//! Prompt + style combination (SPEC §7). IN MEMORY ONLY: results must never be
//! written to disk or logs.

use crate::{Family, Registry};

/// The final prompt/negative sent to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalPrompt {
    pub prompt: String,
    /// `None` for families that don't use negative prompts.
    pub negative: Option<String>,
}

/// Combine user prompt + optional style + family prefix / negatives.
///
/// * prompt/style joined with the family's `style_template`
/// * `auto_prompt_prefix` prepended when `apply_prefix`
/// * negative = user negative override if `Some`, else family default negative,
///   then style negative appended — only when `family.uses_negative_prompt`.
pub fn combine(
    registry: &Registry,
    family: &Family,
    prompt: &str,
    style_positive: Option<&str>,
    style_negative: Option<&str>,
    negative_override: Option<&str>,
    apply_prefix: bool,
) -> FinalPrompt {
    let _ = (registry, family, prompt, style_positive, style_negative, negative_override, apply_prefix);
    todo!("registry agent")
}
