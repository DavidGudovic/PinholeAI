//! Styles (SPEC §7): built-ins in `config/styles/` (read-only) + user styles in
//! `Data/styles/<slug>.yaml`. The ONLY user text Pinhole stores, and only when
//! the user explicitly clicks "Save as style".

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::files::{self, builtin_stem, BUILTIN_PREFIX};
use crate::{slugify, write_atomic, DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    /// Slug; `builtin:<slug>` for shipped styles.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub positive: String,
    #[serde(default)]
    pub negative: Option<String>,
    /// Families it's written for; empty = all.
    #[serde(default)]
    pub families: Vec<String>,
    /// Thumbnail file name inside `Data/styles/` (PNG), user-picked only.
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub builtin: bool,
}

const MAX_NAME_CHARS: usize = 120;
const MAX_TEXT_CHARS: usize = 8000;

const HEADER: &str = "# Pinhole style, saved by you. Stored only on this computer; edit freely.\n";

/// Built-ins (sorted by name) followed by user styles (sorted by name).
/// Files that can't be read are skipped.
pub fn list(builtin_dir: &Path, dir: &DataDir) -> Result<Vec<Style>, StoreError> {
    let mut builtins = read_all(builtin_dir, true)?;
    let mut users = read_all(&dir.styles(), false)?;
    builtins.sort_by(|a, b| files::name_order(&a.name, &a.id, &b.name, &b.id));
    users.sort_by(|a, b| files::name_order(&a.name, &a.id, &b.name, &b.id));
    builtins.extend(users);
    Ok(builtins)
}

/// Create or update a user style. Empty `id` → new slug from name (deduplicated).
/// A non-empty `id` must name an existing user style and is kept (renaming
/// doesn't change the id). Built-in styles can't be changed.
pub fn save(dir: &DataDir, style: Style) -> Result<Style, StoreError> {
    let id = style.id.trim().to_string();
    if builtin_stem(&id).is_some() {
        return Err(StoreError::Invalid(
            "Built-in styles can't be changed. Save it under a new name to make your own copy.".into(),
        ));
    }
    let mut style = sanitize(style)?;
    let styles_dir = dir.styles();

    let (id, fresh) = if id.is_empty() {
        (files::reserve_unique(&styles_dir, &slugify(&style.name))?, true)
    } else {
        if !files::is_safe_stem(&id) || !styles_dir.join(format!("{id}.yaml")).is_file() {
            return Err(StoreError::NotFound("Style".into()));
        }
        (id, false)
    };
    style.id = id;
    style.builtin = false;

    let path = styles_dir.join(format!("{}.yaml", style.id));
    let written = files::to_library_yaml(&style, HEADER).and_then(|yaml| write_atomic(&path, yaml.as_bytes()));
    if let Err(e) = written {
        if fresh {
            let _ = std::fs::remove_file(&path);
        }
        return Err(e);
    }
    Ok(style)
}

pub fn delete(dir: &DataDir, id: &str) -> Result<(), StoreError> {
    if builtin_stem(id).is_some() {
        return Err(StoreError::Invalid("Built-in styles can't be deleted.".into()));
    }
    if !files::is_safe_stem(id) {
        return Err(StoreError::NotFound("Style".into()));
    }
    let path = dir.styles().join(format!("{id}.yaml"));
    // Best effort: remove the user-picked thumbnail that belongs to this style.
    let thumbnail = read_one(&path, id, false).ok().and_then(|s| s.thumbnail);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(StoreError::NotFound("Style".into())),
        Err(e) => return Err(e.into()),
    }
    if let Some(thumb) = thumbnail.filter(|t| valid_thumbnail(t)) {
        let _ = std::fs::remove_file(dir.styles().join(thumb));
    }
    Ok(())
}

/// Look up any style (built-in or user) by id.
pub fn get(builtin_dir: &Path, dir: &DataDir, id: &str) -> Result<Style, StoreError> {
    let (folder, stem, builtin) = match builtin_stem(id) {
        Some(stem) => (builtin_dir.to_path_buf(), stem, true),
        None => (dir.styles(), id, false),
    };
    if !files::is_safe_stem(stem) {
        return Err(StoreError::NotFound("Style".into()));
    }
    read_one(&folder.join(format!("{stem}.yaml")), stem, builtin)
}

fn read_all(folder: &Path, builtin: bool) -> Result<Vec<Style>, StoreError> {
    Ok(files::yaml_files(folder)?
        .into_iter()
        .filter_map(|(stem, path)| read_one(&path, &stem, builtin).ok())
        .collect())
}

fn read_one(path: &Path, stem: &str, builtin: bool) -> Result<Style, StoreError> {
    let text = files::read_text_or_not_found(path, "Style")?;
    let mut style: Style = serde_yaml::from_str(&text).map_err(|e| files::parse_error(path, e))?;
    style.id = if builtin { format!("{BUILTIN_PREFIX}{stem}") } else { stem.to_string() };
    style.builtin = builtin;
    Ok(style)
}

