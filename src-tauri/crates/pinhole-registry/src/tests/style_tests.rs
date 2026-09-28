use super::*;
use crate::style::{combine, FinalPrompt};
use crate::Family;

fn fam(id: &str) -> &'static Family {
    shipped().family(id).unwrap()
}

fn c(
    id: &str,
    prompt: &str,
    style: Option<&str>,
    style_neg: Option<&str>,
    neg: Option<&str>,
    prefix: bool,
) -> FinalPrompt {
    combine(shipped(), fam(id), prompt, style, style_neg, neg, prefix)
}

#[test]
fn natural_template() {
    let p = c(
        "flux1_dev",
        "a red fox in snow",
        Some("35mm film, soft light"),
        Some("cartoon"),
        Some("ignored"),
        true,
    );
    assert_eq!(p.prompt, "a red fox in snow. Style: 35mm film, soft light");
    assert_eq!(p.negative, None); // Flux does not use negatives
    assert_eq!(
        c(
            "z_image_turbo",
            "  a red fox.  ",
            Some(" film "),
            None,
            None,
            true
        )
        .prompt,
        "a red fox. Style: film"
    );
    assert_eq!(
        c("z_image_turbo", "Wow!", Some("film"), None, None, true).prompt,
        "Wow! Style: film"
    );
    assert_eq!(
        c(
            "qwen_image_edit_2511",
            "make it blue",
            None,
            None,
            None,
            true
        )
        .prompt,
        "make it blue"
    );
    assert_eq!(
        c("z_image_turbo", "", Some("film"), None, None, true).prompt,
        "film"
    );
    assert_eq!(
        c("z_image_turbo", " ", Some("  "), None, None, true).prompt,
        ""
    );
    // Placeholders inside user text are not substituted.
    assert_eq!(
        c("flux1_dev", "{style} sign", Some("neon"), None, None, true).prompt,
        "{style} sign. Style: neon"
    );
}

#[test]
fn tags_template_and_negatives() {
    let p = c(
        "sdxl",
        "a cat on a sofa",
        Some("watercolor, soft"),
        Some("photo, 3d"),
        None,
        true,
    );
    assert_eq!(p.prompt, "a cat on a sofa, watercolor, soft");
    assert_eq!(
        p.negative.as_deref(),
        Some("lowres, blurry, bad anatomy, deformed hands, watermark, text, photo, 3d")
    );
    assert_eq!(
        c("sdxl", "a cat, ", Some(", watercolor"), None, None, true).prompt,
        "a cat, watercolor"
    );
    assert_eq!(c("sd15", "a cat", None, None, None, true).prompt, "a cat");
    assert_eq!(
        c("sd15", "", Some("watercolor"), None, None, true).prompt,
        "watercolor"
    );

    // User override replaces the family default; style negative still appended.
    assert_eq!(
        c("sdxl", "x", None, Some("cartoon"), Some("blurry, "), true)
            .negative
            .as_deref(),
        Some("blurry, cartoon")
    );
    assert_eq!(
        c("sdxl", "x", None, None, Some(""), true)
            .negative
            .as_deref(),
        Some("")
    );
    assert_eq!(
        c("sdxl", "x", None, Some(" "), Some(""), true)
            .negative
            .as_deref(),
        Some("")
    );
    // Family with negatives but no default negative.
    assert_eq!(
        c("z_image_base", "x", None, Some("cartoon"), None, true)
            .negative
            .as_deref(),
        Some("cartoon")
    );
}

#[test]
fn pony_prefix() {
    let p = c(
        "sdxl_pony",
        "a knight",
        Some("oil painting"),
        None,
        None,
        true,
    );
    assert_eq!(
        p.prompt,
        "score_9, score_8_up, score_7_up, a knight, oil painting"
    );
    assert_eq!(
        p.negative.as_deref(),
        Some("score_4, score_5, score_6, lowres, bad anatomy")
    );
    assert_eq!(
        c(
            "sdxl_pony",
            "a knight",
            Some("oil painting"),
            None,
            None,
            false
        )
        .prompt,
        "a knight, oil painting"
    );
    // Not added twice when the user typed it.
    assert_eq!(
        c(
            "sdxl_pony",
            "score_9, score_8_up, score_7_up, a knight",
            None,
            None,
            None,
            true
        )
        .prompt,
        "score_9, score_8_up, score_7_up, a knight"
    );
    assert_eq!(
        c("sdxl_pony", "", None, None, None, true).prompt,
        "score_9, score_8_up, score_7_up"
    );
    assert_eq!(
        c("sdxl_illustrious", "1girl", None, None, None, true).prompt,
        "masterpiece, best quality, 1girl"
    );
    // Families without a prefix ignore `apply_prefix`.
    assert_eq!(
        c("sdxl", "a knight", None, None, None, true).prompt,
        "a knight"
    );
}
