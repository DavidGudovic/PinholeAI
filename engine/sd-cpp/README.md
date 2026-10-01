# Pinhole's sd-server patch

Upstream `sd-server` has no authentication and answers any CORS `Origin`. Pinhole's patch makes
the local engine accept requests only from Pinhole itself (RELEASE-SPEC §12, SPEC §13 "Local
engine API exposure").
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

Pinhole downloads engines from GitHub releases pinned in `config/engine.yaml`. The patched
builds come from David's public fork of leejet/stable-diffusion.cpp (public repos get free
Actions minutes, Windows included; this repo's minutes are limited). The fork holds exactly
this patch and the build workflow; everything else stays upstream's code at the pinned commit.

1. Fork leejet/stable-diffusion.cpp and enable Actions in the fork (forks start with it off).
2. On the fork's default branch add `pinhole/0001-server-api-key-and-reject-origin.patch`
   (this file) and `.github/workflows/pinhole-build.yml` (copy of `pinhole-build.yml` here).
   GitHub only offers "Run workflow" for workflows on the default branch; the workflow
   builds upstream's code at the tag you give it, not the fork's branch.
3. Actions → Pinhole engine build → Run workflow. It builds Linux CPU / Vulkan / CUDA and
   Windows CPU / Vulkan / CUDA, then publishes a release with the zips and `SHA256SUMS.txt`.
   Linux CUDA is new (upstream has none) and is built only for RTX 30/40/50 cards to keep the
   download small; Windows CUDA keeps upstream's architecture list.
4. In Pinhole: point `config/engine.yaml` at that release (sizes and SHA-256 from the release's
   `SHA256SUMS.txt`, cross-checked with the verify-pins workflow) and run the engine smoke test.
   `ENGINE_LOCKDOWN` in `pinhole-core/src/generate.rs` is on, so an unpatched engine won't start.

Current pin: `master-929-3f8527a-pinhole1` from DavidGudovic/stable-diffusion.cpp.

Updating the engine: rerun the workflow with the new upstream tag. If the patch no longer
applies, refresh it against the new tag and copy it here too.

## Checking a build

```
SD_API_KEY=s3cret ./sd-server --diffusion-model <model> --listen-port 18765 --reject-origin
curl -i localhost:18765/sdcpp/v1/capabilities                                   # 401
curl -i -H 'Authorization: Bearer s3cret' localhost:18765/sdcpp/v1/capabilities # 200
curl -i -H 'Authorization: Bearer s3cret' -H 'Origin: https://example.com' \
     localhost:18765/sdcpp/v1/capabilities                                      # 403
```
