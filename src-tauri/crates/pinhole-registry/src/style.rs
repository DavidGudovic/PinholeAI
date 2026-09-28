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

/// Used when the registry has no template of the family's name.
const FALLBACK_NATURAL: &str = "{prompt}. Style: {style}";
const FALLBACK_TAGS: &str = "{prompt}, {style}";

/// Combine user prompt + optional style + family prefix / negatives.
///
/// * prompt/style joined with the family's `style_template`
/// * `auto_prompt_prefix` prepended when `apply_prefix`
/// * negative = user negative override if `Some`, else family default negative,
///   then style negative appended — only when `family.uses_negative_prompt`.
///
/// Everything is trimmed; an empty style or prompt never leaves a dangling
/// separator ("a cat" + no style = "a cat", no prompt + style = the style),
/// and punctuation already at the end of the prompt is not doubled.
pub fn combine(
    registry: &Registry,
    family: &Family,
    prompt: &str,
    style_positive: Option<&str>,
    style_negative: Option<&str>,
    negative_override: Option<&str>,
    apply_prefix: bool,
) -> FinalPrompt {
    let template = registry.style_template(&family.style_template).unwrap_or(
        match family.style_template.as_str() {
            "natural" => FALLBACK_NATURAL,
            _ => FALLBACK_TAGS,
        },
    );
    let mut text = join_with_template(template, prompt, style_positive.unwrap_or(""));

    if apply_prefix {
        if let Some(prefix) = family.defaults.auto_prompt_prefix.as_deref() {
            text = add_prefix(prefix, &text);
        }
    }

    let negative = family.uses_negative_prompt.then(|| {
        let base = negative_override
            .or(family.defaults.negative_prompt.as_deref())
            .unwrap_or("");
        join_list(&[base, style_negative.unwrap_or("")])
    });
    FinalPrompt {
        prompt: text,
        negative,
    }
}

/// Trim whitespace and stray list separators from both ends.
fn clean(s: &str) -> &str {
    s.trim()
        .trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace())
}

/// `a, b` for non-empty parts.
fn join_list(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| clean(p))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

fn add_prefix(prefix: &str, text: &str) -> String {
    let prefix = clean(prefix);
    if prefix.is_empty() {
        return text.to_owned();
    }
    // Don't add it twice when the user already typed it.
    if text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        return text.to_owned();
    }
    join_list(&[prefix, text])
}

/// Substitute `{prompt}` / `{style}` in one pass (user text is never re-scanned
/// for placeholders). Empty sides collapse to the other side alone.
fn join_with_template(template: &str, prompt: &str, style: &str) -> String {
    let prompt = clean(prompt);
    let style =
        clean(style.trim_start_matches(|c: char| c == '.' || c == ',' || c.is_whitespace()));
    match (prompt.is_empty(), style.is_empty()) {
        (true, true) => return String::new(),
        (false, true) => return prompt.to_owned(),
        (true, false) => return style.to_owned(),
        (false, false) => {}
    }

    // Literal text right after {prompt}: drop matching punctuation the prompt already ends with.
    let sep = template
        .find("{prompt}")
        .map(|i| &template[i + "{prompt}".len()..])
        .map(|rest| rest.split('{').next().unwrap_or(""))
        .unwrap_or("");
    let first = sep.trim_start().chars().next();
    let mut p = prompt;
    let mut skip_sep_char = false;
    if let Some(c) = first.filter(|c| matches!(c, ',' | '.' | ';' | ':')) {
        p = p.trim_end_matches(|x: char| x == c || x.is_whitespace());
        // "Wow!" + ". Style: …" → "Wow! Style: …"
        if c == '.' && p.ends_with(['!', '?', '…']) {
            skip_sep_char = true;
        }
    }

    let mut out = String::with_capacity(template.len() + p.len() + style.len());
    let mut rest = template;
    let mut after_prompt = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("{prompt}") {
            out.push_str(p);
            rest = r;
            after_prompt = true;
        } else if let Some(r) = rest.strip_prefix("{style}") {
            out.push_str(style);
            rest = r;
            after_prompt = false;
        } else {
            let c = rest.chars().next().unwrap_or_default();
            let is_sep_char =
                after_prompt && skip_sep_char && Some(c) == first && !c.is_whitespace();
            if is_sep_char {
                skip_sep_char = false;
            } else {
                out.push(c);
            }
            if !c.is_whitespace() {
                after_prompt = false;
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out.trim().to_owned()
}
