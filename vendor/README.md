# Vendored dependencies

This directory contains external Rust packages needed to build PipeSQL and its
development tools offline. [Cargo.lock](../Cargo.lock) pins the selected versions;
[.cargo/config.toml](../.cargo/config.toml) directs Cargo here instead of crates.io.
The Rust toolchain and native platform headers are installed separately.

## Structure

Each package has its own directory containing its manifest, source, upstream
notices and `.cargo-checksum.json`. That checksum file identifies the package and
its source files for Cargo's directory-source validation. This root README is
PipeSQL documentation, outside those package inventories.

| Packages | Use in PipeSQL |
| --- | --- |
| [libc](libc/) | Native ABI bindings for the filesystem boundary, CLI and development drivers. |
| [serde_json](serde_json/) and its dependencies | Structured output and result checking in the development runner. |
| [sha2](sha2/) and its dependencies | Development source and artifact hashes. |
| [num-bigint](num-bigint/), [num-rational](num-rational/), [num-traits](num-traits/) and their dependencies | Independent exact arithmetic in development checks. |

The root [Cargo manifest](../Cargo.toml), [filesystem manifest](../filesystem/Cargo.toml)
and [development manifest](../dev/Cargo.toml) distinguish production dependencies
from development dependencies. A package being stored here does not mean it is
linked into the database library.

## Maintain an update

Change dependencies through Cargo and review the resulting lockfile and vendored
packages together. Preserve upstream source bytes, checksum files and license
notices; do not apply first-party formatting or documentation edits inside package
directories. Keep this README when refreshing the package inventory.

Verify affected builds with `--offline --locked` so missing packages or inconsistent
versions fail locally. [THIRD_PARTY.md](../THIRD_PARTY.md) owns attribution, and
[DEVELOPMENT.md](../DEVELOPMENT.md) explains verification and the separate dependency
inputs used when rebuilding the standard library for sanitizer diagnostics.
