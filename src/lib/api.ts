// Typed wrappers for every Tauri command and event. This file IS the contract
// with the Rust side (src-tauri/src/commands/*.rs). Command names are
// snake_case; argument keys are camelCase (Tauri converts them).
//
// PRIVACY: never console.log arguments or results of prompt-bearing calls.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type * as T from "./types";

// ---------------------------------------------------------------- app (store agent)
export const appInfo = () => invoke<T.AppInfo>("app_info");
export const getSettings = () => invoke<T.Settings>("get_settings");
/** Saves settings.yaml and applies side effects (offline flag, theme, GPU override). */
export const setSettings = (settings: T.Settings) => invoke<T.Settings>("set_settings", { settings });
export const getHardware = () => invoke<T.HardwareView>("get_hardware");
export const openDataFolder = () => invoke<void>("open_data_folder");
export const openOutputsFolder = () => invoke<void>("open_outputs_folder");
/** One request to GitHub's releases API. Only ever called from the "Check for updates" button. */
export const checkForUpdates = () => invoke<T.UpdateCheck>("check_for_updates");
/** Downloads + verifies the update (progress via onDownload, kind "appUpdate"), then Pinhole restarts. Resolves only on failure paths that return. */
export const installUpdate = (version: string) => invoke<void>("install_update", { version });
/** Opens the GitHub release page (or the releases list) in the system browser. */
export const openReleasePage = (version: string | null) => invoke<void>("open_release_page", { version });
/** Optional GitHub token (OS keychain only) so updates work while the repository is private. */
export const hasGithubToken = () => invoke<boolean>("has_github_token");
export const setGithubToken = (token: string) => invoke<void>("set_github_token", { token });
export const clearGithubToken = () => invoke<void>("clear_github_token");

// ---------------------------------------------------------------- engine (engine agent)
export const engineStatus = () => invoke<T.EngineStatus>("engine_status");
/** Downloads + verifies + unpacks the engine for the current backend. Progress via onDownload. */
export const installEngine = () => invoke<T.EngineStatus>("install_engine");

// ---------------------------------------------------------------- downloads (net agent)
export const listDownloads = () => invoke<T.GroupStatus[]>("list_downloads");
export const cancelDownload = (groupId: string) => invoke<void>("cancel_download", { groupId });

// ---------------------------------------------------------------- models (catalog agent)
export const listModels = () => invoke<T.InstalledModel[]>("list_models");
export const listLoras = () => invoke<T.InstalledLora[]>("list_loras");
export const getRecommended = () => invoke<T.RecommendedPick[]>("get_recommended");
export const installRecommended = (role: string) => invoke<T.InstallStarted>("install_recommended", { role });
/** "Add a file I already have": detects the family, then copies the file into Data/models/<kind>/ (hash computed while copying). */
export const addLocalModel = (path: string) => invoke<T.AddFileResult>("add_local_model", { path });
export const confirmFamily = (token: string, familyId: string) =>
  invoke<T.AddFileResult>("confirm_family", { token, familyId });
export const previewDelete = (modelId: string) => invoke<T.DeletePreview>("preview_delete", { modelId });
export const deleteModel = (modelId: string) => invoke<void>("delete_model", { modelId });
/** Paste from CivitAI: match resources to installed files or installable CivitAI versions. No prompt text is sent. */
export const resolveCivitaiResources = (resources: T.PastedResource[]) =>
  invoke<T.ResolvedResources>("resolve_civitai_resources", { resources });

// ---------------------------------------------------------------- catalog (catalog agent)
export const catalogFilters = () => invoke<T.CatalogFilterOptions>("catalog_filters");
export const browseCatalog = (query: T.BrowseQuery) => invoke<T.BrowsePage>("browse_catalog", { query });
/** Preview image bytes fetched by Rust (the WebView makes no network calls). */
export const fetchPreview = (url: string) => invoke<ArrayBuffer>("fetch_preview", { url });
/** Model details page: preview images + their generation data (kept in memory only). */
export const modelGallery = (versionId: number, content: T.ContentMode, modelNsfw: boolean) =>
  invoke<T.ModelGallery>("model_gallery", { versionId, content, modelNsfw });
/** Opens the model's CivitAI page (civitai.red for NSFW models) in the system browser. */
export const openCivitaiPage = (modelId: number, versionId: number | null, nsfw: boolean) =>
  invoke<void>("open_civitai_page", { modelId, versionId, nsfw });
