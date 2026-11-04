# CPF 2DS public preview

**Executable builds and download archives are published only through [GitHub Releases](https://github.com/AC9892/ConsolePreservationFrontend/releases).** This branch contains source, public assets, notices, and documentation.

Download the complete Windows package from the 2DS preview release. Keep
`2DS.exe`, `cpf.exe`, and `cpf-preview.exe` together after extraction.

[Setup instructions](INSTALLATION.md) · [Known issues](KNOWN_ISSUES.md)

Experimental Windows frontend for user-supplied original USA 2DS materials declared as 6.0.0-12U. This is a limited preview, not a complete firmware emulator or a verified playable release.

This package contains CPF only. It contains no Nintendo firmware, NAND, keys, games, console identity, private profiles, saves, logs, Azahar executable, or development artwork. Supply your own separate portable backend and original HOME Menu materials. Read INSTALLATION.md and KNOWN_ISSUES.md before use.

Start 2DS.exe. Guided setup identifies the original HOME Menu CXI, its separate matching backup CXI, and azahar.exe. CPF generates the private profile and prepares a fresh runtime for you. Cancelling a picker exits. cpf-preview.exe opens the original geometric public shell without console data. cpf.exe exposes developer commands; run it without arguments for usage.

Source: a Windows Rust MSVC toolchain and Windows SDK/resource compiler are required. Build with `cargo build --release --offline --features public-release --bins` after extracting the accompanying source archive. Dependencies and their license files are included under vendor. The release scripts additionally use Python 3.11 or newer and produce separate source and binary candidates, remapped build paths, a statically linked CRT, checksums, and an audit report. Publish generated executables and archives only as release assets, never in Git branches. Keep development and user runtime directories private.

CPF code and original public geometry: GPL-3.0-only, matching the DSi CPF release track. See LICENSE. No Nintendo affiliation is implied.
