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
/// `questionable + explicit` that counts as intimate (nude, underwear only, see-through).
pub const INTIMATE_RATING: f32 = 0.5;
/// Nudity score an intimate image must also reach.
pub const INTIMATE_NUDITY: f32 = 0.8;
/// Nudity score with which an intimate rating counts as sexual.
pub const SEXUAL_NUDITY: f32 = 0.85;
/// Tagger child tags (`loli`, `shota`, `child`).
pub const CHILD_TAG: f32 = 0.5;
/// Photo style: `realistic` or `photorealistic` at least this. Real photos score
/// 0.15–0.97 on `realistic`; drawings stay near 0. The age estimate only runs on
/// photo-style images: it was trained on photos and can't judge drawn faces.
pub const PHOTO_STYLE: f32 = 0.1;
/// Face finder score and smallest face side (pixels) that count as a face.
pub const FACE_SCORE: f32 = 0.8;
pub const FACE_MIN_SIDE: f32 = 40.0;
/// Chance that a face is under 10 (age classes 0–2 plus 3–9) that blocks. Aimed at
/// clear children: on photos, adults score at most ~0.05 here, young children ~0.77.
pub const UNDER_TEN: f32 = 0.6;

/// The tagger scores the rules use.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tags {
    pub general: f32,
    pub sensitive: f32,
    pub questionable: f32,
    pub explicit: f32,
    pub loli: f32,
    pub shota: f32,
    pub child: f32,
    pub realistic: f32,
    pub photorealistic: f32,
}

/// One face the face finder saw.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Face {
    pub score: f32,
    /// Shorter side of the face box, in image pixels.
    pub side: f32,
    /// Chance the face is under 10, when the age estimate ran.
    pub under_ten: Option<f32>,
}

impl Face {
    pub fn counts(&self) -> bool {
        self.score >= FACE_SCORE && self.side >= FACE_MIN_SIDE
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

/// Nude, underwear only, see-through or sexual.
pub fn is_intimate(nudity: f32, tags: Option<&Tags>) -> bool {
    tags.is_some_and(|t| {
        nudity >= INTIMATE_NUDITY && t.questionable + t.explicit >= INTIMATE_RATING
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
        let child_tag = tags.is_some_and(|t| t.loli.max(t.shota).max(t.child) >= CHILD_TAG);
        let child_face = tags.is_some_and(is_photo_style)
            && r.faces
                .iter()
                .flatten()
                .any(|f| f.counts() && f.under_ten.is_some_and(|u| u >= UNDER_TEN));
        if child_tag || child_face {
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
    fn face(under_ten: f32) -> Face {
        Face {
            score: 0.9,
            side: 120.0,
            under_ten: Some(under_ten),
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
        for i in 0..3 {
            let mut t = tags(0.0, 0.8);
            *[&mut t.loli, &mut t.shota, &mut t.child][i] = 0.6;
            let r = readings(0.9, Some(t), vec![]);
            assert_eq!(decide(&r, &[], false), Some(Rule::LooksUnderage));
        }
        // The same tag on a non-sexual picture (a drawn kid at school) passes.
        let mut t = tags(0.0, 0.0);
        t.child = 0.9;
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
        // Faces too small or unsure don't count.
        let tiny = Face {
            side: 20.0,
            ..face(0.9)
        };
        let unsure = Face {
            score: 0.5,
            ..face(0.9)
        };
        let r = readings(0.95, Some(photo(tags(0.2, 0.7))), vec![tiny, unsure]);
        assert_eq!(decide(&r, &[], false), None);
        // Not a photo: the age estimate is ignored.
        let r = readings(0.95, Some(tags(0.2, 0.7)), vec![face(0.9)]);
        assert_eq!(decide(&r, &[], false), None);
    }

    #[test]
    fn brought_in_photo_made_intimate_is_blocked() {
        let r = readings(0.9, Some(photo(tags(0.6, 0.1))), vec![face(0.02)]);
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
        // Revealing but not intimate (swimwear): passes.
        let r = readings(0.9, Some(photo(tags(0.2, 0.0))), vec![]);
        assert_eq!(decide(&r, &[PHOTO_ORIGINAL], false), None);
    }

    #[test]
    fn safe_images_only_models_cant_make_intimate_images() {
        let r = readings(0.9, Some(tags(0.7, 0.2)), vec![]);
        assert_eq!(decide(&r, &[], true), Some(Rule::SafeImagesOnlyModel));
        assert_eq!(decide(&r, &[], false), None);
        let r = readings(0.4, Some(tags(0.1, 0.0)), vec![]);
        assert_eq!(decide(&r, &[], true), None);
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
        t.loli = 0.7;
        assert_eq!(
            decide(&readings(0.05, Some(t), vec![]), &[], false),
            Some(Rule::LooksUnderage)
        );
        assert!(!is_sexual(1.0, None));
        assert!(!is_intimate(1.0, None));
    }
}
