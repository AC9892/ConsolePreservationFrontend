use cpf_core::{LaunchResult, Workspace};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

struct ScreenProcess {
    child: Child,
    stdin: ChildStdin,
    machine_id: String,
}

#[derive(Default)]
pub struct ScreenBridge(Mutex<Option<ScreenProcess>>, Arc<AtomicBool>, Arc<Mutex<Option<Value>>>);

impl ScreenBridge {
    pub fn stop_and_wait(&self) -> Result<(), String> {
        let mut active = self.0.lock().map_err(|e| e.to_string())?;
        let Some(process) = active.as_mut() else {
            return Ok(());
        };
        if process
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none()
        {
            process
                .stdin
                .write_all(b"{\"kind\":\"stop\"}\n")
                .map_err(|e| e.to_string())?;
            process.stdin.flush().map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while process
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_none()
            {
                if Instant::now() >= deadline {
                    return Err("The DSi worker is still closing".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        *active = None;
        self.1.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub fn start(
        &self,
        workspace: &Workspace,
        machine_id: &str,
        public_mode: bool,
        app: AppHandle,
    ) -> Result<LaunchResult, String> {
        let machine = workspace.machine_dir(machine_id)?;
        let ready = workspace.readiness(machine_id)?;
        if !ready.ready_to_attempt_boot {
            return Err(format!(
                "Runtime files missing: {}",
                ready.missing.join(", ")
            ));
        }
        let mut active = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = active.as_mut() {
            if existing
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err("A console session is already running".into());
            }
        }
        let worker = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("Console executable has no directory")?
            .join("DSi Backend.exe");
        if !worker.is_file() {
            return Err("DSi backend is unavailable; rebuild the desktop apps".into());
        }
        let log = File::create(machine.join("logs/screen-worker-error.log"))
            .map_err(|e| format!("Could not create runtime error log: {e}"))?;
        let mut command = Command::new(&worker);
        command
            .arg("--workspace")
            .arg(&workspace.root)
            .arg("--machine-id")
            .arg(machine_id)
            .current_dir(&workspace.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(log));
        if public_mode {
            command.arg("--core-file").arg("melondsds_system_compat.dll");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start console worker: {e}"))?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or("Console worker has no control channel")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Console worker has no frame channel")?;
        self.1.store(false, Ordering::Relaxed);
        if let Ok(mut event) = self.2.lock() { *event = None; }
        *active = Some(ScreenProcess { child, stdin, machine_id: machine_id.to_owned() });
        let live_supported = self.1.clone();
        let cartridge_event = self.2.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("kind").and_then(Value::as_str) == Some("ready") {
                    live_supported.store(value.get("live_cartridge").and_then(Value::as_bool).unwrap_or(false), Ordering::Relaxed);
                }
                if matches!(value.get("kind").and_then(Value::as_str), Some("cartridge" | "cartridge_error")) {
                    if let Ok(mut event) = cartridge_event.lock() { *event = Some(value.clone()); }
                }
                let _ = app.emit("cpf-screen", value);
            }
            live_supported.store(false, Ordering::Relaxed);
            let _ = app.emit("cpf-screen", json!({"kind": "stopped"}));
        });
        Ok(LaunchResult { pid, backend: "melonDS DS libretro; Rust worker; Tauri console presentation".into(),
            note: "The Tauri window owns screens and controls. Rust manages generated NAND, firmware, SD, audit, and the melonDS libretro backend.".into() })
    }

    pub fn control(&self, value: Value) -> Result<(), String> {
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("Control kind missing")?;
        match kind {
            "stop" | "pause" | "reset" | "cartridge_eject" => {}
            "cartridge_insert" => {
                let path = value.get("path").and_then(Value::as_str).ok_or("Cartridge path missing")?;
                let path = std::path::Path::new(path);
                if !path.is_absolute() || !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("nds")) {
                    return Err("Select an absolute .nds file".into());
                }
                let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("Cartridge must be a regular file".into());
                }
            }
            "input" => {
                let button = value
                    .get("button")
                    .and_then(Value::as_str)
                    .ok_or("Input button missing")?;
                if ![
                    "UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "L", "R", "START", "SELECT",
                ]
                .contains(&button)
                    || !value.get("pressed").is_some_and(Value::is_boolean)
                {
                    return Err("Invalid console input".into());
                }
            }
            "touch" => {
                let x = value
                    .get("x")
                    .and_then(Value::as_i64)
                    .ok_or("Touch X missing")?;
                let y = value
                    .get("y")
                    .and_then(Value::as_i64)
                    .ok_or("Touch Y missing")?;
                if !(0..=255).contains(&x)
                    || !(0..=191).contains(&y)
                    || !value.get("pressed").is_some_and(Value::is_boolean)
                {
                    return Err("Invalid console touch".into());
                }
            }
            _ => return Err("Unknown console control".into()),
        }
        let mut active = self.0.lock().map_err(|e| e.to_string())?;
        if matches!(kind, "cartridge_insert" | "cartridge_eject") {
            *self.2.lock().map_err(|e| e.to_string())? = None;
        }
        let process = active.as_mut().ok_or("Console is not running")?;
        if process
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("Console process has stopped".into());
        }
        serde_json::to_writer(&mut process.stdin, &value).map_err(|e| e.to_string())?;
        process.stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        process.stdin.flush().map_err(|e| e.to_string())
    }

    pub fn session(&self) -> Result<Value, String> {
        let mut active = self.0.lock().map_err(|e| e.to_string())?;
        let Some(process) = active.as_mut() else {
            return Ok(json!({"running": false}));
        };
        if process.child.try_wait().map_err(|e| e.to_string())?.is_some() {
            *active = None;
            self.1.store(false, Ordering::Relaxed);
            return Ok(json!({"running": false}));
        }
        Ok(json!({"running": true, "machine_id": process.machine_id, "pid": process.child.id()}))
    }

    pub fn live_cartridge_supported(&self) -> bool {
        self.1.load(Ordering::Relaxed)
    }

    pub fn take_cartridge_result(&self) -> Option<Value> {
        self.2.lock().ok()?.take()
    }
}
