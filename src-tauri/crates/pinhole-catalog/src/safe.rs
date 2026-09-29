//! Safe mode (SPEC §5.4): which CivitAI models count as made for adults, and
//! which sample image may be a card preview. Rules come from
//! `catalog-filters.yaml → safe_filter`; they were tuned on live
//! `/api/v1/models` answers (2026-09-28, see `tests/fixtures/live_*.json`).
//!
//! CivitAI's `nsfw=false` only hides models *flagged* NSFW, and the model-level
//! `nsfwLevel` is a bitmask of every rating its images have (1 PG, 2 PG-13,
//! 4 R, 8 X, 16 XXX, 32 blocked): mainstream Juggernaut XL is 31, exactly like
//! the suggestive anime merges. So a model is judged on several signals:
//! the NSFW flag, tags, words in its name, and the share of its creator's
//! sample images that are rated R or above.

use serde::Deserialize;

use crate::api::{Model, ModelImage};

/// PG bit of `nsfwLevel`.
pub const LEVEL_PG: u32 = 1;
/// PG-13: the highest preview level Safe mode may ever use.
pub const LEVEL_PG13: u32 = 2;
/// R: images at this level or above count as mature.
pub const LEVEL_R: u32 = 4;
/// Blocked by CivitAI's moderators: never a card preview.
pub const LEVEL_BLOCKED: u32 = 32;

/// `safe_filter` in `catalog-filters.yaml`. Lists are lowercased on load.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SafeFilter {
    /// Any of these tags hides the model.
    pub hide_tags: Vec<String>,
    /// `suggestive_tags_to_hide` or more of these hide the model.
    pub suggestive_tags: Vec<String>,
    pub suggestive_tags_to_hide: usize,
    /// Whole words (or phrases of whole words) in the model name.
    pub hide_name_words: Vec<String>,
    /// Hide when more than this share of rated sample images is R or above.
    pub max_mature_image_share: f32,
    /// …counted only with at least this many rated images.
    pub min_rated_images: usize,
    /// Highest image rating a Safe preview may have (1 = PG).
    pub max_preview_level: u32,
}

impl Default for SafeFilter {
    fn default() -> Self {
        Self {
            hide_tags: Vec::new(),
            suggestive_tags: Vec::new(),
            suggestive_tags_to_hide: 2,
            hide_name_words: Vec::new(),
            max_mature_image_share: 0.5,
            min_rated_images: 5,
            max_preview_level: LEVEL_PG,
        }
    }
}

/// Why Safe mode hides a model (for tests and hidden counts).
#[derive(Debug, Clone, PartialEq)]
pub enum AdultReason {
    /// CivitAI flags the model NSFW.
    MarkedNsfw,
    /// Its `nsfwLevel` has no PG bit: nothing about it is safe.
    NoSafeContent,
    Tag(String),
    SuggestiveTags(Vec<String>),
    NameWord(String),
    /// Share of rated sample images that are R or above.
    MatureImages(f32),
}

/// Rated sample images of a model (all versions): `(mature, rated)`.
pub fn image_ratings(m: &Model) -> (usize, usize) {
    let mut mature = 0;
    let mut rated = 0;
    for level in m
        .model_versions
        .iter()
        .flat_map(|v| v.images.iter())
        .filter_map(|i| i.nsfw_level)
        .filter(|l| *l > 0)
    {
        rated += 1;
        if level >= LEVEL_R {
            mature += 1;
        }
    }
    (mature, rated)
}

/// Lowercase words of a name: split on anything that isn't a letter or digit
/// (`REED_XXX_SDXL` → `reed`, `xxx`, `sdxl`; `XXMix` stays one word).
pub fn name_words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn normalize(list: &mut [String]) {
    for s in list.iter_mut() {
        *s = s.trim().to_lowercase();
    }
}

impl SafeFilter {
    /// Lowercase the lists and clamp the numbers to safe ranges: the YAML may
    /// tune Safe mode, never switch it off or allow previews above PG-13.
    pub fn normalized(mut self) -> Self {
        normalize(&mut self.hide_tags);
        normalize(&mut self.suggestive_tags);
        normalize(&mut self.hide_name_words);
        self.hide_tags.retain(|t| !t.is_empty());
        self.suggestive_tags.retain(|t| !t.is_empty());
        self.hide_name_words.retain(|t| !name_words(t).is_empty());
        self.suggestive_tags_to_hide = self.suggestive_tags_to_hide.max(1);
        if !self.max_mature_image_share.is_finite() {
            self.max_mature_image_share = Self::default().max_mature_image_share;
        }
        self.max_mature_image_share = self.max_mature_image_share.clamp(0.0, 1.0);
        self.min_rated_images = self.min_rated_images.max(1);
        self.max_preview_level = self.max_preview_level.clamp(LEVEL_PG, LEVEL_PG13);
        self
    }

