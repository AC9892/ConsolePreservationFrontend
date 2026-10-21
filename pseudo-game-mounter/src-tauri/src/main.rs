#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod media;
mod melonds;
mod melonds_ui;
mod dev_dsi;
mod library;

use media::MediaInfo;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, State};

#[derive(Default, Deserialize, Serialize)]
struct Settings {
    emulator: Option<PathBuf>,
    selected_game: Option<PathBuf>,
    library_folder: Option<PathBuf>,
}

#[derive(Serialize)]
struct View {
    emulator: Option<String>,
    packaged_backend: bool,
    selected: Option<MediaInfo>,
    running: bool,
    settings_open: bool,
    mounted_title: Option<String>,
    mounted_path: Option<String>,
    slot_label: String,
    slot_inserted: Option<bool>,
    status: String,
    sessions: Vec<melonds_ui::EmulatorSession>,
    selected_pid: Option<u32>,
    session_kind: Option<&'static str>,
    session_label: Option<String>,
    show_melon_tools: bool,
}

struct Inner {
    settings: Settings,
    child: Option<Child>,
    settings_child: Option<Child>,
    mounted_title: Option<String>,
    mounted_game: Option<(u32, PathBuf)>,
    slot_observation: Option<melonds_ui::SlotObservation>,
    status: String,
    selected_pid: Option<u32>,
    show_melon_tools: bool,
}

struct AppState(Mutex<Inner>);

fn settings_path() -> Result<PathBuf, String> {
    Ok(melonds::data_root()?.join("settings.json"))
}

fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = settings_path()?;
    fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
    let content = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, content).map_err(|error| error.to_string())
}

fn process_running(inner: &mut Inner) -> Result<bool, String> {
    if let Some(child) = inner.child.as_mut() {
        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
            return Ok(true);
        }
        inner.child = None;
        inner.mounted_title = None;
        inner.mounted_game = None;
        inner.slot_observation = None;
        inner.status = "melonDS closed. The cartridge is no longer inserted.".into();
    }
    Ok(false)
}

fn settings_running(inner: &mut Inner) -> Result<bool, String> {
    if let Some(child) = inner.settings_child.as_mut() {
        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
            return Ok(true);
        }
        inner.settings_child = None;
    }
    Ok(false)
}

fn view(inner: &mut Inner) -> Result<View, String> {
    if inner.settings.emulator.is_none() {
        inner.settings.emulator = melonds::packaged_backend();
    }
    let managed_running = process_running(inner)?;
    let managed_pid = if managed_running { inner.child.as_ref().map(Child::id) } else { None };
    let mut sessions = melonds_ui::sessions();
    let dev_sessions = dev_dsi::sessions();
    sessions.extend(dev_sessions.iter().cloned());
    sessions.sort_by_key(|session| session.pid);
    if inner.selected_pid.is_none() && sessions.len() == 1 {
        inner.selected_pid = Some(sessions[0].pid);
        inner.slot_observation = None;
        inner.mounted_game = None;
        inner.mounted_title = None;
        if dev_sessions.iter().any(|session| session.pid == sessions[0].pid) {
            inner.status = format!("{} detected. Press Check slot or choose an .nds file to insert.",
                dev_sessions.iter().find(|session| session.pid == sessions[0].pid)
                    .map_or("DSi", |session| session.title.split(" (").next().unwrap_or("DSi")));
        }
    }
    if !inner.selected_pid.is_some_and(|pid| sessions.iter().any(|session| session.pid == pid) || managed_pid == Some(pid)) {
        inner.selected_pid = managed_pid;
        inner.slot_observation = None;
        inner.mounted_game = None;
        inner.mounted_title = None;
    }
    let selected_dev = inner.selected_pid.is_some_and(|pid| dev_sessions.iter().any(|session| session.pid == pid));
    if selected_dev {
        if let Some(pid) = inner.selected_pid {
            if let Ok(slot) = dev_dsi::check(pid) {
                if !slot.inserted || inner.mounted_title.as_ref().is_some_and(|title| title != &slot.label) {
                    inner.mounted_game = None;
                    inner.mounted_title = None;
                }
                inner.slot_observation = Some(slot);
            }
        }
    }
    let running = inner.selected_pid.is_some();
    let settings_open = settings_running(inner)?;
    let selected = inner.settings.selected_game.as_ref()
        .and_then(|path| media::inspect(path).ok());
    Ok(View {
        emulator: inner.settings.emulator.as_ref().map(|path| path.display().to_string()),
        packaged_backend: inner.settings.emulator.as_ref().is_some_and(|path| melonds::packaged_backend().as_ref() == Some(path)),
        selected,
        running,
        settings_open,
        mounted_title: inner.mounted_title.clone(),
        mounted_path: inner.mounted_game.as_ref()
            .filter(|(pid, _)| Some(*pid) == inner.selected_pid)
            .map(|(_, path)| path.display().to_string()),
        slot_label: if running {
            inner.slot_observation.as_ref().map_or_else(|| "Not checked".into(), |slot| {
                if slot.inserted { slot.label.clone() } else { "Empty".into() }
            })
        } else { "Empty".into() },
        slot_inserted: if running { inner.slot_observation.as_ref().map(|slot| slot.inserted) } else { Some(false) },
        status: inner.status.clone(),
        sessions,
        selected_pid: inner.selected_pid,
        session_kind: inner.selected_pid.map(|_| if selected_dev { "dev_dsi" } else { "melon" }),
        session_label: inner.selected_pid.and_then(|pid| {
            dev_sessions.iter().find(|session| session.pid == pid)
                .map(|session| session.title.split(" (").next().unwrap_or("DSi").to_owned())
        }),
        show_melon_tools: inner.show_melon_tools,
    })
}

