# Pinhole's sd-server patch

Upstream `sd-server` has no authentication and answers any CORS `Origin`, so while Pinhole
runs, any program on the computer or any web page in the browser could send it jobs and skip
Pinhole's word and image checks (RELEASE-SPEC §12, SPEC §13 "Local engine API exposure").
Pinhole keeps upstream stable-diffusion.cpp unchanged except for this one small patch.

`0001-server-api-key-and-reject-origin.patch` (against `master-929-3f8527a`, `examples/server`
only) adds two opt-in options:

- `--api-key <key>`, or the `SD_API_KEY` environment variable: every request needs
  `Authorization: Bearer <key>`, else `401`. Pinhole passes a random key per launch in the
  environment (not on the command line) and sends it with every request.
- `--reject-origin`: every request with an `Origin` header (that is, from a web page) gets
  `403`, and no CORS headers are sent.

Both 401 and 403 close the connection. The checks sit in the server's pre-routing handler, so
every route is covered. The key is never logged.

## Where the builds come from

No fork and no second repo: the **Engine** workflow in this repo (`.github/workflows/engine.yml`)
checks out official stable-diffusion.cpp at the commit pinned in `config/engine.yaml`, applies
every `engine/sd-cpp/*.patch`, builds Linux CPU / Vulkan / CUDA and Windows CPU / Vulkan / CUDA,
and publishes them as a release `engine-<version>-p<N>` here. Linux CUDA is new (upstream has
none) and is built only for RTX 30/40/50 cards to keep the download small; Windows CUDA keeps
upstream's architecture list. App releases (`v*`) don't rebuild the engine.

1. Actions → Engine → Run workflow (`patch_level` 1; bump it when only the patch changes).
2. Point `config/engine.yaml` at that release (sizes and SHA-256 checked with the verify-pins
   workflow), add a `linux_cuda` build (+ its cudart zip as `extra`), set
   `ENGINE_LOCKDOWN = true` in `pinhole-core/src/generate.rs`, run the engine smoke test.

Updating the engine: change the pin in `config/engine.yaml`, refresh the patch if it no longer
applies, run the workflow again.

While the repo is private, Pinhole can't download these release files without a GitHub token
(the engine installer sends none), and the workflow uses the private repo's Actions minutes.

## Checking a build

```
SD_API_KEY=s3cret ./sd-server --diffusion-model <model> --listen-port 18765 --reject-origin
curl -i localhost:18765/sdcpp/v1/capabilities                                   # 401
curl -i -H 'Authorization: Bearer s3cret' localhost:18765/sdcpp/v1/capabilities # 200
curl -i -H 'Authorization: Bearer s3cret' -H 'Origin: https://example.com' \
     localhost:18765/sdcpp/v1/capabilities                                      # 403
```
