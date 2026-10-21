use cpf_core::{
    CleanNandResult, CloneResult, MachineDetail, MachineSettings, MachineSummary, Readiness, RuntimeRegistry, SaveImportResult, SdRestoreResult,
    SdImportResult, SdSnapshot, SdSnapshotSummary, StorageComparison, VerificationReport, Workspace,
};
use serde_json::Value;
use std::process::Command;
use tauri::Manager;

mod screen;
use screen::ScreenBridge;
mod dev_bridge;
mod public_setup;
mod public_shell;
mod public_input;
mod button_layout;

struct AppState(Workspace);

struct DesktopMode {
    dsi: bool,
    public: bool,
    machine_id: Option<String>,
}

#[tauri::command]
fn desktop_mode(mode: tauri::State<'_, DesktopMode>) -> Value {
    serde_json::json!({"dsi": mode.dsi, "public": mode.public, "machine_id": mode.machine_id})
}

#[tauri::command]
fn public_setup_status(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>) -> Result<public_setup::SetupStatus, String> {
    if !mode.public { return Err("Public setup is unavailable in this build".into()); }
    Ok(public_setup::status(&state.0))
}

#[tauri::command]
fn public_scan_folder(folder: String) -> Result<serde_json::Value, String> {
    public_setup::scan_folder(std::path::Path::new(&folder))
}

