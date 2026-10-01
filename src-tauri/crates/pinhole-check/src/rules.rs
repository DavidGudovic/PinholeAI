//! What the check blocks, as pure functions over the classifiers' scores, so every rule
//! is unit-tested with made-up scores (no model files, no test images).
//!
//! Three things are blocked, whatever Safe mode is set to (RELEASE-SPEC §4):
//! 1. a brought-in photo of a person made intimate;
//! 2. a sexual image of someone who looks like a child;
//! 3. an intimate image from a model CivitAI marks "safe images only".
//!
//! The thresholds are constants on purpose: nothing outside the code can change them.

/// Tagger `explicit` rating that counts as sexual on its own.
pub const EXPLICIT: f32 = 0.35;
/// Tagger `explicit` rating that counts as intimate even without a nudity or underwear tag
/// (the tagger was trained on drawings and may not name what it sees on a photo).
pub const EXPLICIT_INTIMATE: f32 = 0.6;
/// `questionable + explicit` that counts as intimate (nude, underwear only, see-through).
pub const INTIMATE_RATING: f32 = 0.5;
/// Nudity score an intimate image must also reach.
pub const INTIMATE_NUDITY: f32 = 0.8;
/// Nudity score with which an intimate rating counts as sexual.
pub const SEXUAL_NUDITY: f32 = 0.85;
/// A nudity or underwear tag at least this counts.
pub const INTIMATE_TAG: f32 = 0.5;
/// Tagger tags for a drawn character tagged as a child (`Tags::minor`).
pub const MINOR_TAG: f32 = 0.5;
/// Photo style: `realistic` or `photorealistic` at least this. Real photos score
/// 0.15–0.97 on `realistic`; drawings stay near 0. The age estimate only runs on
/// photo-style images: it was trained on photos and can't judge drawn faces.
pub const PHOTO_STYLE: f32 = 0.1;
/// Face finder score and smallest face side (pixels) that count as a face.
pub const FACE_SCORE: f32 = 0.8;
pub const FACE_MIN_SIDE: f32 = 40.0;
/// Smallest face side (pixels) the age estimate still judges on a sexual photo. Below it a
/// clear face is too small to judge and counts as a child's (fails closed).
pub const AGE_MIN_SIDE: f32 = 16.0;
/// Smallest face side (pixels) that counts on a brought-in picture. Much lower than on a result:
/// Edit and Upscale make a small face large again, so a small face still means a person.
pub const ORIGINAL_FACE_MIN_SIDE: f32 = 12.0;
/// Age estimate's confidence that a face is a child's (its 0–2 and 3–9 groups) that blocks.
/// On photos, adults score at most ~0.05 here and young children ~0.77. See `decide` for why
/// the photo-face rule stops at clear children.
pub const CHILD_FACE: f32 = 0.6;

/// The tagger scores the rules use.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tags {
    pub general: f32,
    pub sensitive: f32,
    pub questionable: f32,
    pub explicit: f32,
    /// Highest of the tagger's tags for a character tagged as a child.
    pub minor: f32,
    pub realistic: f32,
    pub photorealistic: f32,
    /// Highest of the tagger's nudity tags. "Intimate" (rules 1 and 3) needs this or
    /// `underwear`: the nudity model and the questionable rating alone also fire on swimwear.
    /// Rule 2 doesn't use them (it stays as strict as it was).
    pub nude: f32,
    /// Highest of the tagger's underwear, lingerie and see-through tags (not swimwear).
    pub underwear: f32,
}

/// One face the face finder saw.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Face {
    pub score: f32,
    /// Shorter side of the face box, in image pixels.
    pub side: f32,
    /// Age estimate's confidence that this is a child's face (its 0–2 and 3–9 groups), when
    /// it ran.
    pub child_face: Option<f32>,
}

impl Face {
    pub fn counts(&self) -> bool {
        self.score >= FACE_SCORE && self.side >= FACE_MIN_SIDE
    }

    /// A clear face big enough for the age estimate.
    pub fn judged(&self) -> bool {
        self.score >= FACE_SCORE && self.side >= AGE_MIN_SIDE
    }

    /// A clear face too small for the age estimate to judge.
    pub fn too_small_to_judge(&self) -> bool {
        self.score >= FACE_SCORE && self.side < AGE_MIN_SIDE
    }

    /// A face on a brought-in picture (see [`ORIGINAL_FACE_MIN_SIDE`]).
    pub fn counts_in_original(&self) -> bool {
        self.score >= FACE_SCORE && self.side >= ORIGINAL_FACE_MIN_SIDE
    }
}

