//! Local word check: text that pairs an under-18 term with a sexual term is blocked,
//! whatever Safe mode says. Runs on the prompt sent to the image engine in Create and every
//! Edit mode (after styles, trigger words and add-ons are combined), on the idea sent to
//! "Improve my prompt", on what the Describe model writes back (Describe and Improve) and on
//! Browse search text. See RELEASE-SPEC §11.
//!
//! Lives in the engine crate so the image engine's request type can require a checked prompt
//! (`CheckedPrompt`); `pinhole_core::text_check` wraps it with the app's error.
//!
//! Deliberately plain: fixed word lists in code (not YAML, so a config edit can't turn it
//! off), whole words only, both lists must match. Spellings are normalized first (see
//! [`pairs_minor_with_sexual`]). It is a first line, not the §3 image check: it misses
//! misspellings and made-up words. PRIVACY: the text and which words matched are
//! never logged, stored or put in an error.

use unicode_normalization::UnicodeNormalization;

/// Shown when the check stops something (the text check here, the image check too). Neutral on
/// purpose: no details (they would have to quote the text), no retry hint (it read like an
/// invitation to reword around the check) and no naming of harmful content, so a false block
/// never reads as an accusation (David, 2026-09-30). The UI shows the usage guidelines with it.
pub const BLOCKED_MESSAGE: &str = "Pinhole can't help with this. See the usage guidelines.";

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

/// The word check said no. Carries nothing about the text (PRIVACY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocked;

/// Blocks text that pairs an under-18 term with a sexual term.
pub fn check(text: &str) -> Result<(), Blocked> {
    if pairs_minor_with_sexual(text) {
        Err(Blocked)
    } else {
        Ok(())
    }
}

/// A prompt that passed [`check`]. The only way to make one is [`CheckedPrompt::check`], and
/// [`crate::sdapi::ImgGenRequest::new`] takes nothing else, so no text reaches the image engine
/// without the word check (RELEASE-SPEC §1 request builder).
#[derive(Clone, PartialEq, Eq)]
pub struct CheckedPrompt(String);

impl CheckedPrompt {
    pub fn check(text: impl Into<String>) -> Result<Self, Blocked> {
        let text = text.into();
        check(&text)?;
        Ok(Self(text))
    }

    /// [`CheckedPrompt::check`] on the prompt together with `context` (add-on names and
    /// trigger words, which steer the picture but aren't sent as text).
    pub fn check_with(text: impl Into<String>, context: &[String]) -> Result<Self, Blocked> {
        let text = text.into();
        let mut all = vec![text.clone()];
        all.extend(context.iter().cloned());
        check(&all.join(", "))?;
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for CheckedPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CheckedPrompt([redacted])")
    }
}

/// True when `text` has at least one under-18 term and at least one sexual term. Both lists
/// are matched against every spelling in [`views`], so look-alike letters, numbers for letters,
/// accents, spaced-out letters, repeated letters and glued words don't get around them.
pub fn pairs_minor_with_sexual(text: &str) -> bool {
    let views = views(text);
    let any = |list: &[&str]| views.iter().any(|w| list.iter().any(|p| has_phrase(w, p)));
    // Ages are read from the plain spellings only: numbers-for-letters would turn "18" into
    // something else.
    let minor = any(UNDER_18) || views[..2].iter().any(|w| has_young_age(w));
    minor && any(SEXUAL)
}

