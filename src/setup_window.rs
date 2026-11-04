//! Persistent, labelled public file setup, following the DSi import form.
use std::{
    io,
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr::{null, null_mut},
    thread::JoinHandle,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::Dialogs::*, WindowsAndMessaging::*},
};

pub struct Selection {
    pub profile: PathBuf,
    pub backend: PathBuf,
    pub artwork: Option<PathBuf>,
}
struct State {
    edits: [HWND; 4],
    buttons: [HWND; 4],
    start: HWND,
    cancel: HWND,
    status: HWND,
    background: HBRUSH,
    input_background: HBRUSH,
    worker: Option<JoinHandle<Result<Selection, &'static str>>>,
    result: Option<Selection>,
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe fn text(window: HWND, value: &str) {
    SetWindowTextW(window, wide(value).as_ptr());
}
unsafe fn path_value(window: HWND) -> Option<PathBuf> {
    let size = GetWindowTextLengthW(window);
    if size == 0 {
        return None;
    }
    let mut data = vec![0u16; size as usize + 1];
    let length = GetWindowTextW(window, data.as_mut_ptr(), data.len() as i32);
    (length > 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&data[..length as usize])))
}
unsafe fn enabled(state: &State, value: bool) {
    for control in state
        .edits
        .iter()
        .chain(state.buttons.iter())
        .copied()
        .chain([state.start, state.cancel])
    {
        EnableWindow(control, value as i32);
    }
}
unsafe fn pick(owner: HWND, index: usize) -> Option<PathBuf> {
    let titles = [
        "Select original USA HOME Menu CXI — 0004003000008F02",
        "Select the separate matching HOME Menu backup CXI",
        "Select azahar.exe in your private portable runtime folder",
        "Select your local 2DS artwork — 1536 × 1024 PNG",
    ];
    let filter = wide(match index {
        2 => "Portable Azahar (*.exe)\0*.exe\0",
        3 => "2DS artwork (*.png)\0*.png\0",
        _ => "Original HOME Menu (*.cxi)\0*.cxi\0",
    });
    let title = wide(titles[index]);
    let mut path = vec![0u16; 32768];
    let mut dialog: OPENFILENAMEW = zeroed();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
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
        return None;
    }
    let end = path.iter().position(|c| *c == 0)?;
    Some(std::ffi::OsString::from_wide(&path[..end]).into())
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if !ptr.is_null() {
        let state = &mut *ptr;
        match msg {
            WM_CTLCOLORSTATIC => {
                let dc = wparam as HDC;
                SetTextColor(dc, 0x00e8e6e4);
                SetBkColor(dc, 0x002d2a28);
                return state.background as LRESULT;
            }
            WM_CTLCOLOREDIT => {
                let dc = wparam as HDC;
                SetTextColor(dc, 0x00ffffff);
                SetBkColor(dc, 0x003b3734);
                return state.input_background as LRESULT;
            }
            WM_CLOSE => {
                if state.worker.is_none() {
                    DestroyWindow(hwnd);
                }
                return 0;
            }
            WM_COMMAND if (wparam >> 16) == 0 => {
                match wparam & 0xffff {
                    id @ 100..=103 if state.worker.is_none() => {
                        let index = id - 100;
                        if let Some(path) = pick(hwnd, index) {
                            let value: Vec<u16> =
                                path.as_os_str().encode_wide().chain(Some(0)).collect();
                            SetWindowTextW(state.edits[index], value.as_ptr());
                            text(state.status,"Selection updated. Your files stay private and are checked when you start.");
                        }
                    }
                    2 if state.worker.is_none() => {
                        DestroyWindow(hwnd);
                    }
                    1 if state.worker.is_none() => {
                        let required = [
                            path_value(state.edits[0]),
                            path_value(state.edits[1]),
                            path_value(state.edits[2]),
                        ];
                        let [Some(home), Some(backup), Some(backend)] = required else {
                            text(state.status,"Choose HOME Menu, its separate backup, and the Azahar executable. Device artwork is optional.");
                            return 0;
                        };
                        let artwork = path_value(state.edits[3]);
                        text(
                            state.status,
                            "Checking your files and preparing private working state…",
                        );
                        enabled(state, false);
                        state.worker = Some(std::thread::spawn(move || {
                            if let Some(path) = &artwork {
                                crate::preview::load_shell_from_path(crate::preview::ShellModel::Original2ds,path).map_err(|_|"Choose a valid RGBA 2DS PNG at 1536 × 1024 pixels, or leave Device artwork blank.")?;
                            }
                            let (profile, backend) =
                                crate::public_setup::prepare(&home, &backup, &backend)?;
                            Ok(Selection {
                                profile,
                                backend,
                                artwork,
                            })
                        }));
                    }
                    _ => {}
                }
                return 0;
            }
            WM_TIMER => {
                if state
                    .worker
                    .as_ref()
                    .is_some_and(|worker| worker.is_finished())
                {
                    match state.worker.take().unwrap().join() {
                        Ok(Ok(selection)) => {
                            state.result = Some(selection);
                            DestroyWindow(hwnd);
                        }
                        Ok(Err(message)) => {
                            text(state.status, message);
                            enabled(state, true);
                        }
                        Err(_) => {
                            text(state.status,"Setup could not finish. Your selections are kept; check the files and try again.");
                            enabled(state, true);
                        }
                    }
                }
                return 0;
            }
            _ => {}
        }
    }
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub fn show() -> io::Result<Option<Selection>> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let name = wide("CPFPublicSetupForm");
        let background = CreateSolidBrush(0x002d2a28);
        let input_background = CreateSolidBrush(0x003b3734);
        let mut class: WNDCLASSW = zeroed();
        class.hInstance = instance;
        class.lpszClassName = name.as_ptr();
        class.lpfnWndProc = Some(procedure);
        class.hbrBackground = background;
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            name.as_ptr(),
            wide("Set up Nintendo 2DS").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            910,
            660,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        let font = CreateFontW(
            17,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let heading = CreateFontW(
            27,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let control = |class: &str,
                       value: &str,
                       style: u32,
                       x: i32,
                       y: i32,
                       w: i32,
                       h: i32,
                       id: usize|
         -> HWND {
            let child = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(value).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                w,
                h,
                window,
                id as HMENU,
                instance,
                null_mut(),
            );
            SendMessageW(child, WM_SETFONT, font as usize, 1);
            child
        };
        let title = control("STATIC", "Set up your Nintendo 2DS", 0, 30, 23, 840, 38, 0);
        SendMessageW(title, WM_SETFONT, heading as usize, 1);
        control(
            "STATIC",
            "Original USA 2DS · 6.0.0-12U preservation target",
            0,
            30,
            68,
            840,
            24,
            0,
        );
        control("STATIC","Choose your local files below. Originals stay unchanged; CPF uses private working state.",0,30,100,840,25,0);
        let labels = [
            "HOME Menu",
            "Verified backup",
            "Azahar runtime",
            "Device artwork",
        ];
        let hints = [
            "Original USA export: 0004003000008F02… (CTR-N-HMME).cxi",
            "Separate CXI copy matching your HOME Menu",
            "azahar.exe from your separately extracted portable runtime folder",
            "Optional 2DS PNG, 1536 × 1024. Leave blank for the built-in device shell.",
        ];
        let mut edits = [null_mut(); 4];
        let mut buttons = [null_mut(); 4];
        for i in 0..4 {
            let y = 149 + i as i32 * 81;
            control("STATIC", labels[i], 0, 30, y + 8, 155, 28, 0);
            edits[i] = control(
                "EDIT",
                "",
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                190,
                y,
                555,
                34,
                200 + i,
            );
            SendMessageW(
                edits[i],
                0x1501,
                0,
                wide(if i == 3 {
                    "Built-in 2DS shell"
                } else {
                    "No file selected"
                })
                .as_ptr() as isize,
            );
            buttons[i] = control("BUTTON", "Browse…", WS_TABSTOP, 759, y, 105, 35, 100 + i);
            control("STATIC", hints[i], 0, 190, y + 41, 675, 30, 0);
        }
        let status=control("STATIC","Select the three required files, then choose Set up and start 2DS. No NAND or keys are needed for this setup.",0,30,484,830,49,0);
        let cancel = control("BUTTON", "Cancel", WS_TABSTOP, 510, 554, 105, 39, 2);
        let start = control(
            "BUTTON",
            "Set up and start 2DS",
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            631,
            554,
            233,
            39,
            1,
        );
        let mut state = Box::new(State {
            edits,
            buttons,
            start,
            cancel,
            status,
            background,
            input_background,
            worker: None,
            result: None,
        });
        SetWindowLongPtrW(window, GWLP_USERDATA, (&mut *state as *mut State) as isize);
        SetTimer(window, 1, 50, None);
        ShowWindow(window, SW_SHOW);
        SetFocus(edits[0]);
        let mut message: MSG = zeroed();
        let mut failure = None;
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == 0 {
                break;
            }
            if result < 0 {
                failure = Some(io::Error::last_os_error());
                break;
            }
            if IsDialogMessageW(window, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Some(worker) = state.worker.take() {
            let _ = worker.join();
        }
        if IsWindow(window) != 0 {
            DestroyWindow(window);
        }
        UnregisterClassW(name.as_ptr(), instance);
        DeleteObject(font as _);
        DeleteObject(heading as _);
        DeleteObject(background as _);
        DeleteObject(input_background as _);
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(state.result.take())
    }
}
