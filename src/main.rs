use cpf::{
    arm::{ArmProbe, KernelProbe, KernelStop, StopReason},
    azahar::AzaharBackend,
    catalog::{
        read_installed_launch_links, read_installed_title_tree, read_title_directory,
        LaunchLinkStatus,
    },
    collect,
    firmware::FirmwareConfig,
    runtime_diagnostics::read_ns_launch_report,
    verify_raw_sha256_sidecar, CiaMetadataCheck, ExecutableLayoutCheck, HeaderCheck, Kind, Model,
    NcsdImage, ProcessImage, TitleFormat,
};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::Path;

fn main() {
    let mut args = env::args_os();
    let _program = args.next();
    let Some(command) = args.next() else {
        usage();
        return;
    };
    match command.to_string_lossy().as_ref() {
        "prepare-public-runtime" => {
            let (Some(home), Some(backup), Some(backend)) = (args.next(), args.next(), args.next())
            else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            cpf::public_setup::prepare(Path::new(&home), Path::new(&backup), Path::new(&backend))
                .unwrap_or_else(|message| {
                    eprintln!("{message}");
                    std::process::exit(1);
                });
            println!("Private runtime prepared: HOME backup verified, return copy checked, and profile generated inside user/cpf-setup. Original sources were not modified.");
        }
        "runtime-failure-report" => {
            let Some(log) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let report = cpf::runtime_diagnostics::read_failure_report(Path::new(&log))
                .unwrap_or_else(|_| {
                    eprintln!("Unable to read a bounded UTF-8 runtime log");
                    std::process::exit(1);
                });
            println!("Fatal error calls: {}", report.fatal_calls);
            for ((svc, result), count) in report.svc_results {
                println!("SVC 0x{svc:02X}, result 0x{result:08X}: {count}");
            }
            println!("Counts only; these records do not establish the cause or identify the invalid object. No files changed.");
        }
        "sd-check" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let package =
                cpf::sd_media::SdPackage::open(Path::new(&source)).unwrap_or_else(|error| {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                });
            println!(
                "Base SD application CIA verified: title {:016X}, title version {}.",
                package.title_id(),
                package.title_version()
            );
            println!("CIA metadata/content hashes and main executable layout checked. Signatures, firmware compatibility, installation, and gameplay remain unverified. No files changed.");
        }
        "sd-install" | "sd-verify" => {
            let (Some(profile), Some(executable), Some(source)) =
                (args.next(), args.next(), args.next())
            else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let config = FirmwareConfig::load(Path::new(&profile)).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            let package =
                cpf::sd_media::SdPackage::open(Path::new(&source)).unwrap_or_else(|error| {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                });
            let backend = AzaharBackend::new(Path::new(&executable)).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            let system = backend.system_settings();
            if !config.accepts_backend_system(system.is_new_3ds, system.region_value) {
                eprintln!("Azahar portable settings do not match the selected firmware target");
                std::process::exit(1);
            }
            if command == "sd-verify" {
                backend
                    .verify_sd_install_idle(&package)
                    .unwrap_or_else(|error| {
                        eprintln!("{}", error.label());
                        std::process::exit(1);
                    });
                println!("Installed base metadata and all included content files match the CIA. No title files changed. Launch, saves, and gameplay remain unverified.");
                return;
            }
            let mut session = backend.start_sd_install(&package).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            println!("Installing a verified base application into the private virtual SD; completion is followed by content verification.");
            match session.wait() {
                Ok(status) if status.success() => {}
                _ => {
                    eprintln!("SD installer did not complete successfully; inspect private runtime state before retrying.");
                    std::process::exit(1);
                }
            }
            backend.verify_sd_install(&package).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            println!("Installed base metadata and all included content files match the verified CIA; the source hash is unchanged. HOME discovery, launch, saves, and gameplay require a live test.");
        }
        "cartridge-check" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let cartridge = cpf::cartridge::CartridgeImage::open(Path::new(&source))
                .unwrap_or_else(|error| {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                });
            println!("NCSD cartridge preflight passed: {} bounded partitions; decrypted executable first partition; trimmed media: {}", cartridge.partition_count, cartridge.trimmed);
            println!("Structure checked only. Internal content hashes, signatures, firmware compatibility, mounting, and gameplay are not established. No files changed.");
        }
        "cartridge-select" | "cartridge-eject" => {
            let Some(executable) = args.next() else {
                usage();
                return;
            };
            let cartridge = if command == "cartridge-select" {
                let Some(source) = args.next() else {
                    usage();
                    return;
                };
                Some(
                    cpf::cartridge::CartridgeImage::open(Path::new(&source)).unwrap_or_else(
                        |error| {
                            eprintln!("{}", error.label());
                            std::process::exit(1);
                        },
                    ),
                )
            } else {
                None
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let backend = AzaharBackend::new(Path::new(&executable)).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            backend
                .set_startup_cartridge(cartridge.as_ref())
                .unwrap_or_else(|error| {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                });
            if cartridge.is_some() {
                println!("Cartridge selected for the next HOME startup. The source image was opened read-only and was not copied.");
            } else {
                println!("Cartridge ejected for the next HOME startup.");
            }
            println!("Restart the CPF session to apply this selection. Insertion, launch, and gameplay require a live test.");
        }
        "installed-title-report" => {
            let Some(directory) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let catalog = match read_installed_title_tree(Path::new(&directory)) {
                Ok(catalog) => catalog,
                Err(_) => {
                    eprintln!("installed title tree unavailable");
                    std::process::exit(1);
                }
            };
            let ids: BTreeSet<u64> = catalog.titles.iter().map(|title| title.title_id).collect();
            println!("Validated installed NCCH contents: {}; distinct title IDs: {}; rejected or missing candidates: {}", catalog.titles.len(), ids.len(), catalog.rejected);
            for id in ids {
                println!("{id:016X}");
            }
            let links = match read_installed_launch_links(Path::new(&directory)) {
                Ok(links) => links,
                Err(_) => {
                    eprintln!("installed launch metadata unavailable");
                    std::process::exit(1);
                }
            };
            let ready = links
                .iter()
                .filter(|link| link.status == LaunchLinkStatus::Ready)
                .count();
            println!("TMD-linked main contents: {ready}/{} verified", links.len());
            let fallback = links
                .iter()
                .filter(|link| link.status == LaunchLinkStatus::FallbackVerified)
                .count();
            if fallback != 0 {
                println!("Internally verified NCCH fallbacks without TMD: {fallback}");
            }
            for link in links {
                println!("{:016X}: {}", link.title_id, link.status.label());
            }
            println!("This checks installed content and its base TMD link, not HOME Menu selection or launch. No files changed.");
        }
        "title-catalog" => {
            let Some(directory) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let catalog = match read_title_directory(Path::new(&directory)) {
                Ok(catalog) => catalog,
                Err(_) => {
                    eprintln!("title directory unavailable");
                    std::process::exit(1);
                }
            };
            println!(
                "Validated local title exports: {}; rejected or unreadable candidates: {}",
                catalog.titles.len(),
                catalog.rejected
            );
            for title in catalog.titles {
                match title.format {
                    TitleFormat::Cia => println!(
                        "{:016X} CIA title-version {}",
                        title.title_id,
                        title.title_version.unwrap()
                    ),
                    TitleFormat::Ncch => println!(
                        "{:016X} NCCH header-version {} (title version unavailable)",
                        title.title_id,
                        title.ncch_version.unwrap()
                    ),
                }
            }
            println!("Internal structure and hashes checked; title signatures and firmware version not authenticated. No files changed.");
        }
        "home-service-report" => {
            let (Some(log), Some(directory)) = (args.next(), args.next()) else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let report = match read_ns_launch_report(Path::new(&log)) {
                Ok(report) => report,
                Err(_) => {
                    eprintln!("private runtime log unavailable or too large");
                    std::process::exit(1);
                }
            };
            let catalog = match read_title_directory(Path::new(&directory)) {
                Ok(catalog) => catalog,
                Err(_) => {
                    eprintln!("title directory unavailable");
                    std::process::exit(1);
                }
            };
            let available: BTreeSet<u64> =
                catalog.titles.iter().map(|title| title.title_id).collect();
            let matched = report.module_titles.intersection(&available).count();
            println!(
                "Unimplemented ns:s LaunchTitle calls: {}; distinct title IDs: {}",
                report.calls,
                report.unique_titles.len()
            );
            println!(
                "System-module IDs requested: {}; matching validated exports: {}",
                report.module_titles.len(),
                matched
            );
            println!(
                "Other title IDs requested: {}; rejected title candidates: {}",
                report.other_titles.len(),
                catalog.rejected
            );
            println!("This startup service report does not establish whether a selected application can launch. No log lines, paths, or file contents shown; no files changed.");
        }
        "prepare-home-return" => {
            let (Some(profile), Some(backup), Some(executable)) =
                (args.next(), args.next(), args.next())
            else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let config = FirmwareConfig::load(Path::new(&profile)).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            let home = config.validate_home_menu().unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            let backend = AzaharBackend::new(Path::new(&executable)).unwrap_or_else(|error| {
                eprintln!("{}", error.label());
                std::process::exit(1);
            });
            let system = backend.system_settings();
            if !config.accepts_backend_system(system.is_new_3ds, system.region_value) {
                eprintln!("Azahar portable settings do not match the selected firmware target");
                std::process::exit(1);
            }
            match backend.prepare_home_return(&home, Path::new(&backup)) {
                Ok(cpf::runtime_setup::ReturnTitleState::Created) => {
                    println!("HOME return working copy created; source, backup, and copied SHA-256 match.");
                }
                Ok(cpf::runtime_setup::ReturnTitleState::AlreadyVerified) => {
                    println!("Existing HOME return working copy matches the selected source and backup. No title files changed.");
                }
                Err(error) => {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                }
            }
            println!("Preparation does not establish applet return functionality.");
        }
        "home-check" | "home-launch" | "home-shell" => {
            let shell = command == "home-shell";
            let launch = command == "home-launch" || shell;
            let Some(profile_path) = args.next() else {
                usage();
                return;
            };
            let executable = if launch {
                let Some(path) = args.next() else {
                    usage();
                    return;
                };
                Some(path)
            } else {
                None
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let config = match FirmwareConfig::load(Path::new(&profile_path)) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                }
            };
            let home = match config.validate_home_menu() {
                Ok(home) => home,
                Err(error) => {
                    eprintln!("{}", error.label());
                    std::process::exit(1);
                }
            };
            println!("Original 2DS HOME Menu candidate: bounded structure, executable layout, and internal hashes validated");
            println!("Declared firmware {} is profile metadata; title signature and installed firmware version are not authenticated", config.declared_firmware);
            if let Some(executable) = executable {
                let backend = match AzaharBackend::new(Path::new(&executable)) {
                    Ok(backend) => backend,
                    Err(error) => {
                        eprintln!("{}", error.label());
                        std::process::exit(1);
                    }
                };
                let system = backend.system_settings();
                if !config.accepts_backend_system(system.is_new_3ds, system.region_value) {
                    eprintln!("Azahar portable settings do not match the original USA 2DS target");
                    std::process::exit(1);
                }
                let button_keys = if shell {
                    match backend.button_keys() {
                        Ok(keys) => Some(keys),
                        Err(error) => {
                            eprintln!("{}", error.label());
                            std::process::exit(1);
                        }
                    }
                } else {
                    None
                };
                #[cfg(windows)]
                if shell {
                    if let Err(error) = run_home_with_media(&backend, &home, button_keys.unwrap()) {
                        if error.to_string() == cpf::process_lifecycle::BACKEND_EXITED_MESSAGE {
                            eprintln!("{}", cpf::process_lifecycle::BACKEND_EXITED_MESSAGE);
                        } else {
                            eprintln!(
                                "Unable to run live 2DS shell (error kind: {:?})",
                                error.kind()
                            );
                        }
                        std::process::exit(1);
                    }
                    return;
                }
                let mut session = match backend.start(&home) {
                    Ok(session) => session,
                    Err(error) => {
                        eprintln!("{}", error.label());
                        std::process::exit(1);
                    }
                };
                #[cfg(windows)]
                {
                    let window = match session.wait_for_window(std::time::Duration::from_secs(10)) {
                        Ok(Some(window)) => window,
                        Ok(None) => {
                            eprintln!("Azahar window was not found during startup");
                            let _ = session.stop();
                            std::process::exit(1);
                        }
                        Err(error) => {
                            eprintln!("{}", error.label());
                            let _ = session.stop();
                            std::process::exit(1);
                        }
                    };
                    let _ = window;
                    println!("Azahar window ready; HOME Menu rendering and interaction require separate confirmation");
                }
                match session.wait() {
                    Ok(status) if status.success() => {
                        println!("Azahar session exited successfully")
                    }
                    Ok(_) => {
                        eprintln!("Azahar session exited with an error");
                        std::process::exit(1);
                    }
                    Err(error) => {
                        eprintln!("{}", error.label());
                        std::process::exit(1);
                    }
                }
            }
        }
        "inspect" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            match collect(Path::new(&source)) {
                Ok(entries) => {
                    let mut counts: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
                    for entry in &entries {
                        let item = counts.entry(entry.kind.label()).or_default();
                        item.0 += 1;
                        item.1 += entry.size;
                    }
                    println!("Local archive inventory (file names and paths withheld)");
                    println!("Files: {}", entries.len());
                    println!("Bytes: {}", entries.iter().map(|e| e.size).sum::<u64>());
                    for (kind, (count, bytes)) in counts {
                        println!("{kind}: {count} file(s), {bytes} bytes");
                    }
                    let ncch = entries.iter().filter(|e| e.kind == Kind::Ncch).count();
                    println!("Verified NCCH containers: {ncch}");
                    let cia_bounds = entries
                        .iter()
                        .filter(|e| {
                            e.kind == Kind::CiaCandidate
                                && e.header_check == HeaderCheck::BoundsConsistent
                        })
                        .count();
                    println!("CIA candidates with sections within file bounds: {cia_bounds}");
                    let cia_consistent = entries
                        .iter()
                        .filter(|e| e.cia_metadata == CiaMetadataCheck::Consistent)
                        .count();
                    let cia_invalid = entries
                        .iter()
                        .filter(|e| e.cia_metadata == CiaMetadataCheck::Invalid)
                        .count();
                    println!("CIA candidates with matching ticket/TMD title and content sizes: {cia_consistent}");
                    println!("CIA candidates with inconsistent metadata: {cia_invalid}");
                    let tmd_hashes = entries
                        .iter()
                        .filter(|e| e.cia_tmd_hashes == CiaMetadataCheck::Consistent)
                        .count();
                    let tmd_hash_failures = entries
                        .iter()
                        .filter(|e| e.cia_tmd_hashes == CiaMetadataCheck::Invalid)
                        .count();
                    println!("CIA TMDs with matching internal SHA-256 hashes: {tmd_hashes}");
                    println!(
                        "CIA TMDs with mismatched internal SHA-256 hashes: {tmd_hash_failures}"
                    );
                    println!(
                        "CIA content chunks with matching SHA-256 hashes: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.cia_content_verified))
                            .sum::<u64>()
                    );
                    println!(
                        "CIA content chunks with mismatched SHA-256 hashes: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.cia_content_mismatched))
                            .sum::<u64>()
                    );
                    println!(
                        "CIA encrypted content chunks not checked: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.cia_content_skipped_encrypted))
                            .sum::<u64>()
                    );
                    let tickets = entries
                        .iter()
                        .filter(|e| {
                            e.kind == Kind::TicketCandidate
                                && e.header_check == HeaderCheck::BoundsConsistent
                        })
                        .count();
                    println!("Standalone tickets with bounded structure: {tickets}");
                    let partitions: u64 =
                        entries.iter().map(|e| u64::from(e.ncsd_partitions)).sum();
                    println!("NCSD partition entries within image bounds: {partitions}");
                    for status in [
                        HeaderCheck::BoundsConsistent,
                        HeaderCheck::MediaLargerThanFile,
                        HeaderCheck::Truncated,
                        HeaderCheck::InvalidLayout,
                    ] {
                        let count = entries.iter().filter(|e| e.header_check == status).count();
                        println!("Container {}: {count}", status.label());
                    }
                    let system_apps = entries
                        .iter()
                        .filter(|e| e.system_application_candidate)
                        .count();
                    let known_home = entries
                        .iter()
                        .filter(|e| e.known_home_menu_candidate)
                        .count();
                    let encrypted_ncch = entries.iter().filter(|e| e.ncch_encrypted).count();
                    let encrypted_home = entries
                        .iter()
                        .filter(|e| e.known_home_menu_candidate && e.ncch_encrypted)
                        .count();
                    println!("System application NCCH header candidates: {system_apps}");
                    println!("Known JPN/USA/EUR HOME Menu NCCH header candidates: {known_home}");
                    println!("NCCH headers marked encrypted: {encrypted_ncch}");
                    println!(
                        "NCCH internal hash regions matching SHA-256: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.ncch_hash_verified))
                            .sum::<u64>()
                    );
                    println!(
                        "NCCH internal hash regions mismatching SHA-256: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.ncch_hash_mismatched))
                            .sum::<u64>()
                    );
                    println!(
                        "ExeFS files with matching SHA-256 hashes: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.exefs_sections_verified))
                            .sum::<u64>()
                    );
                    println!(
                        "ExeFS files with mismatched SHA-256 hashes: {}",
                        entries
                            .iter()
                            .map(|e| u64::from(e.exefs_sections_mismatched))
                            .sum::<u64>()
                    );
                    println!(
                        "ExeFS containers with invalid file ranges: {}",
                        entries.iter().filter(|e| e.exefs_layout_invalid).count()
                    );
                    println!(
                        "NCCH containers with hash-matched .code: {}",
                        entries.iter().filter(|e| e.exefs_code_verified).count()
                    );
                    println!(
                        "NCCH executable layouts with uncompressed code size matched: {}",
                        entries
                            .iter()
                            .filter(|e| e.executable_layout
                                == ExecutableLayoutCheck::UncompressedConsistent)
                            .count()
                    );
                    println!(
                        "NCCH compressed code streams decoded with matching layout: {}",
                        entries
                            .iter()
                            .filter(|e| e.executable_layout
                                == ExecutableLayoutCheck::CompressedConsistent)
                            .count()
                    );
                    println!(
                        "NCCH compressed code streams not checked due to size limit: {}",
                        entries
                            .iter()
                            .filter(|e| e.executable_layout
                                == ExecutableLayoutCheck::CompressedNotChecked)
                            .count()
                    );
                    println!(
                        "NCCH executable layouts inconsistent: {}",
                        entries
                            .iter()
                            .filter(|e| e.executable_layout == ExecutableLayoutCheck::Invalid)
                            .count()
                    );
                    println!("HOME Menu candidates marked encrypted: {encrypted_home}");
                    println!(
                        "Known HOME Menu candidates with hash-matched .code: {}",
                        entries
                            .iter()
                            .filter(|e| e.known_home_menu_candidate && e.exefs_code_verified)
                            .count()
                    );
                    println!("HOME Menu signature authenticity and launchability: unverified");
                    println!("ARM execution: bounded diagnostic subset only; no system software was started");
                }
                Err(error) => {
                    // std::io::Error can contain a path supplied by the OS, so never print it.
                    eprintln!(
                        "Unable to inspect source archive (error kind: {:?})",
                        error.kind()
                    );
                    std::process::exit(1);
                }
            }
        }
        "profiles" => {
            for model in [
                Model::ThreeDs,
                Model::ThreeDsXl,
                Model::TwoDs,
                Model::NewThreeDs,
                Model::NewThreeDsXl,
                Model::NewTwoDsXl,
            ] {
                println!(
                    "{}: new-series={}, stereoscopic-display={}",
                    model.name(),
                    model.new_series(),
                    model.stereoscopic_display()
                );
            }
        }
        "verify-raw-sha256" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            let Some(sidecar) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            match verify_raw_sha256_sidecar(Path::new(&source), Path::new(&sidecar)) {
                Ok(true) => println!("Raw SHA-256 sidecar matches source file"),
                Ok(false) => {
                    eprintln!("Raw SHA-256 sidecar mismatch");
                    std::process::exit(1);
                }
                Err(error) => {
                    eprintln!(
                        "Unable to verify raw SHA-256 sidecar (error kind: {:?})",
                        error.kind()
                    );
                    std::process::exit(1);
                }
            }
        }
        "probe-ncsd" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let mut image = match NcsdImage::open(Path::new(&source)) {
                Ok(image) => image,
                Err(error) => {
                    eprintln!("Unable to open NCSD image (error kind: {:?})", error.kind());
                    std::process::exit(1);
                }
            };
            let mut readable = 0;
            for index in 0..8 {
                if image.partition_len(index).is_some() {
                    let mut first_byte = [0u8; 1];
                    if let Err(error) = image.read_partition(index, 0, &mut first_byte) {
                        eprintln!(
                            "Unable to read an NCSD partition (error kind: {:?})",
                            error.kind()
                        );
                        std::process::exit(1);
                    }
                    readable += 1;
                }
            }
            println!("NCSD partitions readable within declared bounds: {readable}");
            println!("Partition contents were not decrypted, interpreted, or logged");
        }
        "probe-ncch-code" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let process = match ProcessImage::load(Path::new(&source)) {
                Ok(process) => process,
                Err(error) => {
                    eprintln!(
                        "Unable to load verified NCCH process image (error kind: {:?})",
                        error.kind()
                    );
                    std::process::exit(1);
                }
            };
            let mut first_byte = [0u8; 1];
            if let Err(error) = process.read(process.layout().text.address, &mut first_byte) {
                eprintln!(
                    "Unable to read mapped NCCH entrypoint (error kind: {:?})",
                    error.kind()
                );
                std::process::exit(1);
            }
            println!("Verified NCCH code decoded and mapped in memory: {} bytes; entrypoint readable; no file written", process.layout().code_bytes().unwrap());
        }
        "probe-arm" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let mut process = match ProcessImage::load(Path::new(&source)) {
                Ok(process) => process,
                Err(error) => {
                    eprintln!(
                        "Unable to load verified NCCH process image (error kind: {:?})",
                        error.kind()
                    );
                    std::process::exit(1);
                }
            };
            let mut cpu = ArmProbe::new(&process);
            let report = cpu.run(&mut process, 1_000_000);
            println!("Bounded A32/T16 probe: {} instruction(s) executed; {} distinct addresses fetched; {} in-memory write(s); stopped at {}; no system service invoked or file written", report.executed, cpu.distinct_instruction_addresses(), cpu.memory_writes(), report.stop.label());
            if report.stop == StopReason::SupervisorCall {
                if let Some(number) = cpu.pending_supervisor_call(&process) {
                    let name = match number {
                        0x21 => "CreateAddressArbiter",
                        _ => "unidentified",
                    };
                    let access = match process.svc_allowed(number) {
                        Some(true) => "allowed by exheader mask",
                        Some(false) => "not allowed by exheader mask",
                        None => "exheader mask unavailable",
                    };
                    println!("Pending public SVC 0x{number:02X} ({name}); {access}");
                }
            }
            if report.stop == StopReason::UnsupportedInstruction {
                if let Some(family) = cpu.pending_instruction_family(&process) {
                    println!("Unsupported encoding family: {family}");
                }
            }
        }
        "probe-arm-kernel" => {
            let Some(source) = args.next() else {
                usage();
                return;
            };
            if args.next().is_some() {
                usage();
                return;
            }
            let mut process = match ProcessImage::load(Path::new(&source)) {
                Ok(process) => process,
                Err(error) => {
                    eprintln!(
                        "Unable to load verified NCCH process image (error kind: {:?})",
                        error.kind()
                    );
                    std::process::exit(1);
                }
            };
            let mut cpu = ArmProbe::new(&process);
            let mut kernel = KernelProbe::new();
            let report = cpu.run_with_kernel(&mut process, &mut kernel, 1_000_000);
            println!("Bounded A32/T16/kernel probe: {} instruction(s); {} modeled call(s); {} in-memory address arbiter(s); stopped at {}; no host service invoked or file written", report.executed, report.serviced, kernel.address_arbiter_count(), report.stop.label());
            if let Some(number) = cpu.pending_supervisor_call(&process) {
                let name = match number {
                    0x01 => "ControlMemory",
                    0x21 => "CreateAddressArbiter",
                    _ => "unidentified",
                };
                let access = match process.svc_allowed(number) {
                    Some(true) => "allowed by exheader mask",
                    Some(false) => "not allowed by exheader mask",
                    None => "exheader mask unavailable",
                };
                println!("Pending public SVC 0x{number:02X} ({name}); {access}");
                if number == 0x01 {
                    let operation = cpu.register(0).unwrap_or(0);
                    let kind = match operation & 0xff {
                        1 => "free",
                        2 => "reserve",
                        3 => "commit",
                        4 => "map",
                        5 => "unmap",
                        6 => "protect",
                        _ => "unidentified",
                    };
                    let region = match operation & 0xf00 {
                        0 => "default",
                        0x100 => "application",
                        0x200 => "system",
                        0x300 => "base",
                        _ => "unidentified",
                    };
                    let size = cpu.register(3).unwrap_or(0);
                    let first_address = cpu.register(1).unwrap_or(0);
                    let first_region = match first_address {
                        0x0800_0000..0x1000_0000 => "application heap range",
                        0x1400_0000..0x1c00_0000 => "old-model linear heap range",
                        0x3000_0000..0x4000_0000 => "new-model linear heap range",
                        _ => "other range",
                    };
                    let permission = match cpu.register(4).unwrap_or(0) {
                        0 => "none",
                        1 => "read",
                        2 => "write",
                        3 => "read-write",
                        _ => "other",
                    };
                    println!("ControlMemory request: {kind}; {region} region; linear={}; {} page(s); first address supplied={}; first address range={first_region}; second address supplied={}; permission={permission}",
                        operation & 0x10000 != 0, size / 0x1000, first_address != 0, cpu.register(2).unwrap_or(0) != 0);
                }
            }
            if report.stop == KernelStop::Cpu(StopReason::UnsupportedInstruction) {
                if let Some(family) = cpu.pending_instruction_family(&process) {
                    println!("Unsupported encoding family: {family}");
                    if family.starts_with("MSR ") {
                        let word = process.fetch_u32(cpu.register(15).unwrap()).unwrap();
                        let fields = (word >> 16) & 15;
                        println!(
                            "MSR destination fields: control={}; extension={}; status={}; flags={}",
                            fields & 1 != 0,
                            fields & 2 != 0,
                            fields & 4 != 0,
                            fields & 8 != 0
                        );
                    }
                }
            }
            if report.stop == KernelStop::Cpu(StopReason::ThumbStateRequired) {
                if let Some(readable) = cpu.pending_blx_thumb_entry_readable(&process) {
                    println!("BLX Thumb entrypoint in executable text: {readable}");
                }
            }
        }
        _ => usage(),
    }
}