#[tauri::command]
fn select_session(state: State<'_, AppState>, pid: u32) -> Result<View, String> {
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    if !melonds_ui::sessions().iter().any(|session| session.pid == pid) && !dev_dsi::is_session(pid) {
        return Err("That console session is no longer available".into());
    }
    inner.selected_pid = Some(pid);
    inner.slot_observation = None;
    inner.mounted_title = None;
    inner.mounted_game = None;
    inner.status = format!("Selected console session {pid}. Press Check slot to read its current cartridge.");
    view(&mut inner)
}

async fn background_view(
    app: tauri::AppHandle,
    action: impl FnOnce(&AppState) -> Result<View, String> + Send + 'static,
) -> Result<View, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        action(&state)
    }).await.map_err(|error| format!("Mounter operation stopped unexpectedly: {error}"))?
}

#[tauri::command]
async fn get_state(app: tauri::AppHandle) -> Result<View, String> {
    background_view(app, |state| {
        let mut inner = state.0.lock().map_err(|error| error.to_string())?;
        view(&mut inner)
    }).await
}

#[tauri::command]
async fn get_library(app: tauri::AppHandle) -> Result<library::LibraryListing, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let folder = state.0.lock().map_err(|error| error.to_string())?.settings.library_folder.clone();
        library::list(folder.as_deref())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
fn choose_library(state: State<'_, AppState>) -> Result<library::LibraryListing, String> {
    let picked = rfd::FileDialog::new().pick_folder();
    let Some(folder) = picked else {
        let current = state.0.lock().map_err(|error| error.to_string())?.settings.library_folder.clone();
        return library::list(current.as_deref());
    };
    let folder = library::create_or_open(&folder)?;
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    inner.settings.library_folder = Some(folder.clone());
    save_settings(&inner.settings)?;
    inner.status = format!("Game library ready at {}. Drop .nds files into this folder.", folder.display());
    library::list(Some(&folder))
}

#[tauri::command]
async fn use_dsi_sd_library(app: tauri::AppHandle) -> Result<library::LibraryListing, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let mut inner = state.0.lock().map_err(|error| error.to_string())?;
        view(&mut inner)?;
        let pid = inner.selected_pid.ok_or("Select a running DSi under Target app first")?;
        if !dev_dsi::is_session(pid) {
            return Err("Choose DSi or Development DSi to use its virtual SD library".into());
        }
        let sd_host = dev_dsi::virtual_sd_host(pid)?;
        let folder = library::create_on_virtual_sd(&sd_host)?;
        inner.settings.library_folder = Some(folder.clone());
        save_settings(&inner.settings)?;
        inner.status = "DSi virtual SD game library selected. New files sync to the SD image on the next DSi start.".into();
        library::list(Some(&folder))
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
fn open_library(state: State<'_, AppState>) -> Result<(), String> {
    let folder = state.0.lock().map_err(|error| error.to_string())?.settings.library_folder.clone()
        .ok_or("Choose a game library folder first")?;
    library::list(Some(&folder))?;
    #[cfg(windows)]
    Command::new("explorer.exe").arg(folder).spawn().map_err(|error| error.to_string())?;
    #[cfg(not(windows))]
    return Err("Open folder is currently available on Windows only".into());
    Ok(())
}

