# Third-party notices

Codex Manager is distributed under Apache-2.0. Runtime and build dependencies are
listed in `package-lock.json` and `Cargo.lock` when the Rust workspace is resolved.

The release workflow generates machine-readable dependency inventories under
`artifacts/release/sbom/` and `artifacts/release/licenses/`. Those files and this notice must
ship with every public beta. Do not hand-edit generated inventories.

The project does not bundle credentials, user transcripts, Codex configuration, or
local databases in release artifacts.

EasyCLIProxyAPI was inspected as a product-flow and sidecar-lifecycle reference at commit
`79b50b7a2b76607e6ccd01966f4b6d4430a31dcd`. Its GUI repository did not contain a
machine-readable or root license at that revision, so Codex Manager does not copy or
adapt its source, styling, icons, brand assets, configuration schema, or Rust
implementation. Codex Manager's downloader, supervisor, and UI are clean-room code.

CLIProxyAPI is an optional, separately versioned runtime from
<https://github.com/router-for-me/CLIProxyAPI>, licensed under the MIT License. Codex
Manager does not compile, link, modify, or bundle its Go source or binary. After an
explicit user action, the app can download the official prebuilt asset directly from the
upstream GitHub Release, verify its published SHA-256 values, and run it as a restricted
loopback sidecar. The upstream binary is not covered by Codex Manager's Developer ID,
notarization, Tauri updater signature, SBOM, or reproducible-build claims. The audited
integration baseline is CLIProxyAPI `v7.2.145`, tag commit
`d9cea8904b14fbbebb77ef26e98ef08f6b48a724`; the actually installed version is shown
separately in the application.