fn usage() {
    eprintln!("Guided setup equivalent: cpf prepare-public-runtime <original-home.cxi> <separate-backup.cxi> <azahar.exe>");
    eprintln!("Failure diagnostics: cpf runtime-failure-report <private-azahar-log>");
    eprintln!("SD commands: cpf sd-check <game.cia> | cpf sd-install <private-profile.toml> <azahar.exe> <backup-game.cia> | cpf sd-verify <private-profile.toml> <azahar.exe> <backup-game.cia>");
    eprintln!("Cartridge commands: cpf cartridge-check <game.cci|game.3ds> | cpf cartridge-select <azahar.exe> <game.cci|game.3ds> | cpf cartridge-eject <azahar.exe>");
    eprintln!("Runtime preparation: cpf prepare-home-return <private-profile.toml> <verified-backup.cxi> <azahar.exe>");
    eprintln!("Usage: cpf installed-title-report <azahar-nand-title-directory> | cpf title-catalog <local-title-directory> | cpf home-service-report <private-azahar-log> <local-title-directory> | cpf home-check <private-profile.toml> | cpf home-launch <private-profile.toml> <azahar.exe> | cpf home-shell <private-profile.toml> <azahar.exe> | cpf inspect <local-source-directory> | cpf verify-raw-sha256 <local-file> <raw-32-byte-sidecar> | cpf probe-ncsd <local-file> | cpf probe-ncch-code <local-file> | cpf probe-arm <local-file> | cpf probe-arm-kernel <local-file> | cpf profiles");
}

