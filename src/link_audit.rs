use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::Path,
};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SymbolKey {
    pub nid: String,
    pub library: String,
    pub module: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Symbol {
    pub key: SymbolKey,
    pub library_version: u16,
    pub module_major: u8,
    pub module_minor: u8,
    pub binding: String,
    pub kind: String,
    pub value: u64,
    pub exported: bool,
}
#[derive(Default)]
struct Object {
    imports: Vec<Symbol>,
    exports: Vec<Symbol>,
    notes: Vec<String>,
    declared_import_libraries: Vec<String>,
}
#[derive(Clone, Serialize)]
pub struct Provider {
    pub path: String,
    pub library_version: u16,
    pub module_major: u8,
    pub module_minor: u8,
}
#[derive(Clone, Serialize)]
pub struct Hle {
    pub source: String,
    pub line: usize,
    pub function: String,
    pub library_version: u16,
    pub stub_marker: bool,
    pub activation_verified: bool,
}
#[derive(Serialize)]
pub struct Finding {
    pub symbol: Symbol,
    pub name_hint: Option<String>,
    pub dump_export_candidates: Vec<Provider>,
    pub exact_version_dump_candidates: usize,
    pub hle_source_candidates: Vec<Hle>,
    pub status: String,
}
#[derive(Serialize)]
pub struct Target {
    pub path: String,
    pub imports: usize,
    pub status_counts: BTreeMap<String, usize>,
    pub library_counts: BTreeMap<String, usize>,
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
    pub declared_libraries_without_symbol_imports: Vec<String>,
}
#[derive(Serialize)]
pub struct Audit {
    pub schema_version: u32,
    pub dump_root: String,
    pub backend_source: String,
    pub backend_revision: String,
    pub inspected_elfs: usize,
    pub targets: Vec<Target>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
fn r16(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(p..p.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn r32(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(p..p.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn r64(b: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(p..p.checked_add(8)?)?.try_into().ok()?,
    ))
}
fn string(b: &[u8], p: u64, end: u64) -> Option<String> {
    let end = usize::try_from(end).ok()?.min(b.len());
    let p = usize::try_from(p).ok()?;
    let tail = b.get(p..end)?;
    let length = tail.iter().take(1025).position(|v| *v == 0)?;
    Some(std::str::from_utf8(&tail[..length]).ok()?.to_owned())
}
fn id(mut value: u16) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-";
    let mut bytes = vec![alphabet[(value & 63) as usize]];
    value >>= 6;
    while value != 0 {
        bytes.push(alphabet[(value & 63) as usize]);
        value >>= 6;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap_or_default()
}
fn parse(b: &[u8]) -> Result<Object, String> {
    if !b.starts_with(b"\x7fELF\x02\x01") {
        return Err("Not a readable little-endian ELF64".into());
    }
    let ph = r64(b, 32).ok_or("Truncated ELF header")?;
    let stride = r16(b, 54).ok_or("Truncated ELF header")? as u64;
    let count = r16(b, 56).ok_or("Truncated ELF header")? as u64;
    if stride < 56 || count > 4096 {
        return Err("Invalid program header bounds".into());
    }
    let mut dyninfo = None;
    let mut dynlib = None;
    for i in 0..count {
        let p = usize::try_from(
            ph.checked_add(i * stride)
                .ok_or("Program header overflow")?,
        )
        .map_err(|e| e.to_string())?;
        let t = r32(b, p).ok_or("Truncated program table")?;
        let offset = r64(b, p + 8).ok_or("Truncated program table")?;
        let length = r64(b, p + 32).ok_or("Truncated program table")?;
        if t == 2 {
            dyninfo = Some((offset, length));
        }
        if t == 0x61000000 {
            dynlib = Some((offset, length));
        }
    }
    let Some((dynamic, length)) = dyninfo else {
        return Ok(Object {
            notes: vec!["No dynamic segment".into()],
            ..Default::default()
        });
    };
    if length / 16 > 65536 {
        return Err("Dynamic table exceeds bound".into());
    }
    let mut tags = BTreeMap::new();
    let mut modules = BTreeMap::new();
    let mut libraries = BTreeMap::new();
    let mut declared_libraries = Vec::new();
    for i in 0..length / 16 {
        let p = usize::try_from(
            dynamic
                .checked_add(i * 16)
                .ok_or("Dynamic offset overflow")?,
        )
        .map_err(|e| e.to_string())?;
        let tag = r64(b, p).ok_or("Truncated dynamic table")?;
        let value = r64(b, p + 8).ok_or("Truncated dynamic table")?;
        if tag == 0 {
            break;
        }
        tags.insert(tag, value);
        if tag == 0x6100000f || tag == 0x6100000d {
            modules.insert(id((value >> 48) as u16), value);
        }
        if tag == 0x61000015 || tag == 0x61000013 {
            libraries.insert(id((value >> 48) as u16), value);
        }
        if tag == 0x61000015 {
            declared_libraries.push(value & 0xffffffff);
        }
    }
    let Some((base, segment_len)) = dynlib else {
        return Ok(Object {
            notes: vec!["No SCE dynamic data; standard ELF symbols not audited".into()],
            ..Default::default()
        });
    };
    let offset = *tags.get(&0x61000035).ok_or("Missing SCE string table")?;
    let size = *tags.get(&0x61000037).ok_or("Missing SCE string size")?;
    let strings = base.checked_add(offset).ok_or("String offset overflow")?;
    let string_end = strings.checked_add(size).ok_or("String size overflow")?;
    let segment_end = base.checked_add(segment_len).ok_or("Segment overflow")?;
    if offset.checked_add(size).is_none_or(|v| v > segment_len) || string_end > b.len() as u64 {
        return Err("SCE string table outside available dynamic data".into());
    }
    let symoffset = *tags.get(&0x61000039).ok_or("Missing SCE symbols")?;
    let symsize = *tags.get(&0x6100003f).ok_or("Missing SCE symbol size")?;
    let syment = *tags.get(&0x6100003b).unwrap_or(&24);
    if !(24..=256).contains(&syment) || symsize % syment != 0 || symsize / syment > 1_000_000 {
        return Err("Invalid symbol table stride or count".into());
    }
    let symbols = base
        .checked_add(symoffset)
        .ok_or("Symbol offset overflow")?;
    if symbols
        .checked_add(symsize)
        .is_none_or(|v| v > segment_end || v > b.len() as u64)
    {
        return Err("SCE symbols outside available dynamic data".into());
    }
    let mut object = Object::default();
    for offset in declared_libraries {
        object
            .declared_import_libraries
            .push(string(b, strings + offset, string_end).ok_or("Invalid declared library name")?);
    }
    let mut skipped = BTreeSet::new();
    for i in 0..symsize / syment {
        let p = (symbols + i * syment) as usize;
        let nameoff = r32(b, p).ok_or("Truncated symbol")? as u64;
        let info = *b.get(p + 4).ok_or("Truncated symbol")?;
        let section = r16(b, p + 6).ok_or("Truncated symbol")?;
        let value = r64(b, p + 8).ok_or("Truncated symbol")?;
        let bind = info >> 4;
        let kind = info & 15;
        if bind != 1 && bind != 2 {
            continue;
        }
        let encoded = string(
            b,
            strings.checked_add(nameoff).ok_or("Symbol name overflow")?,
            string_end,
        )
        .ok_or("Invalid symbol name offset or encoding")?;
        let parts: Vec<_> = encoded.split('#').collect();
        if parts.len() != 3 {
            skipped.insert("Non-SCE-encoded global/weak names skipped".into());
            continue;
        }
        let Some(lib) = libraries.get(parts[1]) else {
            skipped.insert(format!("Unknown library ID {}", parts[1]));
            continue;
        };
        let Some(module) = modules.get(parts[2]) else {
            skipped.insert(format!("Unknown module ID {}", parts[2]));
            continue;
        };
        let libname =
            string(b, strings + (*lib & 0xffffffff), string_end).ok_or("Invalid library name")?;
        let modname =
            string(b, strings + (*module & 0xffffffff), string_end).ok_or("Invalid module name")?;
        let symbol = Symbol {
            key: SymbolKey {
                nid: parts[0].into(),
                library: libname,
                module: modname,
            },
            library_version: ((lib >> 32) & 0xffff) as u16,
            module_major: ((module >> 40) & 255) as u8,
            module_minor: ((module >> 32) & 255) as u8,
            binding: if bind == 2 { "weak" } else { "global" }.into(),
            kind: match kind {
                0 => "unspecified",
                1 => "object",
                2 => "function",
                6 => "tls",
                _ => "other",
            }
            .into(),
            value,
            exported: section != 0,
        };
        if symbol.exported {
            object.exports.push(symbol);
        } else {
            object.imports.push(symbol);
        }
    }
    object.notes.extend(skipped);
    Ok(object)
}
fn reparse(path: &Path) -> Result<bool, String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Ok(true);
        }
    }
    Ok(m.file_type().is_symlink())
}
fn read_elf(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut magic = [0; 6];
    if file.read(&mut magic).map_err(|e| e.to_string())? != 6 || magic != *b"\x7fELF\x02\x01" {
        return Ok(None);
    }
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len > 128 * 1024 * 1024 {
        return Err("ELF exceeds 128 MiB audit bound".into());
    }
    let mut b = magic.to_vec();
    file.take(128 * 1024 * 1024)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    Ok(Some(b))
}
fn strip_comments(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = source.as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < b.len() {
        if quoted {
            if b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if b[i] == b'"' {
            quoted = true;
            i += 1;
            continue;
        }
        if b.get(i..i + 2) == Some(b"//") {
            let start = i;
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                i += 1
            }
            for v in &mut out[start..i] {
                *v = b' ';
            }
        } else if b.get(i..i + 2) == Some(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < b.len() && b.get(i..i + 2) != Some(b"*/") {
                i += 1
            }
            i = (i + 2).min(b.len());
            for v in &mut out[start..i] {
                if *v != b'\n' {
                    *v = b' ';
                }
            }
        } else {
            i += 1
        }
    }
    String::from_utf8(out).unwrap_or_default()
}
type SourceIndex = (BTreeMap<SymbolKey, Vec<Hle>>, BTreeMap<String, String>);
fn source_index(root: &Path) -> Result<SourceIndex, String> {
    let mut index: BTreeMap<SymbolKey, Vec<Hle>> = BTreeMap::new();
    let mut names = BTreeMap::new();
    let macros=regex::Regex::new(r#"LIB_(?:FUNCTION|OBJ)\s*\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*(\d+)\s*,\s*"([^"]+)"\s*,\s*([\w:]+)\s*\)"#).map_err(|e|e.to_string())?;
    for item in WalkDir::new(root.join("src/core/libraries")).follow_links(false) {
        let item = item.map_err(|e| e.to_string())?;
        if reparse(item.path())? {
            return Err("Linked backend source refused".into());
        }
        if !item.file_type().is_file()
            || item.path().extension().and_then(|s| s.to_str()) != Some("cpp")
        {
            continue;
        }
        let raw = fs::read_to_string(item.path()).map_err(|e| e.to_string())?;
        let text = strip_comments(&raw);
        for capture in macros.captures_iter(&text) {
            let function = capture[5].to_string();
            let leaf = function.rsplit("::").next().unwrap_or(&function);
            let pattern = format!(r"\b{}\s*\([^;{{}}]*\)\s*\{{", regex::escape(leaf));
            let definition = regex::Regex::new(&pattern).map_err(|e| e.to_string())?;
            let mut stub = false;
            if let Some(definition) = definition.find(&text) {
                let tail = &text[definition.end()..];
                let end = tail.find('\n').unwrap_or(tail.len());
                let first_lines = tail.lines().take(6).collect::<Vec<_>>().join("\n");
                stub = first_lines.contains("STUBBED") || tail[..end].contains("UNREACHABLE");
            }
            let registration = Hle {
                source: item
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(item.path())
                    .to_string_lossy()
                    .replace('\\', "/"),
                line: text[..capture.get(0).unwrap().start()]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    + 1,
                function,
                library_version: capture[3].parse().unwrap_or(0),
                stub_marker: stub,
                activation_verified: false,
            };
            let key = SymbolKey {
                nid: capture[1].into(),
                library: capture[2].into(),
                module: capture[4].into(),
            };
            index.entry(key).or_default().push(registration);
        }
    }
    let database =
        fs::read_to_string(root.join("src/core/aerolib/aerolib.inl")).map_err(|e| e.to_string())?;
    let pattern = regex::Regex::new(r#"STUB\s*\(\s*"([^"]+)"\s*,\s*([^\s,)]+)\s*\)"#)
        .map_err(|e| e.to_string())?;
    for c in pattern.captures_iter(&database) {
        names.insert(c[1].into(), c[2].into());
    }
    Ok((index, names))
}
pub fn audit(dump: &Path, backend: &Path) -> Result<Audit, String> {
    if reparse(dump)? || reparse(backend)? {
        return Err("Use real dump/backend source roots".into());
    }
    let dump = fs::canonicalize(dump).map_err(|e| e.to_string())?;
    let backend = fs::canonicalize(backend).map_err(|e| e.to_string())?;
    let revision = fs::read_to_string(backend.join("REVISION"))
        .map_err(|_| "Backend source must have a REVISION provenance file".to_string())?;
    if revision.trim() != "529ceb32f5546b820588f194b903cc39d08015fe" {
        return Err("Expected source for the pinned shadPS4 v0.20.0 backend".into());
    }
    let (hle, names) = source_index(&backend)?;
    let mut report=Audit{schema_version:1,dump_root:dump.to_string_lossy().into_owned(),backend_source:backend.to_string_lossy().into_owned(),backend_revision:revision.trim().into(),inspected_elfs:0,targets:vec![],diagnostics:vec![],limitations:vec!["Read-only static analysis; no guest execution or relocation simulation.".into(),"Dump export candidates are global to the selected tree, not guaranteed reachable or loaded at runtime.".into(),"Source registrations are candidates only: build flags, registration activation, wrappers and indirect macros are not evaluated.".into(),"Stub marker detection is a source heuristic; absence of a marker is not evidence of correct behavior.".into(),"AeroLib entries supply name hints only, not implementations.".into(),"Version equality is reported separately; shadPS4's current resolver compares NID/library/module names and ignores versions and type.".into()]};
    let mut exports: BTreeMap<SymbolKey, Vec<Provider>> = BTreeMap::new();
    let mut targets = Vec::new();
    let mut walker = WalkDir::new(&dump)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();
    while let Some(item) = walker.next() {
        let item = item.map_err(|e| e.to_string())?;
        if reparse(item.path())? {
            if item.file_type().is_dir() {
                walker.skip_current_dir();
            }
            report
                .diagnostics
                .push(format!("Reparse point skipped: {}", item.path().display()));
            continue;
        }
        if !item.file_type().is_file() {
            continue;
        }
        let path = item
            .path()
            .strip_prefix(&dump)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let result = read_elf(item.path()).and_then(|data| data.map(|b| parse(&b)).transpose());
        match result {
            Ok(Some(object)) => {
                report.inspected_elfs += 1;
                for s in &object.exports {
                    exports.entry(s.key.clone()).or_default().push(Provider {
                        path: path.clone(),
                        library_version: s.library_version,
                        module_major: s.module_major,
                        module_minor: s.module_minor,
                    });
                }
                if audit_target(&path) {
                    targets.push((path, object));
                }
            }
            Ok(None) => {}
            Err(err) => report.diagnostics.push(format!("{path}: {err}")),
        }
    }
    for (path, object) in targets {
        let mut target = Target {
            path,
            imports: object.imports.len(),
            status_counts: BTreeMap::new(),
            library_counts: BTreeMap::new(),
            findings: vec![],
            notes: object.notes,
            declared_libraries_without_symbol_imports: object
                .declared_import_libraries
                .into_iter()
                .filter(|library| {
                    !object
                        .imports
                        .iter()
                        .any(|symbol| &symbol.key.library == library)
                })
                .collect(),
        };
        for symbol in object.imports {
            let dump_candidates = exports.get(&symbol.key).cloned().unwrap_or_default();
            let hle_candidates = hle.get(&symbol.key).cloned().unwrap_or_default();
            let status = if dump_candidates.is_empty() && hle_candidates.is_empty() {
                if symbol.binding == "weak" {
                    "unmatched-weak"
                } else {
                    "unmatched-global"
                }
            } else if !dump_candidates.is_empty() && !hle_candidates.is_empty() {
                if hle_candidates.iter().all(|h| h.stub_marker) {
                    "dump-export-and-hle-stub-candidates"
                } else {
                    "dump-export-and-hle-source-candidates"
                }
            } else if !dump_candidates.is_empty() {
                "dump-export-candidate"
            } else if hle_candidates.iter().all(|h| h.stub_marker) {
                "hle-stub-candidate"
            } else {
                "hle-source-candidate"
            };
            *target.status_counts.entry(status.into()).or_default() += 1;
            *target
                .library_counts
                .entry(symbol.key.library.clone())
                .or_default() += 1;
            target.findings.push(Finding {
                name_hint: names.get(&symbol.key.nid).cloned(),
                exact_version_dump_candidates: dump_candidates
                    .iter()
                    .filter(|provider| {
                        provider.library_version == symbol.library_version
                            && provider.module_major == symbol.module_major
                            && provider.module_minor == symbol.module_minor
                    })
                    .count(),
                symbol,
                dump_export_candidates: dump_candidates,
                hle_source_candidates: hle_candidates,
                status: status.into(),
            });
        }
        report.targets.push(target);
    }
    if report
        .targets
        .iter()
        .filter(|t| {
            t.path == "system_ex/app/NPXS20001/eboot.bin" || t.path == "system/vsh/SceShellCore.elf"
        })
        .count()
        != 2
    {
        return Err("Both ShellUI and ShellCore readable ELF entry points are required".into());
    }
    Ok(report)
}
fn audit_target(path: &str) -> bool {
    [
        "system_ex/app/NPXS20001/eboot.bin",
        "system/vsh/SceShellCore.elf",
        "system/common/lib/libSceSystemService.sprx",
        "system/common/lib/libSceIpmi.sprx",
        "system/common/lib/libSceMbus.sprx",
        "system/common/lib/libScePsm.sprx",
        "system/common/lib/libSceRegMgr.sprx",
        "system/common/lib/libSceSysCore.sprx",
        "system/priv/lib/libSceLoginMgrServer.sprx",
        "system/common/lib/SecureUIProcess.self",
        "system/common/lib/SecureWebProcess.self",
        "system/common/lib/orbis-jsc-compiler.self",
    ]
    .contains(&path)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut data = vec![0u8; 2048];
        data[..6].copy_from_slice(b"\x7fELF\x02\x01");
        data[32..40].copy_from_slice(&64u64.to_le_bytes());
        data[54..56].copy_from_slice(&56u16.to_le_bytes());
        data[56..58].copy_from_slice(&2u16.to_le_bytes());
        data[64..68].copy_from_slice(&2u32.to_le_bytes());
        data[72..80].copy_from_slice(&192u64.to_le_bytes());
        data[96..104].copy_from_slice(&128u64.to_le_bytes());
        data[120..124].copy_from_slice(&0x61000000u32.to_le_bytes());
        data[128..136].copy_from_slice(&512u64.to_le_bytes());
        data[152..160].copy_from_slice(&1536u64.to_le_bytes());
        let library = 128u64 | (1 << 32) | (1 << 48);
        let module = 144u64 | (2 << 32) | (3 << 40) | (2 << 48);
        for (i, (tag, value)) in [
            (0x61000035u64, 0u64),
            (0x61000037, 256),
            (0x61000039, 256),
            (0x6100003f, 72),
            (0x6100003b, 24),
            (0x61000015, library),
            (0x6100000f, module),
        ]
        .iter()
        .enumerate()
        {
            let offset = 192 + i * 16;
            data[offset..offset + 8].copy_from_slice(&tag.to_le_bytes());
            data[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
        }
        data[513..521].copy_from_slice(b"NID#B#C\0");
        data[528..536].copy_from_slice(b"EXP#B#C\0");
        data[640..648].copy_from_slice(b"libTest\0");
        data[656..667].copy_from_slice(b"moduleTest\0");
        data[792..796].copy_from_slice(&1u32.to_le_bytes());
        data[796] = 0x12;
        data[816..820].copy_from_slice(&16u32.to_le_bytes());
        data[820] = 0x22;
        data[822..824].copy_from_slice(&1u16.to_le_bytes());
        data[824..832].copy_from_slice(&0x1000u64.to_le_bytes());
        data
    }
    #[test]
    fn decodes_sce_imports_exports_ids_versions_and_binding() {
        let object = parse(&fixture()).unwrap();
        assert_eq!(object.imports.len(), 1);
        assert_eq!(object.exports.len(), 1);
        let import = &object.imports[0];
        assert_eq!(
            import.key,
            SymbolKey {
                nid: "NID".into(),
                library: "libTest".into(),
                module: "moduleTest".into()
            }
        );
        assert_eq!(
            (
                import.library_version,
                import.module_major,
                import.module_minor
            ),
            (1, 3, 2)
        );
        assert_eq!(object.exports[0].binding, "weak");
        let mut truncated = fixture();
        truncated.truncate(800);
        assert!(parse(&truncated).is_err());
        let mut invalid = fixture();
        invalid[792..796].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&invalid).is_err());
    }
    #[test]
    fn ids_and_comment_boundaries() {
        assert_eq!(id(0), "A");
        assert_eq!(id(63), "-");
        assert_eq!(id(64), "BA");
        assert_eq!(id(4096), "BAA");
        let source =
            "// LIB_FUNCTION(fake)\nconst char* s=\"https://valid\"; /* disabled */\nreal();";
        let cleaned = strip_comments(source);
        assert!(!cleaned.contains("fake"));
        assert!(cleaned.contains("https://valid"));
        assert_eq!(source.lines().count(), cleaned.lines().count());
    }
    #[test]
    fn malformed_elf_tables_do_not_panic() {
        let mut data = vec![0u8; 64];
        data[..6].copy_from_slice(b"\x7fELF\x02\x01");
        assert!(parse(&data).is_err());
        data[54..56].copy_from_slice(&56u16.to_le_bytes());
        data[56..58].copy_from_slice(&2u16.to_le_bytes());
        data[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&data).is_err());
    }
}
