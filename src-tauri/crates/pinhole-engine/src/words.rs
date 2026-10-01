//! Local word check: text that pairs an under-18 term with a sexual term is blocked, and so is
//! text that names an identity document or banknote together with a word asking for a usable
//! copy of it ([`asks_for_document_copy`]), whatever Safe mode says. Runs on the prompt sent to the image engine in Create and every
//! Edit mode (after styles, trigger words and add-ons are combined), on the idea sent to
//! "Improve my prompt", on what the Describe model writes back (Describe and Improve) and on
//! Browse search text. See RELEASE-SPEC §11.
//!
//! Lives in the engine crate so the image engine's request type can require a checked prompt
//! (`CheckedPrompt`); `pinhole_core::text_check` wraps it with the app's error.
//!
//! Deliberately plain: fixed word lists compiled in (not YAML), whole words (plus common
//! endings and words glued to another word); both lists of a pair must match. Spellings are normalized
//! first (see [`pairs_minor_with_sexual`]). It is a first line; the §3 image check is the main
//! safeguard. PRIVACY: the text and which words matched are
//! never logged, stored or put in an error.

use unicode_normalization::UnicodeNormalization;

/// Message shown when the word check or the image check blocks a request. It includes no
/// details about the text. The UI shows the usage guidelines with it.
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