#[cfg(windows)]
fn run_home_with_media(
    backend: &AzaharBackend,
    home: &cpf::firmware::ValidatedHomeMenu,
    keys: cpf::azahar::AzaharButtonKeys,
) -> std::io::Result<()> {
    use std::{io, time::Duration};
    loop {
        backend
            .prepare_separate_windows()
            .map_err(|error| io::Error::other(error.label()))?;
        let mut session = match backend.start(home) {
            Ok(session) => session,
            Err(cpf::azahar::BackendError::CartridgeSourceUnavailable) => {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
                };
                let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
                let answer = unsafe {
                    MessageBoxW(
                    std::ptr::null_mut(),
                    wide("The saved cartridge is missing or could not pass preflight. Eject it and reopen HOME?").as_ptr(),
                    wide("CPF 2DS").as_ptr(), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                )
                };
                if answer == IDYES {
                    backend
                        .set_startup_cartridge(None)
                        .map_err(|error| io::Error::other(error.label()))?;
                    continue;
                }
                return Ok(());
            }
            Err(error) => return Err(io::Error::other(error.label())),
        };
        let window = session
            .wait_for_window(Duration::from_secs(10))
            .map_err(|error| io::Error::other(error.label()))?
            .ok_or_else(|| io::Error::other("backend window unavailable"))?;
        let request = run_home_shell(&mut session, window, keys)?;
        // Release the runtime lock only after the previous backend has stopped.
        drop(session);
        let Some(request) = request else {
            return Ok(());
        };
        if let Err(message) = apply_media_request(backend, request) {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
            let title: Vec<u16> = "CPF 2DS".encode_utf16().chain(Some(0)).collect();
            let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
    }
}

