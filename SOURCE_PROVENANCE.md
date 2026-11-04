# Source provenance

The checked-in Rust files match the source used for this public candidate.
The matching source archive, distributed through GitHub Releases, additionally contains vendored locked dependencies.
The Windows executables were built with the public-release feature, stripped
symbols, remapped build paths, and a statically linked MSVC CRT.
Original geometric assets replace development images. Azahar and private console
material are excluded. Build cloned repository source with `cargo build --release
--features public-release --bins`; omit `--offline` unless dependencies are cached.
The complete source archive can build offline using its vendored dependencies.

This is a limited preview. See KNOWN_ISSUES.md; game HOME suspension is unresolved.