#[tauri::command]
async fn public_import(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>, files: public_setup::ImportFiles) -> Result<public_setup::SetupStatus, String> {
    if !mode.public { return Err("Public setup is unavailable in this build".into()); }
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || public_setup::import(&workspace, files)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn public_pick_file(folder: bool) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let picker = rfd::FileDialog::new();
        let chosen = if folder { picker.pick_folder() } else { picker.pick_file() };
        chosen.map(|p| p.to_string_lossy().into_owned())
    }).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn public_shell_image(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>) -> Result<Option<String>, String> {
    if !mode.public { return Err("Custom shell is unavailable in this build".into()); }
    let root = state.0.root.clone();
    tauri::async_runtime::spawn_blocking(move || public_shell::load(&root)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn public_set_shell(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>, source_path: Option<String>) -> Result<(), String> {
    if !mode.public { return Err("Custom shell is unavailable in this build".into()); }
    let root = state.0.root.clone();
    tauri::async_runtime::spawn_blocking(move || public_shell::set(&root, source_path.as_deref())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn public_input_config(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>) -> Result<public_input::InputConfig, String> {
    if !mode.public { return Err("Keyboard config is unavailable in this build".into()); }
    public_input::load(&state.0.root)
}

#[tauri::command]
fn get_button_layout(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>) -> Result<Option<button_layout::ButtonLayout>, String> {
    if !mode.dsi { return Err("Button layout is only available in the DSi app".into()); }
    button_layout::load(&state.0.root, mode.public)
}

#[tauri::command]
fn save_button_layout(state: tauri::State<'_, AppState>, mode: tauri::State<'_, DesktopMode>, layout: button_layout::ButtonLayout) -> Result<(), String> {
    if !mode.dsi { return Err("Button layout is only available in the DSi app".into()); }
    button_layout::save(&state.0.root, mode.public, layout)
}

#[tauri::command]
fn open_dsi(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<cpf_core::LaunchResult, String> {
    state.0.machine_dir(&machine_id)?;
    let executable = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("App has no executable directory")?
        .join("Development_DSI.exe");
    if !executable.is_file() {
        return Err("Development_DSI.exe is unavailable; rebuild the desktop apps".into());
    }
    let child = Command::new(&executable)
        .arg("--machine-id")
        .arg(&machine_id)
        .current_dir(&state.0.root)
        .spawn()
        .map_err(|e| format!("Could not open DSi: {e}"))?;
    Ok(cpf_core::LaunchResult {
        pid: child.id(),
        backend: "DSi desktop app".into(),
        note: "The DSi runs in its own window and process".into(),
    })
}

#[tauri::command]
fn list_machines(state: tauri::State<'_, AppState>) -> Result<Vec<MachineSummary>, String> {
    state.0.machines()
}

#[tauri::command]
fn machine_detail(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<MachineDetail, String> {
    state.0.detail(&machine_id)
}

#[tauri::command]
fn machine_settings(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<MachineSettings, String> {
    state.0.settings(&machine_id)
}

#[tauri::command]
fn save_machine_settings(
    state: tauri::State<'_, AppState>,
    machine_id: String,
    settings: MachineSettings,
) -> Result<MachineSettings, String> {
    state.0.save_settings(&machine_id, settings)
}

#[tauri::command]
fn runtime_readiness(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<Readiness, String> {
    state.0.readiness(&machine_id)
}

#[tauri::command]
fn runtime_status(state: tauri::State<'_, AppState>, machine_id: String) -> Result<Value, String> {
    state.0.runtime_status(&machine_id)
}

#[tauri::command]
async fn verify_preservation(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<VerificationReport, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.verify_preservation(&machine_id))
        .await
        .map_err(|e| format!("Verification task failed: {e}"))?
}

#[tauri::command]
async fn restore_clean_nand(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<CleanNandResult, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.restore_clean_nand(&machine_id))
        .await
        .map_err(|e| format!("NAND restore task failed: {e}"))?
}

#[tauri::command]
async fn clone_machine(
    state: tauri::State<'_, AppState>,
    machine_id: String,
    new_name: String,
) -> Result<CloneResult, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.clone_machine(&machine_id, &new_name))
        .await
        .map_err(|e| format!("Machine clone task failed: {e}"))?
}

#[tauri::command]
async fn import_save(
    state: tauri::State<'_, AppState>,
    machine_id: String,
    category: String,
    source_path: String,
    target_name: Option<String>,
) -> Result<SaveImportResult, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move ||
        workspace.import_save(&machine_id, &category, std::path::Path::new(&source_path), target_name.as_deref()))
        .await
        .map_err(|e| format!("Save import task failed: {e}"))?
}

#[tauri::command]
async fn compare_storage(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<StorageComparison, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.compare_storage(&machine_id))
        .await
        .map_err(|e| format!("SD comparison task failed: {e}"))?
}

#[tauri::command]
fn cartridge_slot(state: tauri::State<'_, AppState>, machine_id: String) -> Result<Value, String> {
    state.0.cartridge_slot(&machine_id)
}

#[tauri::command]
async fn insert_cartridge(state: tauri::State<'_, AppState>, machine_id: String, source_path: String) -> Result<Value, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.insert_cartridge(&machine_id, std::path::Path::new(&source_path)))
        .await.map_err(|e| format!("Cartridge import task failed: {e}"))?
}

#[tauri::command]
async fn eject_cartridge(state: tauri::State<'_, AppState>, machine_id: String) -> Result<Value, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.eject_cartridge(&machine_id))
        .await.map_err(|e| format!("Cartridge eject task failed: {e}"))?
}

#[tauri::command]
async fn import_runtime_sd_backup(state: tauri::State<'_, AppState>, machine_id: String, source_path: String) -> Result<SdImportResult, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.import_runtime_sd_backup(&machine_id, std::path::Path::new(&source_path)))
        .await.map_err(|e| format!("SD import task failed: {e}"))?
}

#[tauri::command]
async fn snapshot_sd_image(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<SdSnapshot, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || workspace.snapshot_sd_image(&machine_id))
        .await
        .map_err(|e| format!("SD snapshot task failed: {e}"))?
}

#[tauri::command]
fn list_sd_snapshots(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<Vec<SdSnapshotSummary>, String> {
    state.0.sd_snapshots(&machine_id)
}

#[tauri::command]
async fn restore_sd_image(
    state: tauri::State<'_, AppState>,
    machine_id: String,
    snapshot_id: String,
) -> Result<SdRestoreResult, String> {
    let workspace = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        workspace.restore_sd_image(&machine_id, &snapshot_id)
    })
    .await
    .map_err(|e| format!("SD restore task failed: {e}"))?
}