/// What the check measured on one result. The nudity classifier and the tagger always
/// run; the face finder and age estimate only on sexual photo-style results, so
/// `faces` can be `None` (and `tags` too, in tests).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readings {
    /// Nudity classifier, 0..1 (it also calls revealing clothes "nsfw").
    pub nudity: f32,
    pub tags: Option<Tags>,
    pub faces: Option<Vec<Face>>,
}

/// What the check measured once on a brought-in picture (the start of a chain).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Original {
    pub has_face: bool,
    /// Already intimate when it was brought in.
    pub intimate: bool,
}

/// Which rule blocked (shown only in dev builds; users see one neutral message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    PhotoMadeIntimate,
    LooksUnderage,
    SafeImagesOnlyModel,
}

impl Rule {
    pub fn key(self) -> &'static str {
        match self {
            Rule::PhotoMadeIntimate => "photo_made_intimate",
            Rule::LooksUnderage => "looks_underage",
            Rule::SafeImagesOnlyModel => "safe_images_only_model",
        }
    }
}

/// Nude, underwear only, see-through or sexual: the tagger sees nudity or underwear (so
/// swimwear alone doesn't count), and the nudity model with the rating, or an explicit
/// rating, agrees. A strongly explicit rating counts on its own.
pub fn is_intimate(nudity: f32, tags: Option<&Tags>) -> bool {
    tags.is_some_and(|t| {
        t.explicit >= EXPLICIT_INTIMATE
            || ((t.nude >= INTIMATE_TAG || t.underwear >= INTIMATE_TAG)
                && ((nudity >= INTIMATE_NUDITY && t.questionable + t.explicit >= INTIMATE_RATING)
                    || t.explicit >= EXPLICIT))
    })
}

pub fn is_sexual(nudity: f32, tags: Option<&Tags>) -> bool {
    tags.is_some_and(|t| {
        t.explicit >= EXPLICIT
            || (nudity >= SEXUAL_NUDITY && t.questionable + t.explicit >= INTIMATE_RATING)
    })
}

pub fn is_photo_style(tags: &Tags) -> bool {
    tags.realistic.max(tags.photorealistic) >= PHOTO_STYLE
}

/// Whether the face finder and age estimate must run on this result.
pub fn needs_faces(nudity: f32, tags: Option<&Tags>) -> bool {
    is_sexual(nudity, tags) && tags.is_some_and(is_photo_style)
}

