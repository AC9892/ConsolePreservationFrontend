#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("The 2DS desktop bundle requires Windows.");
}

#[cfg(windows)]
fn main() {
    if let Err(message) = launch() {
        let title: Vec<u16> = "CPF 2DS".encode_utf16().chain(Some(0)).collect();
        let body: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn launch() -> Result<(), &'static str> {
    use std::os::windows::process::CommandExt;
    #[cfg(not(feature = "public-release"))]
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    let executable =
        std::env::current_exe().map_err(|_| "Unable to locate the 2DS application.")?;
    let bundle = executable
        .parent()
        .ok_or("Unable to locate the 2DS bundle.")?;
    let host = bundle.join("cpf.exe");
    let preview = bundle.join("cpf-preview.exe");
    #[cfg(not(feature = "public-release"))]
    let azahar = bundle.join("Azahar").join("azahar.exe");
    if !host.is_file() || !preview.is_file() {
        return Err("The CPF package is incomplete. Keep 2DS.exe, cpf.exe, and cpf-preview.exe in the same folder.");
    }

    #[cfg(feature = "public-release")]
    let selection =
        match cpf::setup_window::show().map_err(|_| "Unable to open the setup window.")? {
            Some(selection) => selection,
            None => return Ok(()),
        };
    #[cfg(feature = "public-release")]
    let (profile, azahar) = (selection.profile, selection.backend);
    #[cfg(not(feature = "public-release"))]
    let profile = match std::env::var_os("CPF_2DS_PROFILE") {
        Some(path) => PathBuf::from(path),
        None => {
            let local = bundle.join("private").join("original-2ds.cpf-profile.toml");
            if local.is_file() {
                local
            } else {
                bundle
                    .parent()
                    .ok_or("Unable to locate the private profile.")?
                    .join("private")
                    .join("original-2ds.cpf-profile.toml")
            }
        }
    };
    if !azahar.is_file() {
        return Err("The selected portable backend is unavailable.");
    }
    if !profile.is_file() {
        return Err("The original 2DS profile is unavailable. Set CPF_2DS_PROFILE to its private local path.");
    }

    let mut command = Command::new(host);
    command
        .arg("home-shell")
        .arg(profile)
        .arg(azahar)
        .current_dir(bundle)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: the shell is the only visible CPF window.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(feature = "public-release")]
    {
        command.env_remove("CPF_2DS_SHELL");
        if let Some(artwork) = selection.artwork {
            command.env("CPF_2DS_SHELL", artwork);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Unable to start the 2DS runtime.")?;
    let _job = cpf::process_lifecycle::ProcessJob::attach(&mut child)
        .map_err(|_| "Unable to manage the 2DS runtime.")?;
    let result = child
        .wait_with_output()
        .map_err(|_| "Unable to wait for the 2DS runtime.")?;
    if !result.status.success() {
        if std::str::from_utf8(&result.stderr).ok().map(str::trim)
            == Some(cpf::process_lifecycle::BACKEND_EXITED_MESSAGE)
        {
            return Err("The emulation backend stopped unexpectedly. The 2DS window has been closed. Check the private runtime log before reopening.");
        }
        if std::str::from_utf8(&result.stderr).ok().map(str::trim)
            == Some(cpf::azahar::BackendError::InputConfigurationUnavailable.label())
        {
            return Err("Azahar's selected input profile cannot drive the pictured buttons. Assign keyboard keys to A/B/X/Y, the D-pad, Start, and Select in its portable configuration.");
        }
        return Err("The 2DS session ended with an error. Check the private runtime log.");
    }
    Ok(())
}