fn sanitize(style: Style) -> Result<Style, StoreError> {
    let name = style.name.trim().to_string();
    let positive = style.positive.trim().to_string();
    if name.is_empty() {
        return Err(StoreError::Invalid("Give the style a name.".into()));
    }
    if positive.is_empty() {
        return Err(StoreError::Invalid("The style is empty. Describe the look you want first.".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::Invalid(format!("The style name is too long (max {MAX_NAME_CHARS} characters).")));
    }
    let negative = files::clean_opt(style.negative);
    if positive.chars().count() > MAX_TEXT_CHARS || negative.as_ref().is_some_and(|n| n.chars().count() > MAX_TEXT_CHARS) {
        return Err(StoreError::Invalid(format!("The style text is too long (max {MAX_TEXT_CHARS} characters).")));
    }
    let mut families: Vec<String> = Vec::new();
    for f in style.families {
        let f = f.trim().to_string();
        if !f.is_empty() && !families.contains(&f) {
            families.push(f);
        }
    }
    let thumbnail = files::clean_opt(style.thumbnail);
    if let Some(t) = &thumbnail {
        if !valid_thumbnail(t) {
            return Err(StoreError::Invalid("The style picture must be a PNG file in the styles folder.".into()));
        }
    }
    Ok(Style { id: style.id, name, positive, negative, families, thumbnail, builtin: false })
}

/// A bare `*.png` file name (no folders, not hidden).
fn valid_thumbnail(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && name.to_ascii_lowercase().ends_with(".png")
        && name.len() <= 200
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn setup() -> (tempfile::TempDir, PathBuf, DataDir) {
        let tmp = tempfile::tempdir().unwrap();
        let builtin = tmp.path().join("config").join("styles");
        std::fs::create_dir_all(&builtin).unwrap();
        std::fs::write(
            builtin.join("film-photo.yaml"),
            "name: \"Film photo\"\npositive: \"35mm film\"\nnegative: \"cartoon\"\nfamilies: []\n",
        )
        .unwrap();
        std::fs::write(builtin.join("anime-cel.yaml"), "name: Anime cel shading\npositive: anime\nfamilies: [sdxl]\n").unwrap();
        std::fs::write(builtin.join("broken.yaml"), "name: [unclosed").unwrap();
        let data = DataDir::at(tmp.path().join("Data"), false);
        data.ensure_layout().unwrap();
        (tmp, builtin, data)
    }

    fn style(id: &str, name: &str, positive: &str) -> Style {
        Style {
            id: id.into(),
            name: name.into(),
            positive: positive.into(),
            negative: None,
            families: vec![],
            thumbnail: None,
            builtin: false,
        }
    }

    #[test]
    fn shipped_styles_parse() {
        let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../config/styles");
        let data = DataDir::at(PathBuf::from("/nonexistent-pinhole-test"), false);
        let all = list(&shipped, &data).unwrap();
        let ids: Vec<_> = all.iter().map(|s| s.id.as_str()).collect();
        for id in ["builtin:film-photo", "builtin:anime-cel", "builtin:product-white", "builtin:watercolor"] {
            assert!(ids.contains(&id), "{id} missing from {ids:?}");
        }
        assert!(all.iter().all(|s| s.builtin && !s.positive.is_empty()));
    }

    #[test]
    fn list_builtins_then_users_sorted() {
        let (_t, builtin, data) = setup();
        save(&data, style("", "zebra look", "stripes")).unwrap();
        save(&data, style("", "Alpha look", "a")).unwrap();
        let all = list(&builtin, &data).unwrap();
        let names: Vec<_> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Anime cel shading", "Film photo", "Alpha look", "zebra look"]);
        assert_eq!(all[0].id, "builtin:anime-cel");
        assert!(all[0].builtin && all[1].builtin);
        assert!(!all[2].builtin && all[2].id == "alpha-look");
    }

    #[test]
    fn save_new_dedupes_ids_and_update_keeps_id() {
        let (_t, builtin, data) = setup();
        let a = save(&data, style("", "My Look", "soft light")).unwrap();
        let b = save(&data, style("", "My look!", "hard light")).unwrap();
        assert_eq!(a.id, "my-look");
        assert_eq!(b.id, "my-look-2");
        assert!(!a.builtin);

        let mut renamed = a.clone();
        renamed.name = "Completely different".into();
        renamed.positive = "golden hour".into();
        renamed.negative = Some("  ".into());
        let r = save(&data, renamed).unwrap();
        assert_eq!(r.id, "my-look");
        assert_eq!(r.negative, None);
        let got = get(&builtin, &data, "my-look").unwrap();
        assert_eq!(got, r);
        assert_eq!(list(&builtin, &data).unwrap().iter().filter(|s| !s.builtin).count(), 2);
    }

    #[test]
    fn file_format_is_human_editable() {
        let (_t, _b, data) = setup();
        let mut s = style("", "Film look", "35mm film");
        s.negative = Some("cartoon".into());
        s.families = vec!["sdxl".into(), " sdxl ".into(), "".into()];
        s.builtin = true; // ignored
        let saved = save(&data, s).unwrap();
        assert_eq!(saved.families, vec!["sdxl".to_string()]);
        assert!(!saved.builtin);
        let text = std::fs::read_to_string(data.styles().join("film-look.yaml")).unwrap();
        assert!(text.starts_with("# Pinhole style"));
        assert!(text.contains("name: Film look"));
        assert!(text.contains("negative: cartoon"));
        assert!(!text.contains("id:"));
        assert!(!text.contains("builtin"));
        assert!(!text.contains("thumbnail"));
    }

    #[test]
    fn validation() {
        let (_t, _b, data) = setup();
        assert!(matches!(save(&data, style("", "  ", "x")), Err(StoreError::Invalid(_))));
        assert!(matches!(save(&data, style("", "Name", " \n ")), Err(StoreError::Invalid(_))));
        assert!(matches!(save(&data, style("", &"n".repeat(500), "x")), Err(StoreError::Invalid(_))));
        let mut bad_thumb = style("", "T", "x");
        bad_thumb.thumbnail = Some("../../secret.png".into());
        assert!(matches!(save(&data, bad_thumb), Err(StoreError::Invalid(_))));
        let mut ok_thumb = style("", "T", "x");
        ok_thumb.thumbnail = Some("t.png".into());
        assert_eq!(save(&data, ok_thumb).unwrap().thumbnail.as_deref(), Some("t.png"));
        // Unknown / hostile ids on update.
        assert!(matches!(save(&data, style("nope", "N", "x")), Err(StoreError::NotFound(_))));
        assert!(matches!(save(&data, style("../../x", "N", "x")), Err(StoreError::NotFound(_))));
        // Nothing half-written was left behind by the failures.
        let names: Vec<_> = std::fs::read_dir(data.styles()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn builtins_are_read_only() {
        let (_t, builtin, data) = setup();
        let film = get(&builtin, &data, "builtin:film-photo").unwrap();
        assert!(film.builtin);
        assert_eq!(film.negative.as_deref(), Some("cartoon"));
        let e = save(&data, film.clone()).unwrap_err();
        assert!(matches!(e, StoreError::Invalid(ref m) if m.contains("Built-in")));
        let e = delete(&data, "builtin:film-photo").unwrap_err();
        assert!(matches!(e, StoreError::Invalid(ref m) if m == "Built-in styles can't be deleted."));
        assert!(builtin.join("film-photo.yaml").exists());
    }

    #[test]
    fn get_and_delete() {
        let (_t, builtin, data) = setup();
        let mut s = style("", "Mine", "x");
        s.thumbnail = Some("mine.png".into());
        let s = save(&data, s).unwrap();
        std::fs::write(data.styles().join("mine.png"), b"png").unwrap();
        assert_eq!(get(&builtin, &data, &s.id).unwrap().name, "Mine");
        assert!(matches!(get(&builtin, &data, "missing"), Err(StoreError::NotFound(_))));
        assert!(matches!(get(&builtin, &data, "builtin:missing"), Err(StoreError::NotFound(_))));
        assert!(matches!(get(&builtin, &data, "builtin:../styles/x"), Err(StoreError::NotFound(_))));
        assert!(matches!(get(&builtin, &data, "../etc"), Err(StoreError::NotFound(_))));
        delete(&data, &s.id).unwrap();
        assert!(!data.styles().join("mine.yaml").exists());
        assert!(!data.styles().join("mine.png").exists());
        assert!(matches!(delete(&data, &s.id), Err(StoreError::NotFound(_))));
        assert!(matches!(delete(&data, "../config/settings"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn hand_edited_user_file_cannot_claim_builtin_or_other_id() {
        let (_t, builtin, data) = setup();
        std::fs::write(data.styles().join("hand-made.yaml"), "id: builtin:film-photo\nbuiltin: true\nname: Hand\npositive: p\n")
            .unwrap();
        let s = get(&builtin, &data, "hand-made").unwrap();
        assert_eq!(s.id, "hand-made");
        assert!(!s.builtin);
    }

    #[test]
    fn hand_named_file_is_listed_and_manageable() {
        let (_t, builtin, data) = setup();
        std::fs::write(data.styles().join("Portrait Look.yaml"), "name: Portrait look\npositive: soft light\n").unwrap();
        let all = list(&builtin, &data).unwrap();
        let s = all.iter().find(|s| s.id == "Portrait Look").expect("hand-named style listed");
        let mut edited = s.clone();
        edited.positive = "hard light".into();
        assert_eq!(save(&data, edited).unwrap().id, "Portrait Look");
        assert_eq!(get(&builtin, &data, "Portrait Look").unwrap().positive, "hard light");
        delete(&data, "Portrait Look").unwrap();
        assert!(!data.styles().join("Portrait Look.yaml").exists());
    }

    #[test]
    fn missing_user_folder_lists_builtins_only() {
        let (_t, builtin, _data) = setup();
        let data = DataDir::at(PathBuf::from("/nonexistent-pinhole-test"), false);
        assert_eq!(list(&builtin, &data).unwrap().len(), 2);
    }
}
