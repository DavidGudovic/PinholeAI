//! The JSON the Rust core sends to the UI matches `src/lib/types.ts`, and the requests the UI
//! sends use only fields Rust reads.
//!
//! Values come from the core functions behind the Tauri commands (a mock sd-server stands in
//! for the engine) and, for Browse cards and install plans, from the CivitAI fixtures. Each value is compared with its interface in types.ts, nested interfaces
//! included: every key Rust sends must be declared there, and every field types.ts marks as
//! required (no `?`) must be sent.
//!
//! Every interface in types.ts must be either checked here or listed in `NOT_CHECKED`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use pinhole_core::events::{CoreEvent, EventSink};
use pinhole_core::generate::{self, GenerateRequest};
use pinhole_core::{
    app, catalog, describe, engine_setup, imagecheck, library, linked, models, models_folder,
    session, testing, AppCore, CoreError, ShippedPaths,
};
use pinhole_engine::testutil::MockSdServer;
use pinhole_store::datadir::ModelKind;
use pinhole_store::presets::Preset;
use pinhole_store::styles::Style;
use pinhole_store::{DataDir, InstalledFile};
use pinhole_tests::{config_dir, repo_root};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

/// Interfaces this test doesn't build a value for yet: they need CivitAI or GitHub answers
/// beyond the fixtures, or a helper model's answer.
const NOT_CHECKED: &[&str] = &[
    "GalleryItem",
    "ModelGallery",
    "PastedResource",
    "ResolvedResource",
    "ResolvedResources",
    "UpdateInfo",
    "UpdateCheck",
    "ImprovedPrompt",
];

// ---------------------------------------------------------------- types.ts reader

#[derive(Debug, Clone)]
enum Ty {
    /// An inline `{ … }` object.
    Object(Fields),
    /// A named type (interface, alias or primitive).
    Name(String),
    Array(Box<Ty>),
    /// One of several (`A | null`, string literal unions…).
    Union(Vec<Ty>),
    /// Anything else (`Record<…>`, tuples, literals): not compared.
    Other,
}

#[derive(Debug, Clone)]
struct Field {
    optional: bool,
    ty: Ty,
}

type Fields = BTreeMap<String, Field>;

/// The interfaces declared in types.ts (`export interface Name { … }`).
fn read_types_ts() -> HashMap<String, Fields> {
    let text = std::fs::read_to_string(repo_root().join("src/lib/types.ts")).unwrap();
    let src = strip_comments(&text);
    let mut out = HashMap::new();
    let mut rest = src.as_str();
    while let Some(i) = rest.find("export interface ") {
        rest = &rest[i + "export interface ".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let open = rest.find('{').unwrap();
        let body_end = matching(rest, open);
        out.insert(name, parse_fields(&rest[open + 1..body_end]));
        rest = &rest[body_end..];
    }
    assert!(out.len() > 50, "read only {} interfaces", out.len());
    out
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (None, None) => break,
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = &rest[l..];
                rest = &rest[rest.find('\n').unwrap_or(rest.len())..];
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                rest = &rest[b..];
                rest = &rest[rest.find("*/").map_or(rest.len(), |e| e + 2)..];
            }
            _ => unreachable!(),
        }
    }
    out.push_str(rest);
    out
}

