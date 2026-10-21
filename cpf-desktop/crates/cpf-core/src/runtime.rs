use crate::machine::{read_json, Result, Workspace};
use serde::Serialize;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Serialize)]
pub struct LaunchResult {
    pub pid: u32,
    pub backend: String,
    pub note: String,
}

pub trait RuntimeAdapter: Send + Sync {
    fn supports(&self, platform: &str) -> bool;
    fn launch(&self, workspace: &Workspace, machine_id: &str) -> Result<LaunchResult>;
}

pub struct RuntimeRegistry {
    adapters: Vec<Box<dyn RuntimeAdapter>>,
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self {
            adapters: vec![Box::new(DsiLegacyWindowAdapter)],
        }
    }
}

impl RuntimeRegistry {
    pub fn launch(&self, workspace: &Workspace, machine_id: &str) -> Result<LaunchResult> {
        let machine = workspace.machine_dir(machine_id)?;
        let profile = read_json(&machine.join("machine.json"))?;
        let platform = profile
            .get("platform")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        self.adapters
            .iter()
            .find(|adapter| adapter.supports(platform))
            .ok_or_else(|| format!("No runtime adapter is installed for {platform}"))?
            .launch(workspace, machine_id)
    }
}

struct DsiLegacyWindowAdapter;

impl RuntimeAdapter for DsiLegacyWindowAdapter {
    fn supports(&self, platform: &str) -> bool {
        platform == "Nintendo DSi"
    }

    fn launch(&self, workspace: &Workspace, machine_id: &str) -> Result<LaunchResult> {
        let ready = workspace.readiness(machine_id)?;
        if !ready.ready_to_attempt_boot {
            return Err(format!(
                "Runtime files missing: {}",
                ready.missing.join(", ")
            ));
        }
        let machine = workspace.machine_dir(machine_id)?;
        if machine.join("runtime/libretro/running.json").is_file() {
            return Err(
                "This machine already has a runtime session. Close it before launching again."
                    .into(),
            );
        }
        let python = workspace.root.join(".venv/Scripts/pythonw.exe");
        let window = workspace.root.join("cpf_dsi_window.pyw");
        if !python.is_file() || !window.is_file() {
            return Err("The transitional CPF DSi window adapter is unavailable".into());
        }
        let name = machine_id
            .strip_suffix(".cpfmachine")
            .ok_or("Invalid machine ID")?;
        let process = spawn_hidden(
            &python,
            &[window.to_string_lossy().as_ref(), "--machine", name],
            &workspace.root,
        )?;
        Ok(LaunchResult { pid: process.id(), backend: "melonDS DS libretro through CPF DSi window".into(),
            note: "The existing CPF runtime window owns screen, input, and audio during this migration.".into() })
    }
}

fn spawn_hidden(executable: &Path, args: &[&str], directory: &Path) -> Result<std::process::Child> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
        .spawn()
        .map_err(|e| format!("Could not launch runtime: {e}"))
}

impl Workspace {
    pub fn open_legacy_tools(&self) -> Result<LaunchResult> {
        let python = self.root.join(".venv/Scripts/pythonw.exe");
        let viewer = self.root.join("app.pyw");
        if !python.is_file() || !viewer.is_file() {
            return Err("Python CPF tools are unavailable".into());
        }
        let process = spawn_hidden(&python, &[viewer.to_string_lossy().as_ref()], &self.root)?;
        Ok(LaunchResult {
            pid: process.id(),
            backend: "CPF Python tools".into(),
            note: "Advanced preservation tools remain available while they move to Rust.".into(),
        })
    }
}
