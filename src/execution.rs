use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use walkdir::WalkDir;

#[derive(Clone, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub backend_profile: String,
    pub sandbox: String,
    pub source: String,
    pub backend_sha256: String,
    pub executable_sha256: String,
    pub files_copied: usize,
    pub status: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub backend_profile: String,
    pub sandbox: String,
    pub phase: String,
    pub first_failure: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub available_commit_bytes: Option<u64>,
    pub required_backing_bytes: u64,
    pub required_headroom_bytes: u64,
    pub guest_startup_messages: Vec<String>,
    pub last_logged_error: Option<String>,
    pub authentic_execution_verified: bool,
    #[serde(default)]
    pub runtime_diagnostics: crate::diagnostics::RuntimeDiagnostics,
}
fn linked(path: &Path) -> Result<bool, String> {
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
fn hash(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn copy_tree(from: &Path, to: &Path) -> Result<usize, String> {
    let mut count = 0;
    for item in WalkDir::new(from).follow_links(false) {
        let item = item.map_err(|e| e.to_string())?;
        if linked(item.path())? {
            return Err(format!("Refusing linked source: {}", item.path().display()));
        }
        let output = to.join(item.path().strip_prefix(from).map_err(|e| e.to_string())?);
        if item.file_type().is_dir() {
            fs::create_dir_all(output).map_err(|e| e.to_string())?;
        } else if item.file_type().is_file() {
            fs::copy(item.path(), output).map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    Ok(count)
}
pub fn prepare(source: &Path, backend: &Path, parent: &Path) -> Result<Plan, String> {
    if linked(source)? || linked(backend)? || linked(parent)? {
        return Err("Use real paths without reparse points".into());
    }
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let backend = fs::canonicalize(backend).map_err(|e| e.to_string())?;
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if parent.starts_with(&source) || source.starts_with(&parent) {
        return Err("Sandbox parent must be outside the dump and must not contain it".into());
    }
    let app = source.join("system_ex/app/NPXS20001");
    if !app.join("eboot.bin").is_file() {
        return Err(
            "Expected system_ex/app/NPXS20001/eboot.bin under the selected dump root".into(),
        );
    }
    let backend_hash = hash(&backend)?;
    let profile = crate::backend::profile_for_hash(&backend_hash)
        .ok_or("Select a verified official backend: v0.20.0 default or v0.11.0 comparison")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sandbox = parent.join(format!("shellui-{now}"));
    fs::create_dir(&sandbox).map_err(|e| e.to_string())?;
    fs::create_dir(sandbox.join("runtime")).map_err(|e| e.to_string())?;
    fs::create_dir_all(sandbox.join("user/sys_modules")).map_err(|e| e.to_string())?;
    fs::copy(&backend, sandbox.join("runtime/shadPS4.exe")).map_err(|e| e.to_string())?;
    let mut files = copy_tree(&app, &sandbox.join("app0"))?;
    for directory in [
        "system/common/lib",
        "system/priv/lib",
        "system_ex/common_ex/lib",
    ] {
        let from = source.join(directory);
        if !from.is_dir() {
            continue;
        }
        for item in WalkDir::new(&from).follow_links(false) {
            let item = item.map_err(|e| e.to_string())?;
            if linked(item.path())? {
                return Err("Linked library source refused".into());
            }
            if !item.file_type().is_file() {
                continue;
            }
            if item.path().extension().and_then(|v| v.to_str()) != Some("sprx") {
                continue;
            }
            let output = sandbox.join("user/sys_modules").join(item.file_name());
            if output.exists() {
                if hash(&output)? != hash(item.path())? {
                    return Err(format!(
                        "Conflicting library filename: {}",
                        item.file_name().to_string_lossy()
                    ));
                }
                continue;
            }
            fs::copy(item.path(), output).map_err(|e| e.to_string())?;
            files += 1;
        }
    }
    let executable_hash = hash(&sandbox.join("app0/eboot.bin"))?;
    if executable_hash != hash(&app.join("eboot.bin"))? {
        return Err("Staged executable hash differs from source".into());
    }
    let config = serde_json::json!({"General":{"discord_rpc_enabled":false,"sys_modules_dir":"","extra_dmem_in_mbytes":0,"extra_fmem_in_mbytes":0},"Network":{"connected_to_network":false,"shad_net_enabled":false,"enable_upnp":false},"Log":{"enable":true,"sync":true,"type":"wincolor"},"GPU":{"full_screen":false}});
    if profile.id == "shadps4-v0.11.0-comparison" {
        fs::write(sandbox.join("user/config.toml"), "[General]\nisConnectedToNetwork = false\nenableDiscordRPC = false\nlogType = \"sync\"\n").map_err(|e|e.to_string())?;
    } else {
        fs::write(
            sandbox.join("user/config.json"),
            serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    let plan=Plan{backend_profile:profile.id.into(),sandbox:sandbox.to_string_lossy().into_owned(),source:source.to_string_lossy().into_owned(),backend_sha256:backend_hash,executable_sha256:executable_hash,files_copied:files,status:"Prepared copies only; no source dump paths are passed to the backend. Native system mounts and full service environment are not implemented.".into()};
    fs::write(
        sandbox.join("plan.json"),
        serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(plan)
}
#[cfg(windows)]
pub fn available_commit() -> Option<u64> {
    #[repr(C)]
    struct Memory {
        length: u32,
        load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page: u64,
        avail_page: u64,
        total_virtual: u64,
        avail_virtual: u64,
        extended: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut Memory) -> i32;
    }
    let mut m = Memory {
        length: std::mem::size_of::<Memory>() as u32,
        load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page: 0,
        avail_page: 0,
        total_virtual: 0,
        avail_virtual: 0,
        extended: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 {
        Some(m.avail_page)
    } else {
        None
    }
}
#[cfg(not(windows))]
pub fn available_commit() -> Option<u64> {
    None
}
pub fn run(plan: &Plan) -> Result<Attempt, String> {
    let profile = crate::backend::profile_for_hash(&plan.backend_sha256)
        .ok_or("Plan does not identify a verified backend")?;
    let base = PathBuf::from(&plan.sandbox);
    let available = available_commit();
    let mut attempt = Attempt {
        backend_profile: profile.id.into(),
        sandbox: plan.sandbox.clone(),
        phase: "preflight".into(),
        first_failure: String::new(),
        exit_code: None,
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        available_commit_bytes: available,
        required_backing_bytes: profile.backing_bytes,
        required_headroom_bytes: profile.headroom_bytes,
        guest_startup_messages: vec![],
        last_logged_error: None,
        authentic_execution_verified: false,
        runtime_diagnostics: Default::default(),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_base = base.join(format!("attempt-{now}"));
    if available.is_none_or(|v| v < profile.backing_bytes + profile.headroom_bytes) {
        attempt.first_failure=format!("Available Windows commit is insufficient or unknown for {}: {:.2} GiB backing plus {:.2} GiB headroom. Available: {:?} bytes. No guest launched.",profile.id,profile.backing_bytes as f64/1073741824.0,profile.headroom_bytes as f64/1073741824.0,available);
    } else {
        let backend = base.join("runtime/shadPS4.exe");
        if hash(&backend)? != plan.backend_sha256
            || hash(&base.join("app0/eboot.bin"))? != plan.executable_sha256
        {
            return Err("Staged executable changed; prepare a fresh sandbox".into());
        }
        let stdout_path = log_base.with_extension("stdout.txt");
        let stderr_path = log_base.with_extension("stderr.txt");
        let out = File::create(&stdout_path).map_err(|e| e.to_string())?;
        let err = File::create(&stderr_path).map_err(|e| e.to_string())?;
        let mut command = Command::new(backend);
        command
            .current_dir(&base)
            .args([
                "--fullscreen",
                "false",
                "--ignore-game-patch",
                "app0/eboot.bin",
            ])
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                attempt.exit_code = status.code();
                break;
            }
            if started.elapsed() > Duration::from_secs(20) {
                child.kill().map_err(|e| e.to_string())?;
                let _ = child.wait();
                attempt.timed_out = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        attempt.phase = "backend-attempt; guest execution unverified".into();
        attempt.stdout = stdout_path.to_string_lossy().into_owned();
        attempt.stderr = stderr_path.to_string_lossy().into_owned();
        let output = fs::read_to_string(&stdout_path).unwrap_or_default();
        attempt.runtime_diagnostics = crate::diagnostics::analyze(&output);
        let first_diagnostic = output.lines().find(|line| {
            line.contains("<Critical>")
                || line.contains("<Error>")
                || line.contains("Assertion Failed")
        });
        attempt.last_logged_error = output
            .lines()
            .rev()
            .find(|line| line.contains("<Critical>") || line.contains("<Error>"))
            .map(str::to_owned);
        attempt.guest_startup_messages = output
            .lines()
            .filter(|line| line.contains("[Tty]") && line.contains("SysPrxPreLoadStaticBlock"))
            .take(16)
            .map(str::to_owned)
            .collect();
        if !attempt.guest_startup_messages.is_empty() {
            attempt.phase = "original-startup-messages-observed; dashboard unverified".into();
        }
        attempt.first_failure = format!(
            "{} First logged diagnostic (not necessarily causal): {}",
            if attempt.timed_out {
                "Stopped after 20 seconds".into()
            } else {
                format!("Backend exit code {:?}", attempt.exit_code)
            },
            first_diagnostic.unwrap_or("none; inspect complete logs")
        );
    }
    fs::write(
        log_base.with_extension("json"),
        serde_json::to_vec_pretty(&attempt).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(attempt)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_plans_keep_the_original_backend_memory_requirement() {
        let json = r#"{"sandbox":"copied","source":"original","backend_sha256":"d2844a3ad350e2c51edc6e61dc6684b29dcde76e23746f1bdf97cb9367ec88c3","executable_sha256":"test","files_copied":1,"status":"prepared"}"#;
        let plan: Plan = serde_json::from_str(json).unwrap();
        let profile = crate::backend::profile_for_hash(&plan.backend_sha256).unwrap();
        assert_eq!(profile.backing_bytes, 8448 * 1024 * 1024);
        assert_eq!(profile.id, "shadps4-v0.20.0");
    }
}