/// Tens words: a number right after one is part of an adult age ("twenty five").
const TENS: &[&str] = &[
    "twenty", "thirty", "forty", "fourty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

/// Sexual terms, same plural rule. Words with common non-sexual meanings are left out.
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

/// Identity and official documents. Apostrophes are dropped before matching, so "drivers"
/// also covers the possessive.
const DOCUMENT: &[&str] = &[
    "passport",
    "id card",
    "identity card",
    "identification card",
    "identity document",
    "national id",
    "state id",
    "voter id",
    "drivers license",
    "driver license",
    "driving license",
    "drivers licence",
    "driver licence",
    "driving licence",
    "residence permit",
    "work permit",
    "social security card",
    "green card",
    "birth certificate",
    "bank statement",
    "pay stub",
    "payslip",
    "utility bill",
];

/// Banknotes and cheques.
const MONEY: &[&str] = &[
    "banknote",
    "bank note",
    "dollar bill",
    "euro note",
    "euro bill",
    "pound note",
    "currency note",
    "paper money",
    "cheque",
    "bank check",
    "money order",
];

/// Words that ask for a document or banknote as a usable copy: its data fields, a flat scan
/// of it, or that it be taken as genuine or copied. A document or banknote as part of a
/// scene, or a made-up one, passes.
const COPY_OF: &[&str] = &[
    "date of birth",
    "dob",
    "birth date",
    "id number",
    "document number",
    "passport number",
    "license number",
    "licence number",
    "card number",
    "personal number",
    "account number",
    "routing number",
    "serial number",
    "expiry",
    "expiry date",
    "expiration date",
    "date of expiry",
    "date of issue",
    "issue date",
    "mrz",
    "machine readable",
    "barcode",
    "scan",
    "scanned",
    "photocopy",
    "front side",
    "back side",
    "front and back",
    "both sides",
    "flat lay",
    "flatlay",
    "printable",
    "print ready",
    "official",
    "valid",
    "authentic",
    "genuine",
    "government issued",
    "verification",
    "kyc",
    "fake",
    "forged",
    "forgery",
    "counterfeit",
    "replica",
    "template",
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

/// Blocks text that pairs an under-18 term with a sexual term, or that asks for a usable copy
/// of an identity document or banknote.
pub fn check(text: &str) -> Result<(), Blocked> {
    if pairs_minor_with_sexual(text) || asks_for_document_copy(text) {
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
/// are matched against every normalized spelling in [`views`].
pub fn pairs_minor_with_sexual(text: &str) -> bool {
    let views = views(text);
    let lists = lists();
    let any = |list: &List| mentions(&views, list);
    // Ages are read from the plain spellings only: numbers-for-letters would turn "18" into
    // something else.
    let minor = any(&lists.under_18) || AGE_VIEWS.iter().any(|&i| has_young_age(&views[i].words));
    minor && any(&lists.sexual)
}

/// True when `text` names an identity document or a banknote together with a word that asks
/// for a usable copy of it ([`COPY_OF`]). Read with the same spellings as
/// [`pairs_minor_with_sexual`].
pub fn asks_for_document_copy(text: &str) -> bool {
    let views = views(text);
    let lists = lists();
    let any = |list: &List| mentions(&views, list);
    (any(&lists.document) || any(&lists.money)) && any(&lists.copy_of)
}

/// Some spelling of the text has a word or phrase of `list`.
fn mentions(views: &[View], list: &List) -> bool {
    views.iter().any(|v| {
        v.words.iter().any(|w| list.has_word(w))
            || list.phrases.iter().any(|p| has_phrase(&v.words, p))
            || v.joined.iter().any(|w| list.is_listed(w))
    })
}

/// A word list split for speed: single words by their first and by their last letter, and the
/// multi-word phrases. Every form a single word matches in starts with its first letter or,
/// glued after another word, ends with its last letter (before a plural ending), so each word
/// of the text is only compared with the few listed words that share that letter. This keeps
/// long text fast.
struct List {
    first: Vec<Vec<&'static str>>,
    last: Vec<Vec<&'static str>>,
    phrases: Vec<&'static str>,
}

impl List {
    fn new(words: &[&'static str]) -> Self {
        let mut list = List {
            first: vec![Vec::new(); 256],
            last: vec![Vec::new(); 256],
            phrases: Vec::new(),
        };
        for &w in words {
            if w.contains(' ') {
                list.phrases.push(w);
            } else if let (Some(&f), Some(&l)) = (w.as_bytes().first(), w.as_bytes().last()) {
                list.first[usize::from(f)].push(w);
                list.last[usize::from(l)].push(w);
            }
        }
        list
    }

    fn starting_like(&self, token: &str) -> &[&'static str] {
        token
            .as_bytes()
            .first()
            .map_or(&[], |&b| &self.first[usize::from(b)])
    }

    /// The token is a listed single word, stretched or plural ([`word_matches`]).
    fn is_listed(&self, token: &str) -> bool {
        self.starting_like(token)
            .iter()
            .any(|p| word_matches(token, p))
    }

    /// The token is a listed single word, with a common ending or glued to another word.
    fn has_word(&self, token: &str) -> bool {
        let glued_after = || {
            [
                Some(token),
                token.strip_suffix('s'),
                token.strip_suffix("es"),
            ]
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_bytes().last())
            .any(|&b| {
                self.last[usize::from(b)]
                    .iter()
                    .any(|p| glued_to_filler(token, p))
            })
        };
        self.starting_like(token)
            .iter()
            .any(|p| word_matches(token, p) || word_form(token, p) || glued_to_filler(token, p))
            || glued_after()
    }
}

/// The word lists, split once.
struct Lists {
    under_18: List,
    sexual: List,
    both: List,
    document: List,
    money: List,
    copy_of: List,
}

fn lists() -> &'static Lists {
    static LISTS: std::sync::OnceLock<Lists> = std::sync::OnceLock::new();
    LISTS.get_or_init(|| Lists {
        under_18: List::new(UNDER_18),
        sexual: List::new(SEXUAL),
        both: List::new(&[UNDER_18, SEXUAL].concat()),
        document: List::new(DOCUMENT),
        money: List::new(MONEY),
        copy_of: List::new(COPY_OF),
    })
}

/// The text as several lists of lowercase words, all checked:
/// - as written, with invisible characters, accents and apostrophes dropped and compatibility
///   forms (fullwidth, superscript, squared, letterlike, ligatures) made plain;
/// - look-alike letters (Cyrillic and Greek capitals and small letters, small capitals, styled
///   and boxed letters) made Latin;
/// - twice more with numbers and symbols read as letters (`0`→o, `1`→i, then `1`→l, `3`→e,
///   `4`/`@`→a, `5`/`$`→s, `7`→t, `|`, and `!` except at the end
///   of a word, where it is punctuation), only in words that also have
///   letters.
///
/// In every list, spaced-out letters are joined ("l o l i", "l.o.l.i") and a word made of two
/// listed words glued together is split in two; a listed phrase also matches glued into one word.
/// Each view also keeps punctuated chunks joined whole ([`View::joined`]).
fn views(text: &str) -> Vec<View> {
    // Compatibility forms (fullwidth, superscript, squared, letterlike, ligatures…) become
    // plain letters; accents come apart from their letters and are dropped. Apostrophes are
    // dropped so "Kim's ex" reads "kims ex", not "s ex". Fractions stay whole ("18½" isn't
    // "181 2"), and a character that would expand to more than 4 is a space too (keeps long
    // input fast).
    let plain: String = text
        .chars()
        .flat_map(|c| -> Vec<char> {
            match c {
                _ if c.is_ascii() => vec![c],
                '\u{00BC}'..='\u{00BE}' | '\u{2044}' | '\u{2150}'..='\u{215F}' | '\u{2189}' => {
                    vec![c]
                }
                'ŀ' | 'Ŀ' => vec!['l'],
                // Digits from other scripts.
                '\u{0660}'..='\u{0669}' => vec![char::from(b'0' + (c as u32 - 0x0660) as u8)],
                '\u{06F0}'..='\u{06F9}' => vec![char::from(b'0' + (c as u32 - 0x06F0) as u8)],
                '\u{0966}'..='\u{096F}' => vec![char::from(b'0' + (c as u32 - 0x0966) as u8)],
                _ => {
                    let d: Vec<char> = std::iter::once(c).nfkd().collect();
                    if d.len() > 4 {
                        vec![' ']
                    } else {
                        d
                    }
                }
            }
        })
        .filter(|c| !is_invisible(*c) && !is_combining_mark(*c) && !is_apostrophe(*c))
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
    // Words are split on spaces and punctuation. A chunk with punctuation inside it
    // ("lo-li", "l.o.l.i") is also kept joined, right after its pieces. Every such chunk of up
    // to MAX_GLUED letters, and every two neighbouring pieces, are also kept joined in `joined`,
    // whatever the pieces' lengths, and only matched whole against single listed words.
    let words = |text: &str, keep: &dyn Fn(char) -> bool| -> View {
        let mut out = Vec::new();
        let mut joined = Vec::new();
        for chunk in text.split(char::is_whitespace) {
            let pieces: Vec<&str> = chunk
                .split(|c: char| !keep(c))
                .filter(|w| !w.is_empty())
                .collect();
            out.extend(pieces.iter().map(|w| w.to_string()));
            let letters = |ps: &[&str]| ps.iter().map(|p| p.chars().count()).sum::<usize>();
            if pieces.len() > 2 && letters(&pieces) <= MAX_GLUED {
                joined.push(pieces.concat());
            }
            // Two neighbouring pieces too ("lovely-te-en").
            for pair in pieces.windows(2) {
                if letters(pair) <= MAX_GLUED {
                    joined.push(pair.concat());
                }
            }
            // Only short pieces ("lo-li", "chi.ld"): longer ones are words of their own, and
            // joining them would glue neighbouring tags ("nude,eighteen"). Single letters
            // ("y/o", "l.o.l.i") are left to `join_pieces`.
            let len = |p: &&str| p.chars().count();
            if pieces.len() > 1
                && pieces.iter().any(|p| len(p) > 1)
                && pieces.iter().all(|p| len(p) <= 4)
            {
                out.push(pieces.concat());
            }
        }
        View { words: out, joined }
    };
    let alnum = |c: char| c.is_alphanumeric();
    let leet_chars = |c: char| c.is_alphanumeric() || matches!(c, '@' | '$' | '!' | '|');
    let leet_words = words(&folded, &leet_chars);
    let leet_view = |one: char| View {
        words: leet_words.words.iter().map(|w| leet(w, one)).collect(),
        joined: leet_words.joined.iter().map(|w| leet(w, one)).collect(),
    };
    let raw = [
        words(&clean, &alnum),
        words(&folded, &alnum),
        leet_view('i'),
        leet_view('l'),
        words(&split_case(&plain), &alnum),
    ];
    raw.into_iter()
        .map(|v| View {
            words: split_glued(join_pieces(without_naked_eye(v.words))),
            joined: v.joined,
        })
        .collect()
}

/// One spelling of the text (see [`views`]).
struct View {
    /// The words, matched against every list entry, ending and glued form.
    words: Vec<String>,
    /// Punctuated chunks, and neighbouring pieces in them, joined ("nake-d"), matched only as
    /// single listed words: joining whole tags ("adult,nsfw,canteen") must not make glued words.
    joined: Vec<String>,
}

/// Views ages are read from: as written, folded, and split at case and digit changes (not the
/// numbers-for-letters views, which would turn "18" into something else).
const AGE_VIEWS: [usize; 3] = [0, 1, 4];

/// The folded text with a space wherever a lowercase letter meets a capital or a letter meets a
/// digit ("LittleGirl" → "little girl", "12years" → "12 years").
fn split_case(plain: &str) -> String {
    let mut out = String::with_capacity(plain.len() + 8);
    let mut prev: Option<char> = None;
    for c in plain.chars() {
        if let Some(p) = prev {
            let case = p.is_lowercase() && c.is_uppercase();
            let digit = (p.is_ascii_digit() && c.is_alphabetic())
                || (p.is_alphabetic() && c.is_ascii_digit());
            if case || digit {
                out.push(' ');
            }
        }
        out.extend(match fold_capital(c) {
            Some(l) => vec![l],
            None => c.to_lowercase().flat_map(fold).collect::<Vec<_>>(),
        });
        prev = Some(c);
    }
    out
}

/// Characters that show nothing: soft hyphen, zero-width and joiner characters, direction
/// marks, variation selectors, Hangul fillers, blank Braille, tag characters and every other
/// format character (General_Category Cf).
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{0890}'..='\u{0891}' | '\u{08E2}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{2800}' | '\u{3164}' | '\u{FE00}'..='\u{FE0F}'
        | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFB}' | '\u{13430}'..='\u{1343F}'
        | '\u{110BD}' | '\u{110CD}' | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0000}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}')
}

