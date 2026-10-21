use crate::{AppState, screen::ScreenBridge};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub struct Registration(Mutex<Option<PathBuf>>, Mutex<()>);

impl Registration {
    pub fn remove(&self) {
        if let Ok(mut path) = self.0.lock() {
            if let Some(path) = path.take() { let _ = fs::remove_file(path); }
        }
    }
}

pub fn start(app: AppHandle, title: &str) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
        .join("CPF").join("DSiSessions");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let pid = std::process::id();
    let path = root.join(format!("{pid}.json"));
    let temporary = root.join(format!("{pid}.json.tmp"));
    let descriptor = json!({"version":1,"pid":pid,"port":port,"token":token,"title":title});
    fs::write(&temporary, serde_json::to_vec(&descriptor).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if path.exists() { fs::remove_file(&path).map_err(|e| e.to_string())?; }
    fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    *app.state::<Registration>().0.lock().map_err(|e| e.to_string())? = Some(path);
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(stream) = connection else { continue };
            let app = app.clone();
            let token = token.clone();
            std::thread::spawn(move || serve(stream, &app, &token));
        }
    });
    Ok(())
}

fn serve(mut stream: TcpStream, app: &AppHandle, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let result = (|| -> Result<Value, String> {
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
        if line.len() > 16_384 { return Err("Bridge request is too large".into()); }
        let request: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if request["token"].as_str() != Some(token) { return Err("Bridge authentication failed".into()); }
        match request["command"].as_str() {
            Some("status") => status(app),
            Some("insert") => change(app, "cartridge_insert", request["path"].as_str()),
            Some("eject") => change(app, "cartridge_eject", None),
            _ => Err("Unknown bridge command".into()),
        }
    })();
    let response = result.unwrap_or_else(|error| json!({"ok":false,"error":error}));
    let _ = writeln!(stream, "{response}");
}

fn status(app: &AppHandle) -> Result<Value, String> {
    let screen = app.state::<ScreenBridge>();
    let session = screen.session()?;
    let running = session["running"].as_bool() == Some(true);
    let (slot, sd_host_path) = if running {
        let id = session["machine_id"].as_str().ok_or("Running DSi has no machine ID")?;
        let workspace = &app.state::<AppState>().0;
        let slot = workspace.cartridge_slot(id)?;
        let machine = workspace.machine_dir(id)?;
        let config = cpf_core::read_json(&machine.join("runtime/config.json")).unwrap_or(json!({}));
        let sd_host = machine.join("runtime/libretro/save/melonDS DS/dsi_sd_card");
        let sd_host_path = if config["virtual_sd_enabled"] == true && sd_host.is_dir() {
            Some(sd_host)
        } else { None };
        (slot, sd_host_path)
    } else { (json!({"inserted":false}), None) };
    Ok(json!({"ok":true,"running":running,"machine_id":session["machine_id"],
        "worker_pid":session["pid"],"live_supported":screen.live_cartridge_supported(),
        "inserted":slot["inserted"],"title":slot.pointer("/metadata/banner_title_english")
            .or_else(||slot.pointer("/metadata/title_from_header")),
        "source_path":slot["source_path"],"sd_host_path":sd_host_path}))
}

fn change(app: &AppHandle, command: &str, path: Option<&str>) -> Result<Value, String> {
    let registration = app.state::<Registration>();
    let _operation = registration.1.lock().map_err(|e| e.to_string())?;
    let screen = app.state::<ScreenBridge>();
    if !screen.live_cartridge_supported() { return Err("The DSi runtime is not ready for live cartridges".into()); }
    let request = if command == "cartridge_insert" {
        json!({"kind":command,"path":path.ok_or("Select an .nds file")?})
    } else { json!({"kind":command}) };
    screen.control(request)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(event) = screen.take_cartridge_result() {
            if event["kind"] == "cartridge_error" {
                return Err(event["message"].as_str().unwrap_or("Cartridge operation failed").into());
            }
            return status(app);
        }
        if Instant::now() >= deadline { return Err("DSi did not confirm the cartridge operation".into()); }
        std::thread::sleep(Duration::from_millis(50));
    }
}