/// The verdict for one result. `originals`: the brought-in pictures it was made from
/// (empty for a plain Create). `safe_images_only`: a model or LoRA in use is marked
/// "safe images only" on CivitAI.
pub fn decide(r: &Readings, originals: &[Original], safe_images_only: bool) -> Option<Rule> {
    let tags = r.tags.as_ref();
    if is_sexual(r.nudity, tags) {
        let minor_tag = tags.is_some_and(|t| t.minor >= MINOR_TAG);
        // Photos: the age estimate only sorts faces into wide groups (0–2, 3–9, 10–19, 20–29…)
        // and is often off by several years, so it can't tell a teenager from a young adult
        // without also blocking many adults. This rule therefore only acts on faces it is
        // confident are children. Teenagers are left to the word check, the brought-in
        // photo rule (rule 1) and, for drawings, the tagger's tags.
        // A clear face too small to judge counts as a child's (fails closed).
        let child_face = tags.is_some_and(is_photo_style)
            && r.faces.iter().flatten().any(|f| {
                f.too_small_to_judge()
                    || (f.judged() && f.child_face.is_some_and(|c| c >= CHILD_FACE))
            });
        if minor_tag || child_face {
            return Some(Rule::LooksUnderage);
        }
    }
    if is_intimate(r.nudity, tags) {
        if originals.iter().any(|o| o.has_face && !o.intimate) {
            return Some(Rule::PhotoMadeIntimate);
        }
        if safe_images_only {
            return Some(Rule::SafeImagesOnlyModel);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(q: f32, e: f32) -> Tags {
        Tags {
            questionable: q,
            explicit: e,
            general: (1.0 - q - e).max(0.0),
            ..Tags::default()
        }
    }
    fn photo(mut t: Tags) -> Tags {
        t.realistic = 0.7;
        t
    }
    /// The tagger sees nudity.
    fn nude(mut t: Tags) -> Tags {
        t.nude = 0.9;
        t
    }
    fn face(child_face: f32) -> Face {
        Face {
            score: 0.9,
            side: 120.0,
            child_face: Some(child_face),
        }
    }
    fn readings(nudity: f32, t: Option<Tags>, faces: Vec<Face>) -> Readings {
        Readings {
            nudity,
            tags: t,
            faces: Some(faces),
        }
    }
    const PHOTO_ORIGINAL: Original = Original {
        has_face: true,
        intimate: false,
    };

    #[test]
    fn ordinary_pictures_pass() {
        // A landscape: nothing past the first classifier.
        assert_eq!(
            decide(
                &Readings {
                    nudity: 0.01,
                    ..Default::default()
                },
                &[],
                false
            ),
            None
        );
        // A child at the beach in a photo: revealing to the classifier, not intimate.
        let mut t = photo(tags(0.1, 0.0));
        t.sensitive = 0.8;
        assert_eq!(
            decide(
                &readings(0.9, Some(t), vec![face(0.95)]),
                &[PHOTO_ORIGINAL],
                true
            ),
            None
        );
        // An edited family photo that stays ordinary.
        assert_eq!(
            decide(&readings(0.1, None, vec![]), &[PHOTO_ORIGINAL], false),
            None
        );
    }

    #[test]
    fn adult_content_of_adults_passes() {
        let r = readings(0.99, Some(photo(tags(0.1, 0.9))), vec![face(0.02)]);
        assert_eq!(decide(&r, &[], false), None);
        // Drawn adults: no child tags, the age estimate isn't used on drawings.
        let r = readings(0.99, Some(tags(0.1, 0.9)), vec![face(0.9)]);
        assert_eq!(decide(&r, &[], false), None);
        // Starting from an intimate picture that was brought in that way.
        let r = readings(0.99, Some(photo(tags(0.1, 0.9))), vec![face(0.02)]);
        let orig = Original {
            has_face: true,
            intimate: true,
        };
        assert_eq!(decide(&r, &[orig], false), None);
        // A brought-in picture without a person (a room, a landscape).
        let orig = Original {
            has_face: false,
            intimate: false,
        };
        assert_eq!(decide(&r, &[orig], false), None);
    }

    #[test]
    fn sexual_images_with_child_tags_are_blocked() {
        let mut t = tags(0.0, 0.8);
        t.minor = 0.6;
        let r = readings(0.9, Some(t), vec![]);
        assert_eq!(decide(&r, &[], false), Some(Rule::LooksUnderage));
        // The same tag on a non-sexual picture (a drawn kid at school) passes.
        let mut t = tags(0.0, 0.0);
        t.minor = 0.9;
        assert_eq!(decide(&readings(0.3, Some(t), vec![]), &[], false), None);
    }

    #[test]
    fn sexual_photos_with_a_child_face_are_blocked() {
        let r = readings(
            0.95,
            Some(photo(tags(0.2, 0.7))),
            vec![face(0.02), face(0.8)],
        );
        assert_eq!(decide(&r, &[], false), Some(Rule::LooksUnderage));
        // Unsure faces don't count.
        let unsure = Face {
            score: 0.5,
            ..face(0.9)
        };
        let r = readings(0.95, Some(photo(tags(0.2, 0.7))), vec![unsure]);
        assert_eq!(decide(&r, &[], false), None);
        // A clear face too small to judge blocks a sexual photo (fails closed), but not an
        // ordinary one.
        let tiny = Face {
            score: 0.9,
            side: 12.0,
            child_face: None,
        };
        let r = readings(0.95, Some(photo(tags(0.2, 0.7))), vec![tiny]);
        assert_eq!(decide(&r, &[], false), Some(Rule::LooksUnderage));
        let r = readings(0.1, Some(photo(tags(0.0, 0.0))), vec![tiny]);
        assert_eq!(decide(&r, &[], false), None);
        // Not a photo: the age estimate is ignored.
        let r = readings(0.95, Some(tags(0.2, 0.7)), vec![face(0.9)]);
        assert_eq!(decide(&r, &[], false), None);
    }

    #[test]
    fn brought_in_photo_made_intimate_is_blocked() {
        let r = readings(0.9, Some(nude(photo(tags(0.6, 0.1)))), vec![face(0.02)]);
        assert_eq!(
            decide(&r, &[PHOTO_ORIGINAL], false),
            Some(Rule::PhotoMadeIntimate)
        );
        // Any of two inputs.
        let other = Original {
            has_face: false,
            intimate: false,
        };
        assert_eq!(
            decide(&r, &[other, PHOTO_ORIGINAL], false),
            Some(Rule::PhotoMadeIntimate)
        );
        // Underwear counts too.
        let mut t = photo(tags(0.6, 0.1));
        t.underwear = 0.8;
        let r = readings(0.9, Some(t), vec![]);
        assert_eq!(
            decide(&r, &[PHOTO_ORIGINAL], false),
            Some(Rule::PhotoMadeIntimate)
        );
        // Revealing but not intimate: passes.
        let r = readings(0.9, Some(photo(tags(0.2, 0.0))), vec![]);
        assert_eq!(decide(&r, &[PHOTO_ORIGINAL], false), None);
    }

    #[test]
    fn swimwear_is_not_intimate_but_nudity_still_is() {
        // Swimwear: the nudity model and the questionable rating fire, the nudity tags don't.
        let swim = readings(0.99, Some(photo(tags(0.9, 0.02))), vec![]);
        assert_eq!(decide(&swim, &[PHOTO_ORIGINAL], false), None);
        assert_eq!(decide(&swim, &[], true), None);
        // The same scores with a nudity tag: blocked by rules 1 and 3.
        let nudity = readings(0.99, Some(nude(photo(tags(0.9, 0.02)))), vec![]);
        assert_eq!(
            decide(&nudity, &[PHOTO_ORIGINAL], false),
            Some(Rule::PhotoMadeIntimate)
        );
        assert_eq!(decide(&nudity, &[], true), Some(Rule::SafeImagesOnlyModel));
        // An explicit rating with a nudity tag counts even when the nudity model misses it.
        let explicit = readings(0.05, Some(nude(tags(0.1, 0.7))), vec![]);
        assert_eq!(
            decide(&explicit, &[], true),
            Some(Rule::SafeImagesOnlyModel)
        );
        // Rule 2 is unchanged: swimwear that reads as sexual with a child face is still blocked.
        let child = readings(0.97, Some(photo(tags(0.55, 0.22))), vec![face(0.8)]);
        assert_eq!(decide(&child, &[], false), Some(Rule::LooksUnderage));
    }

    #[test]
    fn a_strong_explicit_rating_is_intimate_without_a_tag() {
        // The tagger may not name nudity on a photo; a strong explicit rating counts alone,
        // from exactly EXPLICIT_INTIMATE up.
        let at = readings(0.9, Some(photo(tags(0.2, EXPLICIT_INTIMATE))), vec![]);
        assert_eq!(
            decide(&at, &[PHOTO_ORIGINAL], false),
            Some(Rule::PhotoMadeIntimate)
        );
        let below = readings(0.9, Some(photo(tags(0.2, 0.55))), vec![]);
        assert_eq!(decide(&below, &[PHOTO_ORIGINAL], false), None);
    }

    #[test]
    fn safe_images_only_models_cant_make_intimate_images() {
        let r = readings(0.9, Some(nude(tags(0.7, 0.2))), vec![]);
        assert_eq!(decide(&r, &[], true), Some(Rule::SafeImagesOnlyModel));
        assert_eq!(decide(&r, &[], false), None);
        let r = readings(0.4, Some(tags(0.1, 0.0)), vec![]);
        assert_eq!(decide(&r, &[], true), None);
    }

    #[test]
    fn small_faces_count_on_brought_in_pictures() {
        let small = Face {
            score: 0.9,
            side: 25.0,
            child_face: None,
        };
        assert!(!small.counts());
        assert!(small.counts_in_original());
        let speck = Face { side: 8.0, ..small };
        assert!(!speck.counts_in_original());
    }

    #[test]
    fn later_steps_run_only_when_needed() {
        assert!(!needs_faces(0.95, None));
        assert!(!needs_faces(0.95, Some(&tags(0.1, 0.8))), "drawn");
        assert!(needs_faces(0.95, Some(&photo(tags(0.1, 0.8)))));
        assert!(!needs_faces(0.5, Some(&photo(tags(0.1, 0.0)))));
        // An explicit drawing the nudity classifier misses is still sexual (the tagger
        // always runs, so rule 2's child tags are always read).
        assert!(is_sexual(0.05, Some(&tags(0.0, 0.8))));
        let mut t = tags(0.0, 0.8);
        t.minor = 0.7;
        assert_eq!(
            decide(&readings(0.05, Some(t), vec![]), &[], false),
            Some(Rule::LooksUnderage)
        );
        assert!(!is_sexual(1.0, None));
        assert!(!is_intimate(1.0, None));
    }
}