#[tauri::command]
async fn select_library_game(app: tauri::AppHandle, file_name: String) -> Result<View, String> {
    background_view(app, move |state| {
        let mut inner = state.0.lock().map_err(|error| error.to_string())?;
        let folder = inner.settings.library_folder.as_ref().ok_or("Choose a game library folder first")?;
        let (path, info) = library::game(folder, &file_name)?;
        view(&mut inner)?;
        if let Some(pid) = inner.selected_pid {
            if inner.mounted_game.as_ref().is_some_and(|(mounted_pid, mounted_path)| {
                *mounted_pid == pid && mounted_path.canonicalize().ok() == path.canonicalize().ok()
            }) && inner.slot_observation.as_ref().is_some_and(|slot| slot.inserted) {
                return Err("This game is already mounted in Slot-1".into());
            }
            if !inner.slot_observation.as_ref().is_some_and(|slot| !slot.inserted) {
                return Err("Eject the current cartridge or press Check slot before inserting another game".into());
            }
            let slot = if dev_dsi::is_session(pid) { dev_dsi::insert(pid, &path)? }
                else { melonds_ui::insert(pid, &path)? };
            inner.slot_observation = Some(slot);
            inner.mounted_title = Some(info.title.clone());
            inner.mounted_game = Some((pid, path.clone()));
            inner.status = format!("Inserted {} from the game library into Slot-1.", info.title);
        } else {
            inner.status = format!("Selected {} from the game library. Start a console session to mount it.", info.title);
        }
        inner.settings.selected_game = Some(path);
        save_settings(&inner.settings)?;
        view(&mut inner)
    }).await
}

#[tauri::command]
fn choose_game(state: State<'_, AppState>) -> Result<View, String> {
    let picked = rfd::FileDialog::new()
        .add_filter("Nintendo DS cartridge", &["nds"])
        .pick_file();
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    if let Some(path) = picked {
        let info = media::inspect(&path)?;
        inner.settings.selected_game = Some(path);
        inner.status = format!("Selected {}. Cartridge is not mounted yet.", info.title);
        save_settings(&inner.settings)?;
    }
    view(&mut inner)
}

#[tauri::command]
fn choose_emulator(state: State<'_, AppState>) -> Result<View, String> {
    let picked = rfd::FileDialog::new()
        .add_filter("melonDS executable", &["exe"])
        .pick_file();
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    if let Some(path) = picked {
        if !path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("melonDS.exe")) {
            return Err("Select melonDS.exe".into());
        }
        inner.settings.emulator = Some(path);
        inner.status = "melonDS selected. It must be configured for DSi mode with Direct Boot off.".into();
        save_settings(&inner.settings)?;
    }
    view(&mut inner)
}

#[tauri::command]
fn open_emulator_settings(state: State<'_, AppState>) -> Result<View, String> {
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    if process_running(&mut inner)? {
        return Err("Close the managed melonDS session before opening its settings.".into());
    }
    if settings_running(&mut inner)? {
        inner.status = "melonDS settings are already open. Finish configuring that window, then close it.".into();
        return view(&mut inner);
    }
    let executable = inner.settings.emulator.as_ref().ok_or("Select melonDS.exe first")?;
    melonds::ensure_packaged_config_dir(executable)?;
    let child = Command::new(executable).current_dir(executable.parent().unwrap()).spawn()
        .map_err(|error| format!("Could not open melonDS: {error}"))?;
    inner.settings_child = Some(child);
    inner.status = "Open melonDS settings: select DSi mode, set your BIOS/firmware/NAND paths, and turn off Direct Boot. Then close it before mounting.".into();
    view(&mut inner)
}

#[tauri::command]
async fn mount(app: tauri::AppHandle) -> Result<View, String> {
    background_view(app, |state| mount_inner(state)).await
}