    /// `Some(reason)` when Safe mode hides this model (and the NSFW tag finds it).
    pub fn adult_reason(&self, m: &Model) -> Option<AdultReason> {
        if m.nsfw {
            return Some(AdultReason::MarkedNsfw);
        }
        if m.nsfw_level.is_some_and(|l| l > 0 && l & LEVEL_PG == 0) {
            return Some(AdultReason::NoSafeContent);
        }
        let tags: Vec<String> = m.tags.iter().map(|t| t.trim().to_lowercase()).collect();
        if let Some(t) = tags.iter().find(|t| self.hide_tags.contains(t)) {
            return Some(AdultReason::Tag(t.clone()));
        }
        let mut suggestive: Vec<String> = tags
            .iter()
            .filter(|t| self.suggestive_tags.contains(t))
            .cloned()
            .collect();
        suggestive.dedup();
        if suggestive.len() >= self.suggestive_tags_to_hide {
            return Some(AdultReason::SuggestiveTags(suggestive));
        }
        let words = name_words(&m.name);
        for phrase in &self.hide_name_words {
            let needle = name_words(phrase);
            if words.windows(needle.len()).any(|w| w == needle.as_slice()) {
                return Some(AdultReason::NameWord(phrase.clone()));
            }
        }
        let (mature, rated) = image_ratings(m);
        if rated >= self.min_rated_images {
            let share = mature as f32 / rated as f32;
            if share > self.max_mature_image_share {
                return Some(AdultReason::MatureImages(share));
            }
        }
        None
    }