#[cfg(windows)]
fn apply_media_request(
    backend: &AzaharBackend,
    request: cpf::media_request::MediaRequest,
) -> Result<(), &'static str> {
    use cpf::{azahar::BackendError, media_request::MediaRequest};
    let describe = |error| {
        match error {
        BackendError::RuntimeBusy => "Another CPF session is using this 2DS. Close it before changing game media.",
        BackendError::SdTitleAlreadyInstalled => "This title is already installed. Its existing files were kept.",
        BackendError::SdSourceChanged => "The CIA changed during installation. Check your source copy before retrying.",
        BackendError::SdStorageUnavailable => "The virtual SD is disabled or uses an unsupported storage location.",
        BackendError::SdInstallVerificationFailed => "The installed files did not pass verification. Check the private runtime log before retrying.",
        _ => "The media change could not finish. Check the private runtime log.",
    }
    };
    match request {
        MediaRequest::InsertCartridge(path) => {
            let cartridge = cpf::cartridge::CartridgeImage::open(&path)
                .map_err(|_| "The selected cartridge could not be verified. Choose a supported decrypted .cci or .3ds image.")?;
            backend
                .set_startup_cartridge(Some(&cartridge))
                .map_err(describe)
        }
        MediaRequest::EjectCartridge => backend.set_startup_cartridge(None).map_err(describe),
        MediaRequest::InstallSd(path) => cpf::operation_window::run(|status| {
            let package = cpf::sd_media::SdPackage::open(&path).map_err(|error| match error {
                cpf::sd_media::SdPackageError::NotBaseSdApplication => "Choose a base 3DS game CIA. This installer does not handle system titles, updates, or DLC.",
                _ => "The CIA did not pass verification. Choose a supported decrypted base-game CIA.",
            })?;
            status.set("Installing to virtual SD…");
            let mut installer = backend.start_sd_install(&package).map_err(describe)?;
            if !installer.wait().map_err(describe)?.success() {
                return Err(
                    "Installation did not complete. Check the private runtime log before retrying.",
                );
            }
            status.set("Verifying source and installed files…");
            backend.verify_sd_install(&package).map_err(describe)
        })
        .map_err(|_| "The installation status window could not finish. Check the private runtime log.")?,
    }
}