/// The text as several lists of lowercase words, all checked:
/// - as written, with invisible characters, accents and apostrophes dropped and compatibility
///   forms (fullwidth, superscript, squared, letterlike, ligatures) made plain;
/// - look-alike letters (Cyrillic and Greek capitals and small letters, small capitals, styled
///   and boxed letters) made Latin;
/// - twice more with numbers and symbols read as letters (`0`→o, `1`→i, then `1`→l, `3`→e,
///   `4`/`@`→a, `5`/`$`→s, `7`→t, `|`, and `!` inside a word), only in words that also have
///   letters.
///
/// In every list, spaced-out letters are joined ("l o l i", "l.o.l.i") and a word made of two
/// listed words glued together is split in two; a listed phrase also matches glued into one word.
fn views(text: &str) -> Vec<Vec<String>> {
    // Compatibility forms (fullwidth, superscript, squared, letterlike, ligatures…) become
    // plain letters; accents come apart from their letters and are dropped. Apostrophes are
    // dropped so "Kim's ex" reads "kims ex", not "s ex".
    let plain: String = text
        .nfkd()
        .filter(|c| !is_invisible(*c) && !is_accent(*c) && !is_apostrophe(*c))
        .collect();
    let clean = plain.to_lowercase();
    // Capital look-alikes are read before lowercasing: a Greek capital Eta looks like "H", its
    // lowercase doesn't.
    let folded: String = plain
        .chars()
        .flat_map(|c| match fold_capital(c) {
            Some(l) => vec![l],
            None => c.to_lowercase().flat_map(fold).collect(),
        })
        .collect();
    let words = |text: &str, keep: &dyn Fn(char) -> bool| -> Vec<String> {
        text.split(|c: char| !keep(c))
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let alnum = |c: char| c.is_alphanumeric();
    let leet_chars = |c: char| c.is_alphanumeric() || matches!(c, '@' | '$' | '!' | '|');
    let leet_words = words(&folded, &leet_chars);
    let raw = [
        words(&clean, &alnum),
        words(&folded, &alnum),
        leet_words.iter().map(|w| leet(w, 'i')).collect(),
        leet_words.iter().map(|w| leet(w, 'l')).collect(),
    ];
    raw.into_iter()
        .map(|w| split_glued(join_pieces(without_naked_eye(w))))
        .collect()
}

/// Characters that show nothing: soft hyphen, zero-width and joiner characters, direction
/// marks, variation selectors, Hangul fillers, blank Braille, format controls, tag characters.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{2800}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}' | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}')
}