    /// May this image be a card preview with Safe mode on? Rated at most
    /// `max_preview_level`; unrated images only when the (older) `nsfw` flag
    /// explicitly says "not NSFW".
    pub fn is_safe_preview(&self, img: &ModelImage) -> bool {
        match img.nsfw_level {
            Some(level) if level > 0 => level <= self.max_preview_level,
            _ => img.nsfw == Some(false),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::ModelsPage;
    use crate::filters::tests::filters;

    pub(crate) fn live(name: &str) -> ModelsPage {
        let text = match name {
            "month" => include_str!("../tests/fixtures/live_month_top_rated.json"),
            "alltime" => include_str!("../tests/fixtures/live_alltime_most_downloaded.json"),
            "loras" => include_str!("../tests/fixtures/live_loras_alltime_most_downloaded.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    fn by_name<'a>(page: &'a ModelsPage, name: &str) -> &'a Model {
        page.items
            .iter()
            .find(|m| m.name.starts_with(name))
            .unwrap_or_else(|| panic!("{name} not in fixture"))
    }

    fn model(name: &str, tags: &[&str], levels: &[u32]) -> Model {
        let images: Vec<serde_json::Value> =
            levels.iter().enumerate().map(|(i, l)| serde_json::json!({ "url": format!("https://image.civitai.com/x/{i}.jpeg"), "nsfwLevel": l })).collect();
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": name, "type": "Checkpoint", "nsfw": false, "nsfwLevel": 31, "tags": tags,
            "modelVersions": [{ "id": 10, "baseModel": "SDXL 1.0", "images": images }]
        }))
        .unwrap()
    }

    #[test]
    fn shipped_yaml_loads_and_is_normalized() {
        let s = &filters().safe;
        assert!(s.hide_tags.contains(&"porn".to_string()));
        assert!(s.hide_tags.contains(&"large breasts".to_string()));
        assert!(s.suggestive_tags.contains(&"nsfw".to_string()));
        assert!(s.hide_name_words.contains(&"milk factory".to_string()));
        assert_eq!(s.suggestive_tags_to_hide, 2);
        assert_eq!(s.max_preview_level, LEVEL_PG);
        assert!((s.max_mature_image_share - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn yaml_can_tune_but_not_disable() {
        let s: SafeFilter = serde_yaml::from_str(
            "hide_tags: [' PORN ']\nsuggestive_tags_to_hide: 0\nmax_mature_image_share: 7\nmin_rated_images: 0\nmax_preview_level: 16",
        )
        .unwrap();
        let s = s.normalized();
        assert_eq!(s.hide_tags, ["porn"]);
        assert_eq!(s.suggestive_tags_to_hide, 1);
        assert_eq!(s.max_mature_image_share, 1.0);
        assert_eq!(s.min_rated_images, 1);
        assert_eq!(
            s.max_preview_level, LEVEL_PG13,
            "previews never above PG-13 in Safe"
        );
        // Even an empty filter keeps the hard rules.
        let empty = SafeFilter {
            max_mature_image_share: 1.0,
            ..Default::default()
        }
        .normalized();
        let mut m = model("Plain", &[], &[1]);
        assert_eq!(empty.adult_reason(&m), None);
        m.nsfw = true;
        assert_eq!(empty.adult_reason(&m), Some(AdultReason::MarkedNsfw));
        m.nsfw = false;
        m.nsfw_level = Some(28);
        assert_eq!(empty.adult_reason(&m), Some(AdultReason::NoSafeContent));
    }

    #[test]
    fn name_words_are_whole_words() {
        assert_eq!(
            name_words("REED_XXX_illustrious_SDXL"),
            ["reed", "xxx", "illustrious", "sdxl"]
        );
        assert_eq!(
            name_words("iNiverse Mix(SFW & NSFW)"),
            ["iniverse", "mix", "sfw", "nsfw"]
        );
        assert_eq!(name_words("XXMix_9realistic"), ["xxmix", "9realistic"]);
        assert_eq!(
            name_words("Vixon’s Milk Factory"),
            ["vixon", "s", "milk", "factory"]
        );
        let s = &filters().safe;
        let hide = |n: &str| s.adult_reason(&model(n, &[], &[1]));
        assert_eq!(
            hide("REED_XXX_illustrious_SDXL"),
            Some(AdultReason::NameWord("xxx".into()))
        );
        assert_eq!(
            hide("Vixon’s Milk Factory"),
            Some(AdultReason::NameWord("milk factory".into()))
        );
        assert_eq!(
            hide("MiaoMiao Harem"),
            Some(AdultReason::NameWord("harem".into()))
        );
        assert_eq!(hide("XXMix_9realistic"), None, "not a whole word");
        assert_eq!(
            hide("Plant Milk 🌿 - Model Suite"),
            None,
            "phrase needs both words"
        );
        assert_eq!(hide("Essex Landscapes"), None);
    }

    #[test]
    fn tags_hide_strongly_or_in_pairs() {
        let s = &filters().safe;
        assert_eq!(
            s.adult_reason(&model("A", &["anime", "Hentai"], &[1])),
            Some(AdultReason::Tag("hentai".into()))
        );
        assert_eq!(
            s.adult_reason(&model("B", &["photorealistic", "nsfw"], &[1])),
            None,
            "Juggernaut XL: one suggestive tag"
        );
        assert_eq!(
            s.adult_reason(&model("C", &["sexy", "nsfw", "woman"], &[1])),
            Some(AdultReason::SuggestiveTags(vec![
                "sexy".into(),
                "nsfw".into()
            ]))
        );
    }

    #[test]
    fn mature_image_share() {
        let s = &filters().safe;
        // 3 of 5 rated images R or above → 0.6 > 0.5.
        assert_eq!(
            s.adult_reason(&model("D", &[], &[1, 2, 4, 8, 16])),
            Some(AdultReason::MatureImages(0.6))
        );
        assert_eq!(
            s.adult_reason(&model("E", &[], &[1, 1, 2, 4, 8, 16])),
            None,
            "exactly half is fine"
        );
        assert_eq!(
            s.adult_reason(&model("F", &[], &[4, 8, 16, 0])),
            None,
            "too few rated images to judge"
        );
        assert_eq!(
            image_ratings(&model("G", &[], &[1, 32, 0, 4])),
            (2, 3),
            "blocked counts as mature, unrated is skipped"
        );
    }

    #[test]
    fn previews() {
        let s = &filters().safe;
        let img = |json: &str| serde_json::from_str::<ModelImage>(json).unwrap();
        assert!(s.is_safe_preview(&img(r#"{"url":"u","nsfwLevel":1}"#)));
        assert!(
            !s.is_safe_preview(&img(r#"{"url":"u","nsfwLevel":2}"#)),
            "PG only, like Stability Matrix"
        );
        assert!(!s.is_safe_preview(&img(r#"{"url":"u"}"#)), "unrated");
        assert!(
            s.is_safe_preview(&img(r#"{"url":"u","nsfw":"None"}"#)),
            "older answers: explicit not-NSFW"
        );
        assert!(!s.is_safe_preview(&img(r#"{"url":"u","nsfw":true}"#)));
    }

    /// Live checkpoint sample: "This month · Top rated", the page users used to see first.
    #[test]
    fn live_month_sample() {
        let s = &filters().safe;
        let page = live("month");
        let reason = |n: &str| s.adult_reason(by_name(&page, n));
        assert_eq!(reason("Babes"), Some(AdultReason::Tag("bimbo".into())));
        assert_eq!(
            reason("MiaoMiao Harem"),
            Some(AdultReason::NameWord("harem".into()))
        );
        assert_eq!(
            reason("REED_XXX"),
            Some(AdultReason::NameWord("xxx".into()))
        );
        assert_eq!(
            reason("Vixon’s Milk Factory"),
            Some(AdultReason::NameWord("milk factory".into()))
        );
        assert_eq!(
            reason("Moody Krea 2 Mix (uncensored)"),
            Some(AdultReason::NameWord("uncensored".into()))
        );
        assert_eq!(
            reason("Unholy Desire Mix"),
            Some(AdultReason::Tag("porn".into()))
        );
        assert_eq!(
            reason("Five Stars Illustrious"),
            Some(AdultReason::SuggestiveTags(vec![
                "sexy".into(),
                "babes".into()
            ]))
        );
        assert!(matches!(
            reason("One obsession"),
            Some(AdultReason::MatureImages(_))
        ));
        assert!(matches!(
            reason("Kodoranime"),
            Some(AdultReason::MatureImages(_))
        ));
        assert_eq!(reason("Big Love"), Some(AdultReason::MarkedNsfw));
        for keep in [
            "CyberRealistic Z-Image Turbo",
            "CyberRealistic Krea 2",
            "RIN AnimePopCute",
            "Alchemix Illustrious",
            "LucidDreamer Z",
            "Screen-chanTV",
            "AnimaYume",
        ] {
            assert_eq!(reason(keep), None, "{keep}");
        }
        let kept = page
            .items
            .iter()
            .filter(|m| s.adult_reason(m).is_none())
            .count();
        assert_eq!(kept, 33, "a third of the old default page");
    }

    /// Live checkpoint sample for the new default ("All time · Most downloaded").
    #[test]
    fn live_alltime_sample_keeps_mainstream_models() {
        let s = &filters().safe;
        let page = live("alltime");
        for keep in [
            "Realistic Vision V6.0 B1",
            "DreamShaper",
            "Juggernaut XL",
            "majicMIX realistic",
            "Pony Diffusion V6 XL",
            "epiCRealism",
            "CyberRealistic Pony",
            "RealVisXL V5.0",
            "MeinaMix",
            "Nova Anime XL",
            "FLUX",
            "Photon",
            "XXMix_9realistic",
            "Anima",
        ] {
            assert_eq!(s.adult_reason(by_name(&page, keep)), None, "{keep}");
        }
        for hide in [
            "Pony Realism",
            "One obsession",
            "Babes By Stable Yogi",
            "Analog Madness",
            "NTR MIX",
            "PicX_real",
            "Uber Realistic",
            "WAI-illustrious-SDXL",
        ] {
            assert!(s.adult_reason(by_name(&page, hide)).is_some(), "{hide}");
        }
        let kept = page
            .items
            .iter()
            .filter(|m| s.adult_reason(m).is_none())
            .count();
        assert_eq!(kept, 52);
    }

    #[test]
    fn live_lora_sample() {
        // Style add-ons ("All time · Most downloaded"): the ones CivitAI doesn't flag
        // include clothing / pose add-ons whose sample images are mostly R or above.
        let s = &filters().safe;
        let page = live("loras");
        let kept: Vec<&str> = page
            .items
            .iter()
            .filter(|m| s.adult_reason(m).is_none())
            .map(|m| m.name.as_str())
            .collect();
        for keep in [
            "Detail Tweaker XL",
            "Add More Details",
            "blindbox",
            "Studio Ghibli Style LoRA",
            "Pixel Art XL",
            "Add Micro Details",
            "Pony: People's Works",
        ] {
            assert!(kept.iter().any(|k| k.starts_with(keep)), "{keep}");
        }
        assert_eq!(
            s.adult_reason(by_name(&page, "STYLES | PONY & ANIMAGINE")),
            Some(AdultReason::Tag("sexual".into()))
        );
        assert_eq!(
            s.adult_reason(by_name(&page, "FURRY BABES")),
            Some(AdultReason::NameWord("babes".into()))
        );
        assert_eq!(
            s.adult_reason(by_name(&page, "苍铭明月")),
            Some(AdultReason::NoSafeContent),
            "nsfwLevel 30: no PG image at all"
        );
        assert!(matches!(
            s.adult_reason(by_name(&page, "Hairstyles Collection")),
            Some(AdultReason::MatureImages(_))
        ));
        assert_eq!(kept.len(), 45, "{kept:?}");
    }
}