/// Index of the bracket closing the one at `open`.
fn matching(s: &str, open: usize) -> usize {
    let mut depth = 0i32;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' | '[' | '(' | '<' => depth += 1,
            '}' | ']' | ')' | '>' => {
                depth -= 1;
                if depth == 0 {
                    return open + i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced brackets in types.ts");
}

/// `s` split at `sep` where no bracket is open.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '{' | '[' | '(' | '<' => depth += 1,
            '}' | ']' | ')' | '>' => depth -= 1,
            c if c == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

fn parse_fields(body: &str) -> Fields {
    let mut fields = Fields::new();
    for decl in split_top(body, ';')
        .into_iter()
        .flat_map(|d| split_top(d, '\n'))
    {
        let decl = decl.trim();
        let Some(colon) = decl.find(':') else {
            continue;
        };
        let (name, ty) = (decl[..colon].trim(), decl[colon + 1..].trim());
        let (name, optional) = match name.strip_suffix('?') {
            Some(n) => (n, true),
            None => (name, false),
        };
        fields.insert(
            name.to_string(),
            Field {
                optional,
                ty: parse_ty(ty),
            },
        );
    }
    fields
}

fn parse_ty(ty: &str) -> Ty {
    let ty = ty.trim();
    let members = split_top(ty, '|');
    if members.len() > 1 {
        return Ty::Union(members.into_iter().map(parse_ty).collect());
    }
    if let Some(inner) = ty.strip_suffix("[]") {
        return Ty::Array(Box::new(parse_ty(inner)));
    }
    if ty.starts_with('{') && matching(ty, 0) == ty.len() - 1 {
        return Ty::Object(parse_fields(&ty[1..ty.len() - 1]));
    }
    if !ty.is_empty() && ty.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Ty::Name(ty.to_string());
    }
    Ty::Other
}

// ---------------------------------------------------------------- comparison

struct Checker {
    types: HashMap<String, Fields>,
    checked: BTreeSet<String>,
    problems: BTreeSet<String>,
}

impl Checker {
    fn new() -> Self {
        Self {
            types: read_types_ts(),
            checked: BTreeSet::new(),
            problems: BTreeSet::new(),
        }
    }

    /// Compare a Rust value (or a list of them) with the types.ts interface `name`.
    fn check(&mut self, name: &str, value: &impl Serialize) {
        let v = serde_json::to_value(value).unwrap();
        let ty = Ty::Name(name.to_string());
        if v.as_array().is_some_and(|a| a.is_empty()) {
            self.problems
                .insert(format!("{name}: the test value is an empty list"));
        }
        let ty = if v.is_array() {
            Ty::Array(Box::new(ty))
        } else {
            ty
        };
        self.value(name, &ty, &v);
    }

    fn value(&mut self, at: &str, ty: &Ty, v: &Value) {
        match (ty, v) {
            (Ty::Union(members), _) => {
                // The member that fits the JSON kind (`Foo | null`, `Foo[] | null`).
                let fit = members.iter().find(|m| match (m, v) {
                    (Ty::Array(_), Value::Array(_)) => true,
                    (Ty::Object(_), Value::Object(_)) => true,
                    (Ty::Name(n), Value::Object(_)) => self.types.contains_key(n),
                    _ => false,
                });
                if let Some(m) = fit.cloned() {
                    self.value(at, &m, v);
                }
            }
            (Ty::Array(inner), Value::Array(items)) => {
                for (i, item) in items.iter().enumerate() {
                    self.value(&format!("{at}[{i}]"), inner, item);
                }
            }
            (Ty::Name(n), Value::Object(_)) if self.types.contains_key(n) => {
                self.checked.insert(n.clone());
                let fields = self.types[n].clone();
                let at = if at == n {
                    n.clone()
                } else {
                    format!("{at} ({n})")
                };
                self.object(&at, &fields, v);
            }
            (Ty::Object(fields), Value::Object(_)) => {
                let fields = fields.clone();
                self.object(at, &fields, v);
            }
            _ => {}
        }
    }

    fn object(&mut self, at: &str, fields: &Fields, v: &Value) {
        let obj = v.as_object().unwrap();
        for key in obj.keys() {
            if !fields.contains_key(key) {
                self.problems.insert(format!(
                    "{at}: Rust sends `{key}`, types.ts doesn't declare it"
                ));
            }
        }
        for (key, field) in fields {
            match obj.get(key) {
                Some(child) => self.value(&format!("{at}.{key}"), &field.ty, child),
                None if !field.optional => {
                    self.problems.insert(format!(
                        "{at}: types.ts requires `{key}`, Rust doesn't send it"
                    ));
                }
                None => {}
            }
        }
    }

    /// A request the UI sends: `sample` sets every field of interface `name` (nested
    /// interfaces too), and Rust must read all of them.
    fn request<T: DeserializeOwned>(&mut self, name: &str, sample: Value) {
        self.checked.insert(name.to_string());
        let fields = self.types[name].clone();
        self.covers(name, &fields, &sample);
        let mut ignored = Vec::new();
        let parsed: Result<T, _> = serde_ignored::deserialize(&sample, |path| {
            ignored.push(path.to_string());
        });
        if let Err(e) = parsed {
            self.problems
                .insert(format!("{name}: Rust can't read the sample: {e}"));
        }
        for path in ignored {
            self.problems.insert(format!(
                "{name}: Rust ignores `{path}` that types.ts declares"
            ));
        }
    }

    /// Every field of `fields` is set in `sample`, recursing into named interfaces.
    fn covers(&mut self, at: &str, fields: &Fields, sample: &Value) {
        for (key, field) in fields {
            let Some(child) = sample.get(key) else {
                self.problems
                    .insert(format!("{at}: the test sample leaves out `{key}`"));
                continue;
            };
            let (ty, child) = match (&field.ty, child) {
                (Ty::Array(inner), Value::Array(items)) if !items.is_empty() => {
                    (inner.as_ref(), &items[0])
                }
                (ty, child) => (ty, child),
            };
            let named = match ty {
                Ty::Name(n) => Some(n.clone()),
                Ty::Union(m) => m.iter().find_map(|t| match t {
                    Ty::Name(n) => Some(n.clone()),
                    _ => None,
                }),
                _ => None,
            };
            if let Some(n) = named.filter(|n| self.types.contains_key(n) && child.is_object()) {
                self.checked.insert(n.clone());
                let inner = self.types[&n].clone();
                self.covers(&format!("{at}.{key}"), &inner, child);
            }
        }
    }
}

// ---------------------------------------------------------------- the core under test

#[derive(Default)]
struct Recorder(Mutex<Vec<CoreEvent>>);

impl EventSink for Recorder {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().push(event);
    }
}

fn generate_request(model_id: &str) -> Value {
    json!({
        "modelId": model_id,
        "mode": "txt2img",
        "prompt": "a lighthouse at dusk",
        "styleId": null,
        "dials": { "shape": "square", "quality": "fast", "stick": 0.5, "count": 1 },
        "fineTune": { "seed": 7, "steps": 4, "width": 256, "height": 256 },
        "loras": [],
        "addTriggerWords": false
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rust_json_matches_types_ts() {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let core = AppCore::new(
        ShippedPaths {
            config_dir: config_dir(),
        },
        DataDir::at(tmp.path().join("Data"), false),
        rec.clone(),
    )
    .unwrap();
    let mock = MockSdServer::start().await;
    testing::use_external_engine(&core, &mock.base_url());
    app::start_hardware_detection(&core);
    assert!(app::wait_for_hardware(&core, Duration::from_secs(30)).await);

    let model_id = testing::register_fake_model(&core, "sdxl");
    testing::register_fake_lora(&core, "sdxl", &["neon"]);
    let mut c = Checker::new();

    // App, settings, hardware, engine.
    c.check("AppInfo", &app::app_info(&core));
    c.check("Settings", &app::get_settings(&core));
    c.check("HardwareView", &app::hardware_view(&core));
    c.check(
        "GpuInfo",
        &pinhole_hardware::GpuInfo {
            index: 0,
            vendor: pinhole_hardware::Vendor::Nvidia,
            name: "GPU".into(),
            vram_gb: 16.0,
        },
    );
    c.check("EngineStatus", &engine_setup::engine_status(&core));
    c.check("SafetyCheckStatus", &imagecheck::status(&core));
    c.check(
        "CoreError",
        &CoreError::not_found("model").with_details("details"),
    );

    // Models.
    c.check("InstalledModel", &models::list_models(&core).unwrap());
    c.check("InstalledLora", &models::list_loras(&core).unwrap());
    core.installed.lock().files.push(InstalledFile {
        id: "upscaler-1".into(),
        rel_path: "models/upscale_models/x.safetensors".into(),
        kind: ModelKind::Upscaler,
        sha256: "0".repeat(64),
        size_bytes: 1,
        family: None,
        component_id: None,
        friendly_name: "Upscaler".into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: None,
    });
    c.check("InstalledHelper", &models::list_helpers(&core).unwrap());
    c.check("RecommendedPick", &models::get_recommended(&core).unwrap());
    let model_file = core
        .installed
        .lock()
        .files
        .iter()
        .find(|f| f.id == model_id)
        .cloned()
        .unwrap();
    c.check("AddFileResult", &models::view_of(&core, &model_file));
    c.check(
        "DeletePreview",
        &models::preview_delete(&core, &model_id).unwrap(),
    );
    c.check("ModelsFolderInfo", &models_folder::info(&core));
    let other = tempfile::tempdir().unwrap();
    c.check(
        "ModelsFolderPreview",
        &models_folder::preview(&core, Some(&other.path().to_string_lossy())).unwrap(),
    );
    let linked_dir = tempfile::tempdir().unwrap();
    linked::add(&core, &linked_dir.path().to_string_lossy()).unwrap();
    c.check("LinkedFolder", &linked::list(&core));

    // Describe helpers, styles, presets, Browse filters.
    c.check(
        "CaptionerStatus",
        &describe::captioner_status(&core, describe::Purpose::Describe),
    );
    c.check("HelperModel", &describe::list_helper_models(&core));
    c.check("Style", &library::list_styles(&core).unwrap());
    let preset = json!({
        "id": "", "name": "Mine", "family": "sdxl", "modelId": null, "civitaiVersionId": null,
        "styleId": null, "shape": "square", "quality": "fast", "stick": 0.5, "count": 1,
        "fineTune": { "steps": 20 },
        "loras": [{ "loraId": "l", "civitaiVersionId": 1, "name": "Neon", "weight": 0.8 }],
        "builtin": false
    });
    c.request::<Preset>("Preset", preset.clone());
    library::save_preset(&core, serde_json::from_value(preset).unwrap()).unwrap();
    c.check("Preset", &library::list_presets(&core).unwrap());
    c.request::<Style>(
        "Style",
        json!({
            "id": "", "name": "Mine", "positive": "{prompt}, film grain", "negative": null,
            "families": ["sdxl"], "thumbnail": null, "builtin": false
        }),
    );
    c.check(
        "CatalogFilterOptions",
        &catalog::catalog_filters(&core).unwrap(),
    );

    // Generate, then save.
    let req: GenerateRequest = serde_json::from_value(generate_request(&model_id)).unwrap();
    c.check("FamilyUi", &generate::family_ui(&core, "sdxl").unwrap());
    c.check(
        "FinalPromptPreview",
        &generate::preview_final_prompt(&core, &req).unwrap(),
    );
    let result = generate::generate(&core, req).await.unwrap();
    c.check("GenerateResult", &result);
    let png = session::get(&core, &result.images[0].id).unwrap();
    c.check(
        "ImportedImage",
        &session::import_image(&core, png.to_vec()).unwrap(),
    );
    let mut settings = app::get_settings(&core);
    settings.saved_metadata = "settings".into();
    app::set_settings(&core, settings).unwrap();
    let saved = session::save_image(&core, &result.images[0].id).unwrap();
    c.check("SavedImage", &saved);
    let saved_png = std::fs::read(&saved.path).unwrap();
    c.check(
        "PictureSettings",
        &session::read_picture_settings(&saved_png).expect("settings in the saved picture"),
    );
    let out = tempfile::tempdir().unwrap();
    c.check(
        "SavedBatch",
        &session::save_images_to(
            &core,
            &[result.images[0].id.clone()],
            &out.path().to_string_lossy(),
        )
        .unwrap(),
    );

    // Moving the models folder (sends move progress).
    c.check(
        "ModelsFolderInfo",
        &models_folder::change(&core, Some(other.path().to_string_lossy().into_owned()))
            .await
            .unwrap(),
    );

    // A download that fails at once (Offline mode on).
    pinhole_core::downloads::start_event_forwarding(&core);
    let mut settings = app::get_settings(&core);
    settings.offline = true;
    app::set_settings(&core, settings).unwrap();
    let _ = engine_setup::install_engine(&core).await;
    c.check("GroupStatus", &pinhole_core::downloads::list(&core));

    // Events, as the Tauri bridge sends them (the `payload` part).
    let events: Vec<CoreEvent> = std::mem::take(&mut *rec.0.lock());
    let event_types = HashMap::from([
        ("generation-progress", "GenerationProgress"),
        ("engine-status", "EngineStatus"),
        ("download-progress", "GroupStatus"),
        ("models-move-progress", "ModelsMoveProgress"),
    ]);
    for e in &events {
        if let Some(ty) = event_types.get(e.name()) {
            let payload = serde_json::to_value(e).unwrap()["payload"].clone();
            c.value(e.name(), &Ty::Name(ty.to_string()), &payload);
        }
    }
    assert!(
        c.checked.contains("GenerationProgress"),
        "generate sent no progress events"
    );

    catalog_types(&core, &mut c);
    requests(&mut c);

    let unchecked: Vec<&String> = c
        .types
        .keys()
        .filter(|n| !c.checked.contains(*n) && !NOT_CHECKED.contains(&n.as_str()))
        .collect();
    assert!(
        unchecked.is_empty(),
        "types.ts interfaces with no value checked here (add one, or list them in NOT_CHECKED): {unchecked:?}"
    );
    for name in NOT_CHECKED {
        assert!(c.types.contains_key(*name), "{name} is not in types.ts");
        assert!(
            !c.checked.contains(*name),
            "{name} is checked now: take it off NOT_CHECKED"
        );
    }
    assert!(
        c.problems.is_empty(),
        "Rust and src/lib/types.ts disagree:\n  {}",
        Vec::from_iter(c.problems).join("\n  ")
    );
}

/// Browse cards, an install plan and an empty Browse page, built from the CivitAI fixtures.
fn catalog_types(core: &AppCore, c: &mut Checker) {
    use pinhole_catalog::api::{ModelVersion, ModelsPage};
    use pinhole_catalog::cards::{build_card, CardContext, RegistryEnv};
    use pinhole_catalog::plan::{build_plan, PlanEnv};
    use pinhole_catalog::{BrowsePage, BrowseQuery};

    let fixtures = repo_root().join("src-tauri/crates/pinhole-catalog/tests/fixtures");
    let read = |f: &str| std::fs::read_to_string(fixtures.join(f)).unwrap();
    let page: ModelsPage = serde_json::from_str(&read("models_page.json")).unwrap();
    let version: ModelVersion = serde_json::from_str(&read("model_version.json")).unwrap();

    let reg = core.registry();
    let hw = app::hw_context(core);
    let filters = catalog::filters(core).unwrap();
    let index = core.installed.lock().clone();
    let query = BrowseQuery {
        compatible_only: false,
        ..BrowseQuery::default()
    };
    let ctx = CardContext {
        filters: &filters,
        query: &query,
        now: chrono::Utc::now(),
    };
    let env = RegistryEnv::new(&reg, hw.clone(), &index);
    let cards: Vec<_> = page
        .items
        .iter()
        .filter_map(|m| build_card(&ctx, &env, m))
        .collect();
    assert!(!cards.is_empty(), "no card built from the fixture");
    c.check("CatalogCard", &cards);
    c.check("BrowsePage", &BrowsePage::empty());

    let plan_env = PlanEnv {
        registry: &reg,
        index: &index,
        hw: &hw,
        filters: &filters,
    };
    c.check(
        "InstallPlan",
        &build_plan(&plan_env, &version, None, u64::MAX, false, None),
    );
    c.check(
        "InstallStarted",
        &pinhole_core::InstallStarted {
            group_id: "g".into(),
        },
    );
}

/// Requests the UI sends, with every field types.ts declares set.
fn requests(c: &mut Checker) {
    let fine_tune = json!({
        "sampler": "euler", "scheduler": "karras", "steps": 20, "cfg": 7.0, "guidance": 3.5,
        "seed": 1, "flowShift": 3.0, "clipSkip": 2, "width": 1024, "height": 1024,
        "hires": true, "hiresScale": 1.5, "hiresDenoise": 0.4, "vaeTiling": true,
        "negativePrompt": "blurry", "autoPromptPrefix": true
    });
    c.request::<GenerateRequest>(
        "GenerateRequest",
        json!({
            "modelId": "m", "mode": "img2img", "prompt": "p", "styleId": "s",
            "dials": { "shape": "square", "quality": "fast", "stick": 0.5, "count": 1 },
            "fineTune": fine_tune,
            "loras": [{ "loraId": "l", "weight": 0.8, "words": ["w"] }],
            "addTriggerWords": true, "initImageId": "i", "strength": 0.55,
            "refImageIds": ["r"], "maskImageId": "k", "fixDetails": true,
            "extend": { "width": 10, "height": 10, "left": 0, "top": 0 }
        }),
    );
    let settings = serde_json::to_value(pinhole_store::Settings::default()).unwrap();
    c.request::<pinhole_store::Settings>("Settings", settings);
    c.request::<pinhole_catalog::BrowseQuery>(
        "BrowseQuery",
        json!({
            "kind": "models", "look": "anime", "tags": ["edit"], "content": "safe",
            "price": "free", "sort": "Newest", "period": "Week", "commercialOnly": true,
            "compatibleOnly": true, "runsOnMyCard": true, "hideAnime": true,
            "query": "q", "cursor": null
        }),
    );
}