#[tauri::command]
fn launch_runtime(
    state: tauri::State<'_, AppState>,
    machine_id: String,
) -> Result<cpf_core::LaunchResult, String> {
    RuntimeRegistry::default().launch(&state.0, &machine_id)
}

#[tauri::command]
async fn start_console(
    state: tauri::State<'_, AppState>,
    mode: tauri::State<'_, DesktopMode>,
    app: tauri::AppHandle,
    machine_id: String,
) -> Result<cpf_core::LaunchResult, String> {
    let workspace = state.0.clone();
    let public_mode = mode.public;
    tauri::async_runtime::spawn_blocking(move || {
        let screen = app.state::<ScreenBridge>();
        screen.start(&workspace, &machine_id, public_mode, app.clone())
    })
    .await
    .map_err(|e| format!("Console launch task failed: {e}"))?
}

#[tauri::command]
fn console_control(screen: tauri::State<'_, ScreenBridge>, command: Value) -> Result<(), String> {
    screen.control(command)
}

#[tauri::command]
fn stop_console(screen: tauri::State<'_, ScreenBridge>) -> Result<(), String> {
    screen.stop_and_wait()
}

#[tauri::command]
fn open_legacy_tools(state: tauri::State<'_, AppState>) -> Result<cpf_core::LaunchResult, String> {
    state.0.open_legacy_tools()
}

pub fn run() {
    run_mode(false, false);
}

pub fn run_dsi() {
    run_mode(true, false);
}

pub fn run_public_dsi() {
    run_mode(true, true);
}

fn run_mode(dsi: bool, public: bool) {
    let workspace = if public { public_setup::workspace() } else { Workspace::discover() }
        .expect("CPF workspace unavailable");
    let args: Vec<String> = std::env::args().collect();
    let machine_id = args
        .windows(2)
        .find(|args| args[0] == "--machine-id")
        .map(|args| args[1].clone());
    tauri::Builder::default()
        .manage(AppState(workspace))
        .manage(DesktopMode { dsi, public, machine_id })
        .manage(ScreenBridge::default())
        .manage(dev_bridge::Registration::default())
        .setup(move |app| {
            if dsi {
                if let Err(error) = dev_bridge::start(app.handle().clone(), if public { "DSi" } else { "Development DSi" }) {
                    eprintln!("DSi cartridge bridge unavailable: {error}");
                }
                if let Some(window) = app.get_webview_window("main") {
                    window.set_title("DSi")?;
                    #[cfg(windows)]
                    window.set_icon(tauri::image::Image::from_app_icon_resource(256)?)?;
                    let configured = !public || public_setup::status(&app.state::<AppState>().0).configured;
                    window.set_decorations(!configured)?;
                    window.set_shadow(false)?;
                    window.set_resizable(false)?;
                    window.set_size(tauri::Size::Logical(if configured {
                        tauri::LogicalSize::new(628.0, 628.0)
                    } else {
                        tauri::LogicalSize::new(760.0, 650.0)
                    }))?;
                    window.center()?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let app = window.app_handle().clone();
                    app.state::<dev_bridge::Registration>().remove();
                    std::thread::spawn(move || {
                        let _ = app.state::<ScreenBridge>().stop_and_wait();
                        app.exit(0);
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_machines,
            machine_detail,
            machine_settings,
            save_machine_settings,
            runtime_readiness,
            runtime_status,
            verify_preservation,
            restore_clean_nand,
            clone_machine,
            import_save,
            cartridge_slot,
            insert_cartridge,
            eject_cartridge,
            import_runtime_sd_backup,
            compare_storage,
            snapshot_sd_image,
            list_sd_snapshots,
            restore_sd_image,
            launch_runtime,
            start_console,
            console_control,
            stop_console,
            open_legacy_tools,
            desktop_mode,
            open_dsi,
            public_setup_status,
            public_import,
            public_pick_file,
            public_scan_folder,
            public_shell_image,
            public_set_shell,
            public_input_config,
            get_button_layout,
            save_button_layout
        ])
        .run(tauri::generate_context!())
        .expect("CPF desktop failed to start");
}
