//! Local word check: text that pairs an under-18 term with a sexual term is blocked,
//! whatever Safe mode says. Runs only on what the Describe model writes (Describe and
//! "Improve my prompt"); the user's own prompt is left to the release image check
//! (RELEASE-SPEC §3). See RELEASE-SPEC §11.
//!
//! Deliberately plain: fixed word lists in code (not YAML, so a config edit can't turn it
//! off), whole words only, both lists must match. It is a first line, not the §3 image check:
//! it misses misspellings and made-up words. PRIVACY: the text and which words matched are
//! never logged, stored or put in an error.

use crate::{CoreError, CoreResult};

/// Shown when text is blocked. No details: they would have to quote the text.
pub const BLOCKED_MESSAGE: &str = "Pinhole doesn't make sexual images or text involving anyone under 18. Change the words and try again.";

/// Terms that point at someone under 18. The last word of each also matches with a trailing
/// `s`/`es` ("little girls").
const UNDER_18: &[&str] = &[
    "child",
    "children",
    "childlike",
    "childish body",
    "kid",
    "kiddie",
    "kiddo",
    "toddler",
    "infant",
    "preteen",
    "pre teen",
    "prepubescent",
    "pubescent",
    "underage",
    "under age",
    "under 18",
    "under eighteen",
    "minor",
    "juvenile",
    "loli",
    "lolis",
    "lolicon",
    "shota",
    "shotacon",
    "toddlercon",
    "jailbait",
    "teen",
    "teenage",
    "teenager",
    "teenaged",
    "high school",
    "highschool",
    "high schooler",
    "highschooler",
    "tween",
    "schoolgirl",
    "school girl",
    "school boy",
    "schoolboy",
    "schoolchild",
    "schoolkid",
    "little girl",
    "little boy",
    "young girl",
    "young boy",
    "elementary school",
    "elementary schooler",
    "middle school",
    "middle schooler",
    "grade school",
    "primary school",
    "kindergarten",
    "kindergartener",
];

/// Clearly sexual terms, same plural rule. Words with common harmless meanings ("cock",
/// "tit", "thong") are left out.
const SEXUAL: &[&str] = &[
    "sex",
    "sexual",
    "sexually",
    "sexy",
    "nsfw",
    "nude",
    "nudity",
    "naked",
    "topless",
    "bottomless",
    "undress",
    "undressed",
    "undressing",
    "no clothes",
    "without clothes",
    "explicit",
    "porn",
    "porno",
    "pornographic",
    "erotic",
    "erotica",
    "hentai",
    "lewd",
    "orgasm",
    "masturbate",
    "masturbating",
    "masturbation",
    "genital",
    "genitalia",
    "penis",
    "vagina",
    "vulva",
    "pussy",
    "nipple",
    "breasts",
    "boob",
    "tits",
    "lingerie",
    "panties",
    "upskirt",
    "pantyshot",
    "cameltoe",
    "spread legs",
    "legs spread",
    "seductive",
    "sensual",
    "provocative",
    "intercourse",
    "penetration",
    "fuck",
    "fucking",
    "anal",
    "creampie",
    "incest",
    "molest",
    "molested",
    "panty",
    "cum",
    "cumshot",
    "blowjob",
    "handjob",
    "fellatio",
    "cunnilingus",
    "paizuri",
    "ahegao",
    "rape",
    "bdsm",
    "bondage",
    "fetish",
    "aroused",
    "arousal",
    "horny",
    "stripper",
    "stripping",
];

const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
];

/// Blocks text that pairs an under-18 term with a sexual term.
pub fn check(text: &str) -> CoreResult<()> {
    if pairs_minor_with_sexual(text) {
        Err(CoreError::new("blocked", BLOCKED_MESSAGE))
    } else {
        Ok(())
    }
}

/// True when `text` has at least one under-18 term and at least one sexual term.
pub fn pairs_minor_with_sexual(text: &str) -> bool {
    let words = tokens(text);
    let minor = UNDER_18.iter().any(|p| has_phrase(&words, p)) || has_young_age(&words);
    minor && SEXUAL.iter().any(|p| has_phrase(&words, p))
}

/// Lowercase words; anything that isn't a letter or digit separates them, so booru tags
/// (`school_girl`), weights (`(loli:1.2)`) and hyphens (`12-year-old`) split the same way.
/// Zero-width characters are dropped (so they can't split a word) and fullwidth letters and
/// digits become plain ones. "naked eye" is taken out first.
fn tokens(text: &str) -> Vec<String> {
    let plain: String = text
        .chars()
        .filter(|c| {
            !matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
        })
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect::<String>()
        .to_lowercase();
    let words: Vec<String> = plain
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        if words[i] == "naked" && words.get(i + 1).is_some_and(|w| w == "eye") {
            i += 2;
            continue;
        }
        out.push(words[i].clone());
        i += 1;
    }
    out
}

