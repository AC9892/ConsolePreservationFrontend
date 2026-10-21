# Rust license text provenance

These files fill gaps in the locally cached Rust crate packages. They are license texts only, not replacement source code. The collector verifies the cached crate's `.cargo_vcs_info.json` revision before using each override.

| Cached crate | Crate source revision | License text source |
| --- | --- | --- |
| alloc-stdlib 0.3.0 | `0a81fd6928ea3b33c8cd484aa4575d50ffb98012` | [Dropbox repository LICENSE at that revision](https://github.com/dropbox/rust-alloc-no-stdlib/blob/0a81fd6928ea3b33c8cd484aa4575d50ffb98012/LICENSE) |
| defmt-parser 1.0.0 | `4a8cdb44891ed57b8ff5a023b6bec7137c48708f` | [MIT](https://github.com/knurling-rs/defmt/blob/4a8cdb44891ed57b8ff5a023b6bec7137c48708f/LICENSE-MIT) and [Apache 2.0](https://github.com/knurling-rs/defmt/blob/4a8cdb44891ed57b8ff5a023b6bec7137c48708f/LICENSE-APACHE) files at that revision |
| webview2-com 0.39.1 | `edc2caf886175ccaebe86078c9cfe1ae2a187328` | [Repository LICENSE at that revision](https://github.com/wravery/webview2-rs/blob/edc2caf886175ccaebe86078c9cfe1ae2a187328/LICENSE) |
| webview2-com-sys 0.39.1 | `edc2caf886175ccaebe86078c9cfe1ae2a187328` | Same repository LICENSE and revision |
| webview2-com-macros 0.8.1 | `dffa41a8a46d3f5565eefbff2de57d38d399f158` | [Repository LICENSE at that revision](https://github.com/wravery/webview2-rs/blob/dffa41a8a46d3f5565eefbff2de57d38d399f158/LICENSE) |
| selectors 0.38.0 | `572ecba2d1600e7c3d490586692a209faf703baa` | The cached selectors crate and its Stylo repository revision have no root license file. `LICENSE-MPL-2.0` is the standard MPL 2.0 text copied from the locally cached `cssparser` 0.37.0 package. The selectors crate declares `MPL-2.0`; this substitution still needs human notice review. |

The CPF project license and final third-party notice review remain open release decisions.