/// Combining accents (left over once letters are taken apart).
fn is_accent(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

fn is_apostrophe(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2018}' | '\u{2019}' | '\u{02BC}' | '`' | '\u{00B4}'
    )
}

/// A Cyrillic or Greek capital that looks like a Latin capital whose lowercase doesn't.
fn fold_capital(c: char) -> Option<char> {
    const CAPITALS: &[(&str, char)] = &[
        ("АΑ", 'a'),
        ("ВΒ", 'b'),
        ("СϹ", 'c'),
        ("ЕЁΕ", 'e'),
        ("НΗ", 'h'),
        ("ІЇΙӀ", 'i'),
        ("Ј", 'j'),
        ("КΚ", 'k'),
        ("МΜ", 'm'),
        ("Ν", 'n'),
        ("ОΟ", 'o'),
        ("РΡ", 'p'),
        ("Ԛ", 'q'),
        ("Ѕ", 's'),
        ("ТΤ", 't'),
        ("Ԝ", 'w'),
        ("ХΧ", 'x'),
        ("ҮΥ", 'y'),
        ("Ζ", 'z'),
    ];
    CAPITALS
        .iter()
        .find(|(from, _)| from.contains(c))
        .map(|(_, to)| *to)
}

/// One lowercase character without its accent, or its Latin look-alike.
fn fold(c: char) -> Vec<char> {
    const ACCENTS: &[(&str, char)] = &[
        ("àáâãäåāăąǎȁȃạảấầẩẫậắằẳẵặ", 'a'),
        ("çćĉċč", 'c'),
        ("ďđ", 'd'),
        ("èéêëēĕėęěȅȇẹẻẽếềểễệ", 'e'),
        ("ĝğġģ", 'g'),
        ("ĥħ", 'h'),
        ("ìíîïĩīĭįıǐȉȋịỉ", 'i'),
        ("ĵ", 'j'),
        ("ķ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉ", 'n'),
        ("òóôõöøōŏőǒȍȏọỏốồổỗộớờởỡợơ", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşšș", 's'),
        ("ţťŧț", 't'),
        ("ùúûüũūŭůűųǔȕȗụủứừửữựư", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷỳỵỷỹ", 'y'),
        ("źżž", 'z'),
        // Cyrillic and Greek letters that look like Latin ones.
        ("аα", 'a'),
        ("вβ", 'b'),
        ("сϲ", 'c'),
        ("ԁ", 'd'),
        ("еёєε", 'e'),
        ("ɡ", 'g'),
        ("һ", 'h'),
        ("іїιɩ", 'i'),
        ("ј", 'j'),
        ("кκ", 'k'),
        ("ӏ", 'l'),
        ("м", 'm'),
        ("пη", 'n'),
        ("оοσ", 'o'),
        ("рρ", 'p'),
        ("ѕς", 's'),
        ("тτ", 't'),
        ("υ", 'u'),
        ("ν", 'v'),
        ("ѡω", 'w'),
        ("хχ", 'x'),
        ("уγ", 'y'),
        // Small capitals.
        ("ᴀ", 'a'),
        ("ʙ", 'b'),
        ("ᴄ", 'c'),
        ("ᴅ", 'd'),
        ("ᴇ", 'e'),
        ("ꜰ", 'f'),
        ("ɢ", 'g'),
        ("ʜ", 'h'),
        ("ɪ", 'i'),
        ("ᴊ", 'j'),
        ("ᴋ", 'k'),
        ("ʟ", 'l'),
        ("ᴍ", 'm'),
        ("ɴ", 'n'),
        ("ᴏ", 'o'),
        ("ᴘ", 'p'),
        ("ʀ", 'r'),
        ("ꜱ", 's'),
        ("ᴛ", 't'),
        ("ᴜ", 'u'),
        ("ᴠ", 'v'),
        ("ᴡ", 'w'),
        ("ʏ", 'y'),
        ("ᴢ", 'z'),
    ];
    match c {
        // Combining accents (text typed as letter + accent).
        '\u{0300}'..='\u{036F}' => return Vec::new(),
        'ß' => return vec!['s', 's'],
        'æ' => return vec!['a', 'e'],
        'œ' => return vec!['o', 'e'],
        // Circled letters, and bold/italic/script… letters and digits.
        'ⓐ'..='ⓩ' => return vec![char::from(b'a' + (c as u32 - 0x24D0) as u8)],
        'Ⓐ'..='Ⓩ' => return vec![char::from(b'a' + (c as u32 - 0x24B6) as u8)],
        '\u{1F1E6}'..='\u{1F1FF}' => return vec![char::from(b'a' + (c as u32 - 0x1F1E6) as u8)],
        // Negative circled and negative squared letters.
        '\u{1F150}'..='\u{1F169}' => return vec![char::from(b'a' + (c as u32 - 0x1F150) as u8)],
        '\u{1F170}'..='\u{1F189}' => return vec![char::from(b'a' + (c as u32 - 0x1F170) as u8)],
        '\u{1D400}'..='\u{1D6A3}' => {
            let i = ((c as u32 - 0x1D400) % 52) as u8;
            return vec![char::from(b'a' + i % 26)];
        }
        '\u{1D7CE}'..='\u{1D7FF}' => {
            return vec![char::from(b'0' + ((c as u32 - 0x1D7CE) % 10) as u8)]
        }
        _ => {}
    }
    ACCENTS
        .iter()
        .find(|(from, _)| from.contains(c))
        .map_or(vec![c], |(_, to)| vec![*to])
}

/// Numbers and symbols read as letters, only in words that also have a letter ("18" stays).
fn leet(word: &str, one: char) -> String {
    if !word.chars().any(|c| c.is_alphabetic()) {
        return word.to_string();
    }
    // "!" is a letter only inside a word ("lol!" stays "lol").
    word.trim_matches('!')
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | '|' => one,
            '!' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            _ => c,
        })
        .collect()
}

/// "naked eye" is taken out ("a child looking at the stars with the naked eye").
fn without_naked_eye(words: Vec<String>) -> Vec<String> {
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

/// Spaced-out letters become one word: two or more single letters ("l o l i", "l.o.l.i"), or
/// three or more pieces of one or two letters ("lo l i"). Two two-letter words stay apart
/// ("an al fresco lunch").
fn join_pieces(words: Vec<String>) -> Vec<String> {
    let short = |w: &str| w.chars().count() <= 2 && w.chars().all(char::is_alphabetic);
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let mut j = i;
        while j < words.len() && short(&words[j]) {
            j += 1;
        }
        let run = &words[i..j];
        let letters: usize = run.iter().map(|w| w.chars().count()).sum();
        let joins = letters >= 3
            && (run.len() >= 3 || (run.len() >= 2 && run.iter().any(|w| w.chars().count() == 1)));
        if joins {
            out.push(words[i..j].concat());
            i = j;
        } else {
            out.push(words[i].clone());
            i += 1;
        }
    }
    out
}

/// Longest word [`split_glued`] tries to split.
const MAX_GLUED: usize = 30;

/// A word that is two listed words glued together is split in two; other words stay.
fn split_glued(words: Vec<String>) -> Vec<String> {
    let listed = |w: &str| {
        UNDER_18
            .iter()
            .chain(SEXUAL)
            .filter(|p| !p.contains(' '))
            .any(|p| word_matches(w, p))
    };
    let mut out = Vec::with_capacity(words.len());
    for w in words {
        let chars: Vec<char> = w.chars().collect();
        // Two listed words are at most ~30 letters; longer words aren't split (keeps the check
        // fast on long made-up words).
        let split = ((6..=MAX_GLUED).contains(&chars.len()) && !listed(&w))
            .then(|| {
                (3..=chars.len() - 3).find_map(|k| {
                    let (a, b): (String, String) =
                        (chars[..k].iter().collect(), chars[k..].iter().collect());
                    (listed(&a) && listed(&b)).then_some((a, b))
                })
            })
            .flatten();
        match split {
            Some((a, b)) => {
                out.push(a);
                out.push(b);
            }
            None => out.push(w),
        }
    }
    out
}

/// Repeated letters as one ("looli" → "loli").
fn squeeze(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    for c in w.chars() {
        if !out.ends_with(c) {
            out.push(c);
        }
    }
    out
}

/// The same word, or the word with its letters stretched ("teeen"; never shorter, so "ten"
/// isn't "teen").
fn same_word(token: &str, word: &str) -> bool {
    token == word
        || (token.len() > word.len()
            && token.chars().next() == word.chars().next()
            && squeeze(token) == squeeze(word))
}

/// The word, stretched or not, with or without a plural "s"/"es". A stretched plural needs a
/// letter three times ("kiiids"), so ordinary double letters ("annals") stay apart.
fn word_matches(token: &str, word: &str) -> bool {
    same_word(token, word)
        || ["s", "es"].iter().any(|end| {
            token.strip_suffix(end).is_some_and(|stem| {
                stem == word || (has_triple_letter(stem) && same_word(stem, word))
            })
        })
}

fn has_triple_letter(w: &str) -> bool {
    let c: Vec<char> = w.chars().collect();
    c.windows(3).any(|t| t[0] == t[1] && t[1] == t[2])
}

/// The phrase word by word, or glued into one word ("littlegirls").
fn has_phrase(words: &[String], phrase: &str) -> bool {
    if phrase.contains(' ') {
        let glued = phrase.replace(' ', "");
        if words.iter().any(|w| word_matches(w, &glued)) {
            return true;
        }
    }
    let parts: Vec<&str> = phrase.split(' ').collect();
    let last = parts.len() - 1;
    words.windows(parts.len()).any(|win| {
        win.iter().zip(&parts).enumerate().all(|(i, (w, p))| {
            if i == last {
                word_matches(w, p)
            } else {
                same_word(w, p)
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
    fn blocks_spellings_that_hide_the_words() {
        let missed = [
            // Look-alike letters (Cyrillic о, Greek ο), accents, styled letters.
            "l\u{043E}li, nude",
            "l\u{03BF}li, nud\u{0435}",
            "lol\u{00ED}, n\u{00FA}de",
            "lolı, nude",
            "lo\u{0301}li, nude",
            "\u{1D425}\u{1D428}\u{1D425}\u{1D422}, nude",
            "\u{24DB}\u{24DE}\u{24DB}\u{24D8}, nude",
            // Invisible characters anywhere.
            "lo\u{2063}li, nu\u{034F}de",
            "sh\u{00AD}ota, n\u{FE0F}aked",
            // Numbers and symbols for letters.
            "l0l1, nude",
            "1o1i, nude",
            "l0li, s3xy",
            "sh0ta, n@k3d",
            "ch1ld, nud3",
            "t33n, p0rn",
            "k1ds, $exy",
            "l!ttle g!rl, nude",
            "lo|i, nude",
            // Spaced-out letters.
            "l o l i, nude",
            "l.o.l.i, n.u.d.e",
            "l-o-l-i, nude",
            "l_o_l_i, nude",
            "lo l i, nude",
            "c h i l d, n a k e d",
            // Repeated letters.
            "looooli, nude",
            "teeeen, nuuude",
            "sexxxy, kiiid",
            // Glued words.
            "nudeloli",
            "sexyteen",
            "lolinude, 1girl",
            "teenporn",
            // Capital look-alikes, small capitals, boxed, superscript and letterlike forms.
            "\u{0421}\u{041D}\u{0406}LD, \u{039D}UDE",
            "\u{029F}\u{1D0F}\u{029F}\u{026A}, \u{0274}\u{1D1C}\u{1D05}\u{1D07}",
            "\u{1F13B}\u{1F13E}\u{1F13B}\u{1F138}, nude",
            "\u{1F17B}\u{1F17E}\u{1F17B}\u{1F178}, nude",
            "\u{02E1}\u{1D52}\u{02E1}\u{2071}, nude",
            "\u{2113}oli, nude",
            "\u{FF4C}\u{FF4F}\u{FF4C}\u{FF49}, nude",
            // More invisible characters.
            "lo\u{2800}li, nu\u{FFF9}de",
            // Stretched plurals, glued phrases.
            "kiiids, nude",
            "littlegirls, nude",
            "youngboy, naked",
            // Mixed.
            "L\u{00D6}\u{00D6}L1, NS\u{0192}W, n\u{00FC}d3",
        ]
        .into_iter()
        .filter(|t| !pairs_minor_with_sexual(t))
        .collect::<Vec<_>>();
        assert!(missed.is_empty(), "not blocked: {missed:?}");
    }

    #[test]
    fn normalization_keeps_harmless_text_harmless() {
        let wrong = [
            "family photo at the beach, kids building a sandcastle",
            "kid-friendly cafe, cozy interior",
            "baby blue sports car, studio lighting",
            "18 year old whisky in a crystal glass",
            "18 year old whisky, naked flame of a candle",
            "a 21 year old woman, nude, oil painting",
            "an al fresco lunch with the kids, summer",
            "the annals of a small village school, children at desks",
            "Middlesex county fair, children on a carousel",
            "Sussex coast, kids flying kites",
            "fifteen minutes before sunset, a nude-colored dress on a mannequin",
            "canteen full of kids, school lunch",
            "4k photo of kids at a birthday party",
            "1girl, adult, nsfw, 3d render",
            "2 boys and a dog, 35mm film",
            "kidnapped princess tied to a tree, fairy tale illustration",
            "a toddler with a lollipop",
            "caf\u{00E9} with kids, jazz playing",
            "ten apples, nude still life",
            "b a b y shower decorations",
            "o k, kids, let's go",
            "lol! nude-colored heels, lol!!",
            "Kim's ex took the kids to the zoo",
            "Sm\u{00F6}rg\u{00E5}sbord for the kids' party",
            "\u{2116} 5 school bus, \u{2122} logo, \u{00BD} price",
        ]
        .into_iter()
        .filter(|t| pairs_minor_with_sexual(t))
        .collect::<Vec<_>>();
        assert!(wrong.is_empty(), "blocked: {wrong:?}");
    }

    #[test]
    fn long_made_up_words_stay_fast() {
        // Each of these took minutes before glued words were capped at MAX_GLUED letters.
        let start = std::time::Instant::now();
        assert!(!pairs_minor_with_sexual(&"xyzw".repeat(2500)));
        assert!(!pairs_minor_with_sexual(
            &vec!["ab".repeat(15); 500].join(" ")
        ));
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
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
    fn checked_prompts_only_come_from_the_check() {
        assert_eq!(
            CheckedPrompt::check("loli, nude, PINHOLE_SENTINEL_7f3a"),
            Err(Blocked)
        );
        let ok = CheckedPrompt::check("a nude woman, oil painting").unwrap();
        assert_eq!(ok.as_str(), "a nude woman, oil painting");
        assert!(!format!("{ok:?}").contains("woman"));
    }
}