fn mount_inner(state: &AppState) -> Result<View, String> {
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    if process_running(&mut inner)? {
        return Err("A managed melonDS session is still running. Close it before replacing the cartridge.".into());
    }
    if settings_running(&mut inner)? {
        return Err("Close the melonDS settings window before mounting a cartridge.".into());
    }
    let cartridge = inner.settings.selected_game.clone().ok_or("Choose an .nds file first")?;
    let info = media::inspect(&cartridge)?;
    let executable = inner.settings.emulator.as_ref().ok_or("Choose melonDS.exe first")?;
    let prepared = melonds::prepare(executable)?;
    let mut child = melonds::launch(&prepared, &cartridge)?;
    std::thread::sleep(Duration::from_millis(600));
    if let Some(exit) = child.try_wait().map_err(|error| error.to_string())? {
        return Err(format!("melonDS exited immediately (code {}). Check its configuration.", exit.code().unwrap_or(-1)));
    }
    inner.mounted_title = Some(info.title);
    inner.mounted_game = Some((child.id(), cartridge.clone()));
    inner.selected_pid = Some(child.id());
    inner.child = Some(child);
    inner.slot_observation = None;
    inner.status = "melonDS started with the selected cartridge. Check the DSi Menu, or press Check slot for an explicit emulator status read.".into();
    view(&mut inner)
}

#[tauri::command]
async fn insert_cartridge(app: tauri::AppHandle) -> Result<View, String> {
    let picked = rfd::FileDialog::new()
        .add_filter("Nintendo DS cartridge", &["nds"])
        .pick_file();
    let Some(path) = picked else {
        return background_view(app, |state| {
            let mut inner = state.0.lock().map_err(|error| error.to_string())?;
            inner.status = "Replacement canceled. The current emulator session is unchanged.".into();
            view(&mut inner)
        }).await;
    };
    let info = media::inspect(&path)?;
    background_view(app, move |state| {
        let mut inner = state.0.lock().map_err(|error| error.to_string())?;
        view(&mut inner)?;
        let pid = inner.selected_pid.ok_or("No melonDS session selected. Choose a running session or mount a new one.")?;
        if !inner.slot_observation.as_ref().is_some_and(|slot| !slot.inserted) {
            return Err("Eject the cartridge first, or press Check slot if you ejected it in melonDS.".into());
        }
        let slot = if dev_dsi::is_session(pid) { dev_dsi::insert(pid, &path)? }
            else { melonds_ui::insert(pid, &path)? };
        inner.slot_observation = Some(slot);
        inner.settings.selected_game = Some(path.clone());
        save_settings(&inner.settings)?;
        inner.mounted_title = Some(info.title.clone());
        inner.mounted_game = Some((pid, path.clone()));
        inner.status = format!("Inserted {} into the live DSi Slot-1. Check the original DSi Menu.", info.title);
        view(&mut inner)
    }).await
}

#[tauri::command]
async fn check_slot(app: tauri::AppHandle) -> Result<View, String> {
    background_view(app, |state| check_slot_inner(state)).await
}

fn check_slot_inner(state: &AppState) -> Result<View, String> {
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    let session_name = view(&mut inner)?.session_label.unwrap_or_else(|| "melonDS".into());
    let Some(pid) = inner.selected_pid else {
        inner.status = "No melonDS session selected.".into();
        return view(&mut inner);
    };
    let was_inserted = inner.slot_observation.as_ref().is_some_and(|slot| slot.inserted);
    let selected_dev = dev_dsi::is_session(pid);
    let slot = if selected_dev { dev_dsi::check(pid)? } else { melonds_ui::check(pid)? };
    if !slot.inserted || (selected_dev && inner.mounted_title.as_ref().is_some_and(|title| title != &slot.label)) {
        inner.mounted_title = None;
        inner.mounted_game = None;
    }
    inner.status = if slot.inserted {
        format!("{} reports a cartridge in Slot-1: {}.", session_name, slot.label)
    } else if was_inserted {
        inner.settings.selected_game = None;
        save_settings(&inner.settings)?;
        "Cartridge was ejected in melonDS. Slot-1 is empty.".into()
    } else {
        format!("{} confirms Slot-1 is empty.", session_name)
    };
    inner.slot_observation = Some(slot);
    view(&mut inner)
}

#[tauri::command]
async fn eject(app: tauri::AppHandle) -> Result<View, String> {
    background_view(app, |state| eject_inner(state)).await
}

fn eject_inner(state: &AppState) -> Result<View, String> {
    let mut inner = state.0.lock().map_err(|error| error.to_string())?;
    let session_name = view(&mut inner)?.session_label.unwrap_or_else(|| "melonDS".into());
    let pid = inner.selected_pid.ok_or("No melonDS session selected. Use Clear selection to remove the selected file.")?;
    let selected_dev = dev_dsi::is_session(pid);
    let slot = if selected_dev { dev_dsi::eject(pid)? } else { melonds_ui::eject(pid)? };
    inner.mounted_title = None;
    inner.mounted_game = None;
    inner.slot_observation = Some(slot);
    inner.settings.selected_game = None;
    inner.status = format!("{} confirms the cartridge was ejected. Slot-1 is empty.", session_name);
    save_settings(&inner.settings)?;
    view(&mut inner)
}

