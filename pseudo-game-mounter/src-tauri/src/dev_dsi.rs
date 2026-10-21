use crate::melonds_ui::{EmulatorSession, SlotObservation};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Deserialize)]
struct Descriptor {
    version: u32,
    pid: u32,
    port: u16,
    token: String,
    title: String,
}

fn directory() -> Result<PathBuf, String> {
    Ok(PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
        .join("CPF").join("DSiSessions"))
}

fn descriptor(pid: u32) -> Result<Descriptor, String> {
    let path = directory()?.join(format!("{pid}.json"));
    let descriptor: Descriptor = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if descriptor.version != 1 || descriptor.pid != pid || descriptor.port == 0 {
        return Err("Unsupported DSi session descriptor".into());
    }
    Ok(descriptor)
}

fn request(descriptor: &Descriptor, command: &str, path: Option<&std::path::Path>) -> Result<Value, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], descriptor.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(350))
        .map_err(|e| format!("DSi session is unavailable: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(35))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    let message = json!({"token":descriptor.token,"command":command,
        "path":path.map(|path|path.to_string_lossy().into_owned())});
    writeln!(stream, "{message}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let response: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    if response["ok"] != true {
        return Err(response["error"].as_str().unwrap_or("DSi bridge request failed").into());
    }
    Ok(response)
}

pub fn sessions() -> Vec<EmulatorSession> {
    let Ok(root) = directory() else { return Vec::new() };
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(pid) = path.file_stem().and_then(|name|name.to_str()).and_then(|name|name.parse::<u32>().ok()) else { continue };
        let Ok(descriptor) = descriptor(pid) else { continue };
        let Ok(status) = request(&descriptor, "status", None) else { continue };
        let title = status["machine_id"].as_str()
            .map_or_else(||descriptor.title.clone(), |id|format!("{} ({id})",descriptor.title));
        sessions.push(EmulatorSession {pid, title});
    }
    sessions.sort_by_key(|session| session.pid);
    sessions
}

pub fn is_session(pid: u32) -> bool {
    descriptor(pid).is_ok_and(|descriptor| request(&descriptor, "status", None).is_ok())
}

fn observation(status: &Value) -> SlotObservation {
    let inserted = status["inserted"].as_bool() == Some(true);
    let label = if inserted {
        status["title"].as_str().unwrap_or("Cartridge inserted").to_owned()
    } else if status["running"] != true {
        "DSi starting".into()
    } else { "Empty".into() };
    SlotObservation {inserted, label}
}

pub fn check(pid: u32) -> Result<SlotObservation, String> {
    let descriptor = descriptor(pid)?;
    let status = request(&descriptor, "status", None)?;
    Ok(observation(&status))
}

pub fn virtual_sd_host(pid: u32) -> Result<PathBuf, String> {
    let descriptor = descriptor(pid)?;
    let status = request(&descriptor, "status", None)?;
    if status["running"] != true {
        return Err("Start the DSi machine before attaching its virtual SD library".into());
    }
    let path = status["sd_host_path"].as_str()
        .ok_or("This DSi has no active virtual SD host folder. Enable virtual SD and restart it.")?;
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.is_dir() {
        return Err("The DSi virtual SD host folder is unavailable".into());
    }
    Ok(path)
}

pub fn insert(pid: u32, path: &std::path::Path) -> Result<SlotObservation, String> {
    let descriptor = descriptor(pid)?;
    Ok(observation(&request(&descriptor, "insert", Some(path))?))
}

pub fn eject(pid: u32) -> Result<SlotObservation, String> {
    let descriptor = descriptor(pid)?;
    Ok(observation(&request(&descriptor, "eject", None)?))
}
