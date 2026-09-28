Microsoft Visual C++ 2015-2022 runtime DLLs (x64) for the engines.

The Windows bundle workflow copies the redistributable DLLs from the build
machine's Visual Studio "Redist" folder into this directory before
`tauri build` (see .github/workflows/bundle.yml). They are shipped as a Tauri
resource and copied next to sd-server / llama-server only when the system does
not already have them. The DLLs are not committed to git.
