//! Inference engines (SPEC §2): pinned `sd-server` / `llama-server` builds
//! (`config/engine.yaml`), download + SHA-256 verify + unpack into
//! `Data/engine/<version>/<backend>/`, process management (127.0.0.1 only,
//! random free port, stdout/stderr into an in-memory ring buffer — never to
//! disk), and typed clients for the native sd-server API
//! (`/sdcpp/v1/img_gen`, `/jobs/{id}`, `/jobs/{id}/cancel`, `/capabilities`,
//! `/upscale`) and llama-server's chat API for captioning.
//!
//! OWNER: engine agent. Suggested modules: pins, install, process, ringbuf,
//! sdapi, png (strip text chunks), llama.