export const planCivitaiInstall = (versionId: number) => invoke<T.InstallPlan>("plan_civitai_install", { versionId });
export const installCivitai = (versionId: number, familyId: string | null) =>
  invoke<T.InstallStarted>("install_civitai", { versionId, familyId });
export const hasCivitaiKey = () => invoke<boolean>("has_civitai_key");
export const setCivitaiKey = (key: string) => invoke<void>("set_civitai_key", { key });
export const clearCivitaiKey = () => invoke<void>("clear_civitai_key");

// ---------------------------------------------------------------- generate (engine agent)
export const familyUi = (familyId: string) => invoke<T.FamilyUi>("family_ui", { familyId });
/** Resolves when the images are ready (or rejects with CoreError, code "cancelled" on cancel). */
export const generate = (req: T.GenerateRequest) => invoke<T.GenerateResult>("generate", { req });
export const cancelGeneration = () => invoke<void>("cancel_generation");
/** Read-only "Final prompt sent to the model" (combined in memory, never stored). */
export const previewFinalPrompt = (req: T.GenerateRequest) =>
  invoke<T.FinalPromptPreview>("preview_final_prompt", { req });
/** Put an image (PNG/JPEG/WebP bytes) into the in-memory session. Raw binary body. */
export const importImage = (bytes: Uint8Array) => invoke<T.ImportedImage>("import_image", bytes);
/** Bytes of a session image: PNG for generated/upscaled images; imported images keep their format (PNG/JPEG/WebP). */
export const getImage = (id: string) => invoke<ArrayBuffer>("get_image", { id });
/** Writes Data/outputs/pinhole_YYYYMMDD_HHMMSS_<seed>.png. */
export const saveImage = (id: string) => invoke<T.SavedImage>("save_image", { id });
/** Save to a user-chosen path (from the dialog plugin). */
export const saveImageAs = (id: string, path: string) => invoke<T.SavedImage>("save_image_as", { id, path });
export const copyImage = (id: string) => invoke<void>("copy_image", { id });
export const discardImage = (id: string) => invoke<void>("discard_image", { id });
/** Drops every in-memory image immediately. */
export const clearSession = () => invoke<void>("clear_session");
export const upscaleImage = (id: string, factor: 2 | 4) => invoke<T.ResultImage>("upscale_image", { id, factor });

// ---------------------------------------------------------------- describe (engine agent)
export const captionerStatus = () => invoke<T.CaptionerStatus>("captioner_status");
export const installCaptioner = () => invoke<T.InstallStarted>("install_captioner");
export const describeImage = (imageId: string, style: T.DescribeStyle) =>
  invoke<string>("describe_image", { imageId, style });

// ---------------------------------------------------------------- library (store agent)
export const listStyles = () => invoke<T.Style[]>("list_styles");
/** Explicit user action only ("Save as style"). */
export const saveStyle = (style: T.Style) => invoke<T.Style>("save_style", { style });
export const deleteStyle = (id: string) => invoke<void>("delete_style", { id });
export const listPresets = () => invoke<T.Preset[]>("list_presets");
export const savePreset = (preset: T.Preset) => invoke<T.Preset>("save_preset", { preset });
export const deletePreset = (id: string) => invoke<void>("delete_preset", { id });

// ---------------------------------------------------------------- events
export const onDownload = (cb: (s: T.GroupStatus) => void): Promise<UnlistenFn> =>
  listen<T.GroupStatus>("download-progress", (e) => cb(e.payload));
export const onGeneration = (cb: (p: T.GenerationProgress) => void): Promise<UnlistenFn> =>
  listen<T.GenerationProgress>("generation-progress", (e) => cb(e.payload));
export const onEngine = (cb: (s: T.EngineStatus) => void): Promise<UnlistenFn> =>
  listen<T.EngineStatus>("engine-status", (e) => cb(e.payload));
export const onModelsChanged = (cb: () => void): Promise<UnlistenFn> => listen("models-changed", () => cb());
export const onHardwareReady = (cb: () => void): Promise<UnlistenFn> => listen("hardware-ready", () => cb());

/** Normalise anything thrown by invoke into a CoreError. */
export function asCoreError(e: unknown): T.CoreError {
  if (e && typeof e === "object" && "message" in e && "code" in e) return e as T.CoreError;
  return { code: "internal", message: typeof e === "string" ? e : "Something went wrong.", details: null };
}
