#![cfg_attr(
    all(windows, feature = "desktop", not(debug_assertions)),
    windows_subsystem = "windows"
)]
mod backend;
mod diagnostics;
mod execution;
mod inspector;
use std::{path::PathBuf, sync::atomic::AtomicBool};

#[cfg(feature = "desktop")]
mod desktop {
    use super::*;
    use std::sync::{atomic::Ordering, Arc, Mutex};
    use tauri::{Emitter, Manager};
    #[derive(Default)]
    pub struct Session {
        pub busy: AtomicBool,
        pub cancel: Arc<AtomicBool>,
        pub report: Mutex<Option<inspector::Report>>,
        pub plan: Mutex<Option<execution::Plan>>,
    }
    #[tauri::command]
    pub async fn choose_dump() -> Option<String> {
        rfd::AsyncFileDialog::new()
            .set_title("Select a PS4 dump directory (read only)")
            .pick_folder()
            .await
            .map(|v| v.path().to_string_lossy().into_owned())
    }
    #[tauri::command]
    pub async fn choose_backend() -> Option<String> {
        rfd::AsyncFileDialog::new()
            .set_title("Select verified shadPS4 backend (v0.20 default / v0.11 comparison)")
            .add_filter("Windows executable", &["exe"])
            .pick_file()
            .await
            .map(|v| v.path().to_string_lossy().into_owned())
    }
    #[tauri::command]
    pub async fn prepare_shellui(
        app: tauri::AppHandle,
        root: String,
        backend: String,
        parent: String,
    ) -> Result<execution::Plan, String> {
        if app.state::<Session>().busy.swap(true, Ordering::SeqCst) {
            return Err("Another operation is running".into());
        }
        let result = tauri::async_runtime::spawn_blocking(move || {
            execution::prepare(
                &PathBuf::from(root),
                &PathBuf::from(backend),
                &PathBuf::from(parent),
            )
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|v| v);
        let state = app.state::<Session>();
        state.busy.store(false, Ordering::SeqCst);
        if let Ok(ref plan) = result {
            *state.plan.lock().map_err(|e| e.to_string())? = Some(plan.clone());
        }
        result
    }
    #[tauri::command]
    pub async fn attempt_shellui(app: tauri::AppHandle) -> Result<execution::Attempt, String> {
        let plan = app
            .state::<Session>()
            .plan
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("Prepare a sandbox first")?;
        if app.state::<Session>().busy.swap(true, Ordering::SeqCst) {
            return Err("Another operation is running".into());
        }
        let result = tauri::async_runtime::spawn_blocking(move || execution::run(&plan))
            .await
            .map_err(|e| e.to_string())
            .and_then(|v| v);
        app.state::<Session>().busy.store(false, Ordering::SeqCst);
        result
    }
    #[tauri::command]
    pub async fn inspect_dump(
        app: tauri::AppHandle,
        root: String,
    ) -> Result<inspector::Report, String> {
        let state = app.state::<Session>();
        if state.busy.swap(true, Ordering::SeqCst) {
            return Err("An inspection is already running".into());
        }
        state.cancel.store(false, Ordering::SeqCst);
        let cancel = state.cancel.clone();
        let events = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let mut last = std::time::Instant::now();
            inspector::scan(&PathBuf::from(root), &cancel, |files, bytes| {
                if last.elapsed().as_millis() > 200 {
                    let _ = events.emit(
                        "scan-progress",
                        serde_json::json!({"files":files,"bytes":bytes}),
                    );
                    last = std::time::Instant::now();
                }
            })
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        let state = app.state::<Session>();
        state.busy.store(false, Ordering::SeqCst);
        if let Ok(ref r) = result {
            *state.report.lock().map_err(|e| e.to_string())? = Some(r.clone());
        }
        result
    }
    #[tauri::command]
    pub fn cancel_scan(state: tauri::State<Session>) {
        state.cancel.store(true, Ordering::SeqCst);
    }
    #[tauri::command]
    pub async fn export_report(app: tauri::AppHandle) -> Result<Option<String>, String> {
        let report = app
            .state::<Session>()
            .report
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("Inspect a dump first")?;
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Save private inspection report outside the dump; choose a new filename")
            .set_file_name("ps4-inspection.json")
            .save_file()
            .await
        else {
            return Ok(None);
        };
        let path = file.path().to_path_buf();
        inspector::export(&report, &path)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
    pub fn run() {
        tauri::Builder::default()
            .manage(Session::default())
            .invoke_handler(tauri::generate_handler![
                choose_dump,
                inspect_dump,
                cancel_scan,
                export_report,
                choose_backend,
                prepare_shellui,
                attempt_shellui
            ])
            .run(tauri::generate_context!())
            .expect("PS4 desktop runtime failed");
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--prepare-shellui") && args.len() == 5 {
        match execution::prepare(
            &PathBuf::from(&args[2]),
            &PathBuf::from(&args[3]),
            &PathBuf::from(&args[4]),
        ) {
            Ok(plan) => println!("{}", serde_json::to_string_pretty(&plan).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--attempt-shellui") && args.len() == 3 {
        let result = (|| -> Result<execution::Attempt, String> {
            let bytes = std::fs::read(&args[2]).map_err(|e| e.to_string())?;
            let plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            execution::run(&plan)
        })();
        match result {
            Ok(attempt) => println!("{}", serde_json::to_string_pretty(&attempt).unwrap()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("--inspect") {
        let result = (|| -> Result<(), String> {
            let root = args
                .get(2)
                .ok_or("Usage: PS4 --inspect DUMP [--output REPORT.json]")?;
            let r = inspector::scan(&PathBuf::from(root), &AtomicBool::new(false), |_, _| {})?;
            if let Some(flag) = args.get(3) {
                if flag != "--output" || args.len() != 5 {
                    return Err("Expected --output REPORT.json".into());
                }
                inspector::export(&r, &PathBuf::from(&args[4]))?;
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?
                );
            }
            if !r.complete {
                return Err("Inspection incomplete; review report diagnostics".into());
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(feature = "desktop")]
    desktop::run();
    #[cfg(not(feature = "desktop"))]
    {
        eprintln!("Usage: PS4 --inspect DUMP [--output REPORT.json]");
        std::process::exit(2);
    }
}