fn word_matches(token: &str, word: &str) -> bool {
    token == word
        || token
            .strip_prefix(word)
            .is_some_and(|rest| rest == "s" || rest == "es")
}

fn has_phrase(words: &[String], phrase: &str) -> bool {
    let parts: Vec<&str> = phrase.split(' ').collect();
    let last = parts.len() - 1;
    words.windows(parts.len()).any(|win| {
        win.iter().zip(&parts).enumerate().all(|(i, (w, p))| {
            if i == last {
                word_matches(w, p)
            } else {
                w == p
            }
        })
    })
}

/// An age from 1 to 17: "12", "twelve" or "12yo" followed by "yo", "y o", "year old",
/// "years old", "year olds", "yr old", "yrs old" or "years of age"; or "aged 12", "age of 12".
/// A bare "age 12" doesn't count ("Bronze Age, 3 statues").
fn has_young_age(words: &[String]) -> bool {
    let young =
        |w: &str| w.parse::<u32>().is_ok_and(|n| (1..18).contains(&n)) || NUMBER_WORDS.contains(&w);
    let at = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    for (i, w) in words.iter().enumerate() {
        // Joined forms: "12yo", "12yr", "12yrs", "12y".
        let digits: String = w.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && young(&digits) {
            let rest = &w[digits.len()..];
            if rest == "yo" {
                return true;
            }
            if matches!(rest, "y" | "yr" | "yrs") && (at(i + 1) == "old" || at(i + 1) == "o") {
                return true;
            }
        }
        if young(w) {
            let next = at(i + 1);
            if next == "yo" || (next == "y" && at(i + 1 + 1) == "o") {
                return true;
            }
            if matches!(next, "year" | "years" | "yr" | "yrs")
                && (matches!(at(i + 2), "old" | "olds")
                    || (at(i + 2) == "of" && at(i + 3) == "age"))
            {
                return true;
            }
        }
        if (w == "aged" && young(at(i + 1)))
            || (w == "age" && at(i + 1) == "of" && young(at(i + 2)))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_under_18_terms_paired_with_sexual_terms() {
        for t in [
            "a child, nude",
            "Loli, NSFW, 1girl",
            "(shota:1.2), naked",
            "school_girl uniform, lingerie",
            "schoolgirl in lingerie",
            "a teenager posing topless",
            "little girl, sexy pose",
            "kids, explicit",
            "a 12 year old, nude",
            "12-year-old, erotic",
            "15yo, nsfw",
            "14 y/o, naked",
            "sixteen years old, lewd",
            "aged 13, seductive",
            "age of 9, sexual",
            "preteen, spread legs",
            "pre-teen, lingerie",
            "under 18, nude",
            "little girls, nude",
            "young boys, naked",
            "school girls, lingerie",
            "middle schoolers, nsfw",
            "a minor, nude",
            "highschool student, naked",
            "13 year-olds, lewd",
            "12 years of age, nude",
            "lo\u{200B}li, nude",
            "\u{FF4C}\u{FF4F}\u{FF4C}\u{FF49}, nude",
        ] {
            assert!(pairs_minor_with_sexual(t), "{t}");
        }
    }

    #[test]
    fn allows_either_list_alone_and_adult_text() {
        for t in [
            "a child flying a kite in a park, golden hour",
            "kids playing on the beach, swimsuits",
            "a 12 year old boy reading a book",
            "a nude woman, oil painting, art nude",
            "1girl, nsfw, large breasts, adult woman",
            "25 year old woman, lingerie, bedroom",
            "18yo, nude",
            "a 30 year old couple, erotic, sensual lighting",
            "the heart of a statue, nude marble",
            "12 years later, a naked tree in winter",
            "breastfeeding mother with her baby",
            "a sextant on a ship's chart",
            "skidding car on a wet road, sexy lighting",
            "Bronze Age, 3 nude marble statues",
            "a child looking at the stars with the naked eye",
            "kids watching a blue tit and a cock on the farm",
            "kids in thongs on the beach",
        ] {
            assert!(!pairs_minor_with_sexual(t), "{t}");
        }
    }

    #[test]
    fn matches_whole_words_even_inside_hyphenated_words() {
        // "kid" in "kid-friendly" is a whole word, so this is blocked. Accepted: rare, and
        // the message says to change the words.
        assert!(pairs_minor_with_sexual(
            "sexy sports car, kid-friendly cabin"
        ));
    }

    #[test]
    fn check_returns_a_plain_error_without_the_text() {
        let e = check("loli, nude, PINHOLE_SENTINEL_7f3a").unwrap_err();
        assert_eq!(e.code, "blocked");
        assert_eq!(e.message, BLOCKED_MESSAGE);
        assert!(e.details.is_none());
        assert!(check("a nude woman, oil painting").is_ok());
    }
}