#[cfg(windows)]
fn run_home_shell(
    session: &mut cpf::azahar::AzaharSession,
    window: windows_sys::Win32::Foundation::HWND,
    button_keys: cpf::azahar::AzaharButtonKeys,
) -> std::io::Result<Option<cpf::media_request::MediaRequest>> {
    use std::io;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WM_CLOSE,
    };
    let preview = match std::env::current_exe() {
        Ok(path) => path.with_file_name("cpf-preview.exe"),
        Err(error) => {
            let _ = session.stop();
            return Err(error);
        }
    };
    // Park Azahar's primary UI as soon as it exists. The preview will attach
    // its two render windows separately; leaving the primary UI here avoids
    // showing it behind the device during shell startup.
    if unsafe {
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            -5000,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        )
    } == 0
    {
        let _ = session.stop();
        return Err(io::Error::last_os_error());
    }
    let result = (|| -> io::Result<_> {
        let mut child = Command::new(preview)
            .arg("--direct-pid")
            .arg(session.pid().to_string())
            .arg("--buttons")
            .arg(button_keys.encode())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()?;
        let job = cpf::process_lifecycle::ProcessJob::attach(&mut child)?;
        cpf::process_lifecycle::wait_with_peer(child, job, || {
            session
                .try_wait()
                .map(|status| status.is_none())
                .map_err(|error| io::Error::other(error.label()))
        })
    })();
    unsafe {
        PostMessageW(window, WM_CLOSE, 0, 0);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if matches!(session.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !matches!(session.try_wait(), Ok(Some(_))) {
        let _ = session.stop();
    }
    match result {
        Ok(output) if output.status.success() => {
            cpf::media_request::MediaRequest::decode(&output.stdout).map_err(io::Error::other)
        }
        Ok(_) => Err(io::Error::other("shell preview failed")),
        Err(error) => Err(error),
    }
}
