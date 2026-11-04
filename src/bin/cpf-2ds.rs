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
    let (profile, azahar) = {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDOK, MB_ICONINFORMATION, MB_OKCANCEL,
        };
        let title: Vec<u16> = "CPF 2DS — Files needed"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let body: Vec<u16> = "You will select three files:\n\n1. Your decrypted original USA HOME Menu CXI (title 0004003000008F02), from the 6.0.0-12U preservation set.\n2. A separate verified backup of that same HOME Menu CXI.\n3. azahar.exe from a separately extracted portable Azahar folder.\n\nCPF creates private writable state beside Azahar and generates the profile for you. Keep this runtime separate from your preserved files and the public upload folder. Existing settings and titles are not replaced. No NAND, keys, or game is needed for these setup steps.\n\nSee INSTALLATION.md for details. Continue?".encode_utf16().chain(Some(0)).collect();
        if unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OKCANCEL | MB_ICONINFORMATION,
            )
        } != IDOK
        {
            return Ok(());
        }
        let Some(home) = pick_private_file(0)? else {
            return Ok(());
        };
        let Some(backup) = pick_private_file(1)? else {
            return Ok(());
        };
        let Some(azahar) = pick_private_file(2)? else {
            return Ok(());
        };
        cpf::public_setup::prepare(&home, &backup, &azahar)?
    };
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

    let mut child = Command::new(host)
        .arg("home-shell")
        .arg(profile)
        .arg(azahar)
        .current_dir(bundle)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: the shell is the only visible CPF window.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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

#[cfg(all(windows, feature = "public-release"))]
fn pick_private_file(step: u8) -> Result<Option<std::path::PathBuf>, &'static str> {
    use std::{
        mem::{size_of, zeroed},
        os::windows::ffi::OsStringExt,
    };
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let title: Vec<u16> = match step {
        0 => "1 of 3 — Select original USA HOME Menu 0004003000008F02 CXI",
        1 => "2 of 3 — Select the separate matching HOME Menu backup CXI",
        _ => "3 of 3 — Select azahar.exe in your separate private runtime folder",
    }
    .encode_utf16()
    .chain(Some(0))
    .collect();
    let filter: Vec<u16> = if step == 2 {
        "Portable backend (*.exe)\0*.exe\0\0"
    } else {
        "Decrypted original USA HOME Menu (*.cxi)\0*.cxi\0\0"
    }
    .encode_utf16()
    .collect();
    let mut path = vec![0u16; 32768];
    unsafe {
        let mut dialog: OPENFILENAMEW = zeroed();
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = path.as_mut_ptr();
        dialog.nMaxFile = path.len() as u32;
        dialog.Flags = OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return if CommDlgExtendedError() == 0 {
                Ok(None)
            } else {
                Err("Unable to open setup file picker.")
            };
        }
    }
    let end = path
        .iter()
        .position(|c| *c == 0)
        .ok_or("Selected path is invalid.")?;
    Ok(Some(std::ffi::OsString::from_wide(&path[..end]).into()))
}
