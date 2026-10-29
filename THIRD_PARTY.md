# Source attribution and dependencies

## Xenia Canary

Upstream: https://github.com/xenia-canary/xenia-canary
Pinned revision: `2372a54105d9d598ab5bc7ba94a72ef1bea4b271`
License: BSD 3-clause, reproduced in `xenia/LICENSE.xenia`.

The `xenia` files are modified upstream source with notices retained, except the two explicitly marked helper-only views. The canonical patch records the actual modifications. New dashboard crypto, synthetic AES tests and lifecycle changes retain the Xenia BSD terms; helper views omit existing upstream key constants. This repository is not an upstream Xenia release. Upstream dependencies are fetched separately with their existing licenses and notices; they are not vendored here.

## CPF Xbox 360 Rust module

The existing local CPF module is published under `monitor`, with Cargo.toml and Cargo.lock. Its Cargo dependencies are downloaded through the Rust package registry, not copied into this source upload. Their applicable licenses remain with their respective authors. This publication does not introduce a new project-wide license grant for CPF-owned source; the Xenia license applies to the Xenia-derived files and patch.

The original developer utility sources in the local External Tools directory are not publication inputs. No proprietary SDK, firmware extraction, console certificate, private key, game asset, imported collection or emulator log is included.
