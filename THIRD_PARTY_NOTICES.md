# Third-party notices

Codex Manager is distributed under Apache-2.0. Runtime and build dependencies are
listed in `package-lock.json` and `Cargo.lock` when the Rust workspace is resolved.

The release workflow generates machine-readable dependency inventories under
`artifacts/release/sbom/` and `artifacts/release/licenses/`. Those files and this notice must
ship with every public beta. Do not hand-edit generated inventories.

The project does not bundle credentials, user transcripts, Codex configuration, or
local databases in release artifacts.

EasyCLIProxyAPI was inspected only as a product-flow reference. Its GUI repository did
not contain a machine-readable or root license at the inspected revision, so Codex
Manager does not copy or adapt its source, styling, icons, or brand assets. The OAuth
feature and explicit profile switching are clean-room implementations using the official
Codex CLI and App Server; Codex Manager does not adopt EasyCLIProxyAPI's proxy rotation.