#[tauri::command]
async fn clear_selection(app: tauri::AppHandle) -> Result<View, String> {
    background_view(app, |state| {
        let mut inner = state.0.lock().map_err(|error| error.to_string())?;
        if view(&mut inner)?.running {
            return Err("Eject the cartridge or close the emulator before clearing this selection.".into());
        }
        inner.mounted_title = None;
        inner.mounted_game = None;
        inner.settings.selected_game = None;
        inner.status = "Selection cleared. Slot-1 is empty.".into();
        save_settings(&inner.settings)?;
        view(&mut inner)
    }).await
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let show_melon_tools = args.iter().skip(1).any(|arg| arg == "--melon");
    #[cfg(debug_assertions)]
    if args.len() == 2 && args[1] == "--sessions" {
        let mut sessions = melonds_ui::sessions();
        sessions.extend(dev_dsi::sessions());
        println!("{}", serde_json::to_string_pretty(&sessions).unwrap());
        return;
    }
    #[cfg(debug_assertions)]
    if args.len() >= 3 && args[1].starts_with("--dev-") {
        let pid: u32 = args[2].parse().expect("Expected a Development DSi process ID");
        let result = match args[1].as_str() {
            "--dev-status" if args.len() == 3 => dev_dsi::check(pid),
            "--dev-eject" if args.len() == 3 => dev_dsi::eject(pid),
            "--dev-insert" if args.len() == 4 => dev_dsi::insert(pid, &PathBuf::from(&args[3])),
            _ => Err("Unknown Development DSi diagnostic command".into()),
        };
        match result {
            Ok(slot) => println!("{}", serde_json::to_string_pretty(&slot).unwrap()),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    if args.len() == 3 && args[1] == "--inspect" {
        match media::inspect(&PathBuf::from(&args[2])) {
            Ok(mut info) => {
                info.icon_rgba = None;
                println!("{}", serde_json::to_string_pretty(&info).unwrap());
            }
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    if args.len() == 3 && args[1] == "--prepare-emulator" {
        match melonds::prepare(&PathBuf::from(&args[2])) {
            Ok(prepared) => println!("{}", serde_json::to_string_pretty(&prepared).unwrap()),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    #[cfg(debug_assertions)]
    if args.len() == 3 && (args[1] == "--slot-status" || args[1] == "--eject-slot") {
        let pid: u32 = args[2].parse().expect("Expected a process ID");
        let result = if args[1] == "--eject-slot" { melonds_ui::eject(pid) } else { melonds_ui::check(pid) };
        match result {
            Ok(slot) => println!("{}", serde_json::to_string_pretty(&slot).unwrap()),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
        }
        return;
    }
    let mut settings: Settings = settings_path().ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if settings.emulator.is_none() {
        settings.emulator = melonds::packaged_backend();
    }
    if settings.library_folder.is_none() {
        if let Some(folder) = std::env::current_exe().ok()
            .and_then(|executable| executable.parent().map(|parent| parent.join(".nds")))
            .filter(|folder| folder.is_dir())
        {
            if let Ok(folder) = library::create_or_open(&folder) {
                settings.library_folder = Some(folder);
                let _ = save_settings(&settings);
            }
        }
    }
    let initial_status = if settings.emulator.as_ref().is_some_and(|path| melonds::packaged_backend().as_ref() == Some(path)) {
        "Packaged melonDS found. Configure DSi mode if needed, then choose a cartridge."
    } else if settings.emulator.is_some() {
        "Choose a cartridge and mount it in the configured melonDS session."
    } else if show_melon_tools {
        "Select a configured melonDS executable and choose a cartridge."
    } else {
        "No melonDS backend found. Launch with --melon to choose one."
    };
    tauri::Builder::default()
        .manage(AppState(Mutex::new(Inner {
            settings,
            child: None,
            settings_child: None,
            mounted_title: None,
            mounted_game: None,
            slot_observation: None,
            status: initial_status.into(),
            selected_pid: None,
            show_melon_tools,
        })))
        .invoke_handler(tauri::generate_handler![
            get_state, get_library, choose_library, use_dsi_sd_library, open_library, select_library_game,
            choose_game, choose_emulator, open_emulator_settings, mount, insert_cartridge, check_slot, eject, clear_selection, select_session
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Pseudo Game Mounter");
}