/// Every combining mark (General_Category M: accents left over once letters are taken apart,
/// enclosing marks, vowel signs and other marks of any script).
fn is_combining_mark(c: char) -> bool {
    unicode_normalization::char::is_combining_mark(c)
}

fn is_apostrophe(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2018}' | '\u{2019}' | '\u{02BB}' | '\u{02BC}' | '\u{2032}' | '`' | '\u{00B4}'
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
    if c.is_ascii() {
        return None;
    }
    CAPITALS
        .iter()
        .find(|(from, _)| from.contains(c))
        .map(|(_, to)| *to)
}

/// One lowercase character without its accent, or its Latin look-alike. Most accented and
/// styled letters are already plain after NFKD in [`views`]; the tables stay as a fallback.
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
        // No table has an ASCII character.
        _ if c.is_ascii() => return vec![c],
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
    // A trailing "!" is punctuation ("lol!" stays "lol"); elsewhere it is a letter.
    word.trim_end_matches('!')
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
    let listed = |w: &str| lists().both.is_listed(w);
    let mut out = Vec::with_capacity(words.len());
    for w in words {
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).collect();
        let n = cuts.len();
        // Two listed words are at most ~30 letters; longer words aren't split (keeps the check
        // fast on long made-up words).
        let split = ((6..=MAX_GLUED).contains(&n) && !listed(&w))
            .then(|| {
                (3..=n - 3).find_map(|k| {
                    let (a, b) = w.split_at(cuts[k]);
                    (listed(a) && listed(b)).then(|| (a.to_string(), b.to_string()))
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

/// The letters with repeats as one ("looli" → "loli").
fn squeezed(w: &str) -> impl Iterator<Item = char> + '_ {
    let mut prev = None;
    w.chars().filter(move |&c| prev.replace(c) != Some(c))
}

/// The same word, or the word with its letters stretched ("teeen"; never shorter, so "ten"
/// isn't "teen").
fn same_word(token: &str, word: &str) -> bool {
    token == word
        || (token.len() > word.len()
            && token.chars().next() == word.chars().next()
            && squeezed(token).eq(squeezed(word)))
}

/// The word, stretched or not, with or without a plural "s"/"es". A stretched plural needs a
/// letter three times ("kiiids"), so ordinary double letters ("annals") stay apart.
fn word_matches(token: &str, word: &str) -> bool {
    // Every form below starts with the word's first letter; most tokens stop here.
    if token.as_bytes().first() != word.as_bytes().first() {
        return false;
    }
    let ies = || {
        word.strip_suffix('y')
            .is_some_and(|stem| token.strip_suffix("ies") == Some(stem))
    };
    same_word(token, word)
        || ies()
        || ["s", "es"].iter().any(|end| {
            token.strip_suffix(end).is_some_and(|stem| {
                stem == word || (has_triple_letter(stem) && same_word(stem, word))
            })
        })
}

fn has_triple_letter(w: &str) -> bool {
    let (mut prev, mut run) = (None, 0);
    w.chars().any(|c| {
        run = if prev.replace(c) == Some(c) {
            run + 1
        } else {
            1
        };
        run >= 3
    })
}

/// Ordinary words that start or end with a listed word ([`glued_to_filler`] skips them).
const ORDINARY: &[&str] = &[
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "canteen",
    "velveteen",
    "umpteen",
    "inbetween",
    "childhood",
    "childbirth",
    "childcare",
    "childless",
    "childproof",
    "childbearing",
    "childminder",
    "infantry",
    "infantile",
    "infantryman",
    "analysis",
    "analyses",
    "analyst",
    "analysts",
    "analog",
    "analogue",
    "analogous",
    "analytic",
    "analytics",
    "analytical",
    "analgesic",
    "analyze",
    "analyse",
    "analyzed",
    "analysed",
    "minority",
    "minorities",
    "rapeseed",
    "lolipop",
    "boobytrap",
    "boobytrapped",
    "pantyhose",
    "breaststroke",
    "congenital",
    "homosexual",
    "heterosexual",
    "bisexual",
    "asexual",
    "transsexual",
    "pansexual",
    "intersexual",
    "nonsexual",
    "metrosexual",
    "gobetween",
    "brainchild",
    "outstripping",
    "redbreast",
    "pussycat",
    "pussywillow",
];

/// A listed word glued to another word of 3+ letters ("xxxteen", "teenxxxs"). Under-18 words
/// of 4+ letters count; sexual words only from 5 letters, since the short ones start or end
/// many ordinary words and names.
fn glued_to_filler(token: &str, word: &str) -> bool {
    // Cheapest tests first: this runs for every word against every listed word.
    let ends = |cut: usize| {
        token
            .len()
            .checked_sub(cut)
            .and_then(|n| token.get(..n))
            .is_some_and(|t| t.ends_with(word))
    };
    if token.len() < word.len() + 3 || !(token.starts_with(word) || ends(0) || ends(1) || ends(2)) {
        return false;
    }
    let w = word.chars().count();
    let min = if UNDER_18.contains(&word) { 4 } else { 5 };
    if w < min
        || ORDINARY
            .iter()
            .any(|o| token == *o || token.strip_suffix('s') == Some(o))
    {
        return false;
    }
    [
        Some(token),
        token.strip_suffix('s'),
        token.strip_suffix("es"),
    ]
    .into_iter()
    .flatten()
    .any(|t| {
        let n = t.chars().count();
        n >= w + 3 && n <= MAX_GLUED && (t.starts_with(word) || t.ends_with(word))
    })
}

/// The word with a common ending ("sexier", "sexiest", "nudeness", "lewdly"); only for
/// words of 4+ letters, so short words don't catch ordinary ones ("kidding"). "explicitly"
/// stays harmless ("explicitly labelled").
fn word_form(token: &str, word: &str) -> bool {
    let Some(last) = word.chars().last() else {
        return false;
    };
    // Every stem (the word, "y" → "i", without a silent "e") starts with all but the word's
    // last letter, so most words are ruled out here without building anything.
    let Some(rest) = token.strip_prefix(&word[..word.len() - last.len_utf8()]) else {
        return false;
    };
    if word.chars().count() < 4 || token == "explicitly" {
        return false;
    }
    let ending = |end: &str| matches!(end, "er" | "est" | "ness" | "ed" | "d" | "ly" | "ily");
    rest.strip_prefix(last).is_some_and(ending)
        || (last == 'y' && rest.strip_prefix('i').is_some_and(ending))
        || (last == 'e' && ending(rest))
}

/// A multi-word phrase word by word, or glued into one word ("littlegirls").
fn has_phrase(words: &[String], phrase: &str) -> bool {
    let glued = phrase.replace(' ', "");
    if words.iter().any(|w| word_matches(w, &glued)) {
        return true;
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

/// "yearold", "yearsold", "yrsold", "yold", "yearsofage" written as one word after a number.
fn glued_old(rest: &str) -> bool {
    rest.strip_suffix("olds")
        .or_else(|| rest.strip_suffix("old"))
        .or_else(|| rest.strip_suffix("ofage"))
        .is_some_and(|y| matches!(y, "y" | "yr" | "yrs" | "year" | "years"))
}

/// An age from 1 to 17: "12", "twelve" or "12yo" followed by "yo", "y o", "year old",
/// "years old", "year olds", "yr old", "yrs old" or "years of age"; or "aged 12", "age of 12".
/// A bare "age 12" doesn't count ("Bronze Age, 3 statues"). "year", "years", "yr", "yrs" or "y"
/// may also be glued to "old", "olds" or "ofage" ("12 yearsold"). A number right after a tens
/// word is part of an adult age ("twenty-five years old") and doesn't count; any other word after
/// a tens word ("twenty 12 yo", "thirty twelve yo") still does.
fn has_young_age(words: &[String]) -> bool {
    let young =
        |w: &str| w.parse::<u32>().is_ok_and(|n| (1..18).contains(&n)) || NUMBER_WORDS.contains(&w);
    let at = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    let old_after = |i: usize| {
        matches!(at(i), "old" | "olds" | "o" | "ofage") || (at(i) == "of" && at(i + 1) == "age")
    };
    for (i, w) in words.iter().enumerate() {
        // Only a spelled unit after a tens word ("twenty five") is part of an adult age.
        const UNITS: [&str; 9] = [
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        ];
        if i > 0
            && TENS.contains(&at(i - 1))
            && UNITS
                .iter()
                .any(|u| w.strip_prefix(u).is_some_and(|r| !r.starts_with("teen")))
        {
            continue;
        }
        // A number word glued to "yo" or "years": "twelveyo", "twelveyears old".
        for n in NUMBER_WORDS {
            if let Some(rest) = w.strip_prefix(n) {
                if matches!(rest, "yo" | "yos") || glued_old(rest) {
                    return true;
                }
                if matches!(rest, "y" | "yr" | "yrs" | "year" | "years") && old_after(i + 1) {
                    return true;
                }
            }
        }
        // Joined forms: "12yo", "12yr", "12yrs", "12y".
        let digits: String = w.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && young(&digits) {
            let rest = &w[digits.len()..];
            if rest == "yo" || rest == "yos" || glued_old(rest) {
                return true;
            }
            if matches!(rest, "y" | "yr" | "yrs" | "year" | "years") && old_after(i + 1) {
                return true;
            }
        }
        if young(w) {
            let next = at(i + 1);
            if next == "yo" || next == "yos" || glued_old(next) {
                return true;
            }
            if matches!(next, "year" | "years" | "yr" | "yrs" | "y") && old_after(i + 2) {
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
    fn blocks_usable_copies_of_documents_and_banknotes() {
        for t in [
            "passport, date of birth, document number, scanned",
            "a valid driver's license from Ohio",
            "Drivers-License, front and back",
            "id card template",
            "100 dollar bill, serial number, flat lay",
            "a counterfeit banknote",
            "bank statement, account number",
        ] {
            assert!(asks_for_document_copy(t), "{t}");
            assert_eq!(check(t), Err(Blocked), "{t}");
        }
    }

    #[test]
    fn documents_and_money_in_a_scene_pass() {
        for t in [
            "a tourist holding a passport at the airport, photo",
            "passport photo of a smiling man, studio lighting",
            "a letter on a desk, candlelight",
            "an old map of the city, scanned",
            "movie prop money on a table",
            "a pile of dollar bills, cinematic",
            "a banknote with a cat portrait, engraving style",
            "official portrait of a knight",
            "a scanned watercolor of a valid argument",
        ] {
            assert!(!asks_for_document_copy(t), "{t}");
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
    fn blocks_normalized_spellings() {
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
            // Glued to another word, CamelCase, split by punctuation, other word forms.
            "cuteteen, nude",
            "teenmodel, nakedbeach",
            "LittleGirl, Nude",
            "CuteTeen NudeArt",
            "cuteteens, nude",
            "12yearold, nude",
            "twelveyearsold, nude",
            "lo-li, nu-de",
            "te.en, na.ked",
            "teen, sexier",
            "teen, nakedness",
            // Ages written together or with other digits.
            "12years old, nude",
            "12year-old, nude",
            "twelveyo, nude",
            "12 yos, nude",
            "\u{0661}\u{0662} year old, nude",
            // A "!" leading a word is a letter; a middle-dot "l".
            "!nfant, nude",
            "lo\u{0140}i, nude",
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
            "Sm\u{00F6}rg\u{00E5}sbord, nude lipstick",
            "\u{2116} 5 school bus, \u{2122} logo, \u{00BD} price, nude lipstick",
            "18\u{00BD} year old whisky, naked flame of a candle",
            "cheese aged 1\u{00BD} years, naked flame",
            "\u{00BD} year old cheese, naked flame",
            "eighteen year old whisky, nude lipstick, 18years old",
            "the canteen analysis of a minority, nude colors",
            "congenital heart study, sexual health leaflet for adults",
            "homosexual couple, adults, nude painting",
            "12 years later, a naked tree in winter",
            "rapeseed field at noon, sensual light",
            "1girl,nude,eighteen",
            "adult,nsfw,canteen",
            "woman,nude,in-between",
            "childhood-friend reunion, adults, nude beach",
            "analog-style photo of kids playing in a park",
            "analyzing kids drawings",
            "The Owl and the Pussycat, children's book illustration",
            "children explicitly labelled diagram of the water cycle",
            "a go-between, nude colors",
        ]
        .into_iter()
        .filter(|t| pairs_minor_with_sexual(t))
        .collect::<Vec<_>>();
        assert!(wrong.is_empty(), "blocked: {wrong:?}");
    }

    #[test]
    fn blocks_more_marks_splits_ages_and_word_forms() {
        let missed = [
            // Combining marks of any script, and format characters, inside a word.
            "te\u{064B}en, nude",
            "l\u{0F7A}o\u{0F7A}l\u{0F7A}i, nude",
            "lo\u{10A0D}li, nude",
            "lo\u{0483}li, nude",
            "te\u{0591}en, nu\u{0951}de",
            "ki\u{0E48}d, na\u{302A}ked",
            "te\u{1D167}en, nude",
            "lo\u{0600}li, nu\u{110BD}de",
            "te\u{06DD}en, nude",
            // Punctuated chunks with a long piece.
            "teenage-r, nude",
            "kid, nake-d",
            "toddle.r, nude",
            "teen, lingeri_e",
            "lovely-te-en, nude",
            "kinder-garte-n, nude",
            "teen\u{2027}ager, nude",
            "teen\u{00B7}ager, nude",
            // Ages with "year" glued to "old".
            "12 yearsold, nude",
            "12 yearold, nude",
            "12 yrsold, nude",
            "twelve yearold, nude",
            "twelve yearsold, nude",
            "12 yearolds, nude",
            "12 y old, nude",
            "12 yold, nude",
            "12 years ofage, nude",
            "12yearsofage, nude",
            "12 YearsOld, nude",
            // -ly, -ily and -ies forms.
            "teen, lewdly",
            "kid, nakedly",
            "teen, sexily",
            "child, seductively",
            "teen, sensually",
            "teen, hornies",
            // A tens word only excuses a spelled unit right after it.
            "twenty 12 yo, nude",
            "thirty twelve years old, nude",
            "forty seventeen yo, nude",
        ]
        .into_iter()
        .filter(|t| !pairs_minor_with_sexual(t))
        .collect::<Vec<_>>();
        assert!(missed.is_empty(), "not blocked: {missed:?}");
    }

    #[test]
    fn adult_ages_and_ordinary_words_stay_harmless() {
        let wrong = [
            "infantile humor, nude colors",
            "twenty-five years old woman, nude portrait",
            "twenty five year old woman, nude, oil painting",
            "thirty-one year old woman, art nude",
            "forty-two yo woman, nude marble",
            "sixty five yearsold, nude lipstick",
            "seventy-two years of age, naked flame",
            "ninety-nine yearold oak tree, nude colors",
            "kids, an explicitly marked exit",
            "children explicitly labelled diagram of the water cycle",
            "1girl,nude,eighteen",
            "adult,nsfw,canteen",
            "woman,nude,in-between,adult-only",
            "the canteen-analysis of a minority-report, nude colors",
            "nude-colored heels, sex-ed leaflet for adults",
        ]
        .into_iter()
        .filter(|t| pairs_minor_with_sexual(t))
        .collect::<Vec<_>>();
        assert!(wrong.is_empty(), "blocked: {wrong:?}");
    }

    #[test]
    fn long_prompts_stay_fast() {
        let text = "a quiet-harbour at.dusk, soft_light, lo\u{064B}ng shadows, "
            .repeat(400)
            .chars()
            .take(20_000)
            .collect::<String>();
        let start = std::time::Instant::now();
        assert!(!pairs_minor_with_sexual(&text));
        // Generous for debug builds; a release build takes about 15 ms (keep it well under 50).
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn characters_that_expand_a_lot_stay_fast() {
        let text = "\u{FDFA}".repeat(10_000) + " kids, nude";
        let start = std::time::Instant::now();
        assert!(pairs_minor_with_sexual(&text));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
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
        // "kid" in "kid-friendly" is a whole word, so this is blocked (known false positive).
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
