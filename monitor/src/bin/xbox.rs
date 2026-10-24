#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "xbox/monitor.rs"]
mod monitor;

#[cfg(windows)]
mod native {
    use cpf_xbox360::{inspect_backend_local, Collection, Store};
    use super::monitor::{Monitor, Report};
    use std::{cell::RefCell, ffi::c_void, ptr::null_mut, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
    use windows_sys::Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{GetStockObject, UpdateWindow, DEFAULT_GUI_FONT, WHITE_BRUSH},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };

    const IMPORT: u16 = 101;
    const REFRESH: u16 = 102;
    const VERIFY: u16 = 103;
    const COLLECTIONS: u16 = 104;
    const FILES: u16 = 105;
    const PREVIEW: u16 = 106;
    const BACKEND: u16 = 107;
    const MONITOR_TAB: u16 = 108;
    const FIRMWARE_TAB: u16 = 109;
    const WATCH_LOG: u16 = 110;
    const WORK_DONE: u32 = WM_APP + 1;

    struct MonitorControls {
        panel: HWND, state: HWND, processes: HWND, log_path: HWND,
        log_status: HWND, diagnostics: HWND, events: HWND,
    }

    struct Ui {
        hwnd: HWND, path: HWND, mode: HWND, source: HWND, label: HWND, version: HWND,
        collections_list: HWND, files_list: HWND, details: HWND, backend_path: HWND, status: HWND,
        inspector_panel: HWND, monitor: MonitorControls, watched_log: Arc<Mutex<String>>,
        monitor_stop: Arc<AtomicBool>, store: Option<Arc<Store>>, collections: Vec<Collection>, busy: bool,
    }
    enum WorkResult { Import(bool), Verify(String), Preview(String), Backend(String), Monitor(Report) }
    thread_local! { static UI: RefCell<Option<Ui>> = const { RefCell::new(None) }; }

    fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }
    unsafe fn control(parent: HWND, instance: HINSTANCE, class: &str, title: &str, style: u32,
        x: i32, y: i32, width: i32, height: i32, id: u16) -> HWND {
        let class = wide(class);
        let title = wide(title);
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, x, y, width, height,
            parent, id as usize as *mut c_void, instance, null_mut());
        if !hwnd.is_null() { SendMessageW(hwnd, WM_SETFONT, GetStockObject(DEFAULT_GUI_FONT) as usize, 1); }
        hwnd
    }
    unsafe fn text(hwnd: HWND) -> String {
        let mut buffer = [0u16; 4096];
        let length = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buffer[..length]).trim().to_owned()
    }
    unsafe fn set_text(hwnd: HWND, value: &str) { SetWindowTextW(hwnd, wide(value).as_ptr()); }
    unsafe fn add_item(hwnd: HWND, value: &str, message: u32) {
        SendMessageW(hwnd, message, 0, wide(value).as_ptr() as isize);
    }
    unsafe fn selected(hwnd: HWND, message: u32) -> Option<usize> {
        let index = SendMessageW(hwnd, message, 0, 0);
        (index >= 0).then_some(index as usize)
    }

    unsafe fn populate_files(ui: &mut Ui) {
        SendMessageW(ui.files_list, LB_RESETCONTENT, 0, 0);
        let Some(index) = selected(ui.collections_list, LB_GETCURSEL) else {
            set_text(ui.details, "Select a collection to inspect its files."); return;
        };
        let Some(collection) = ui.collections.get(index) else { return; };
        for file in &collection.files { add_item(ui.files_list, &file.path, LB_ADDSTRING); }
        let mut lines = vec![
            format!("Collection: {}", collection.label),
            format!("Files: {}", collection.files.len()),
            format!("Dashboard: {} ({})", collection.dashboard_version.as_deref().unwrap_or("unknown"), collection.version_evidence),
            format!("Motherboard: {}", collection.motherboard.as_deref().unwrap_or("unknown")),
            format!("Source: {}", collection.source),
        ];
        lines.extend(collection.issues.iter().map(|issue| format!("Warning: {issue}")));
        set_text(ui.details, &lines.join("\r\n"));
    }
    unsafe fn populate_details(ui: &Ui) {
        let Some(collection_index) = selected(ui.collections_list, LB_GETCURSEL) else { return; };
        let Some(file_index) = selected(ui.files_list, LB_GETCURSEL) else { return; };
        let Some(file) = ui.collections.get(collection_index).and_then(|item| item.files.get(file_index)) else { return; };
        let mut lines = vec![format!("Path: {}", file.path), format!("Type: {}", file.kind),
            format!("Size: {} bytes", file.bytes), format!("SHA-256: {}", file.sha256)];
        if let Some(duplicate) = &file.duplicate_of { lines.push(format!("Duplicate of: {duplicate}")); }
        lines.extend(file.details.iter().cloned());
        set_text(ui.details, &lines.join("\r\n"));
    }
    unsafe fn refresh(ui: &mut Ui) {
        let Some(store) = &ui.store else {
            set_text(ui.status, "Firmware library unavailable. Live monitoring remains available.");
            return;
        };
        ui.collections = store.collections_snapshot();
        SendMessageW(ui.collections_list, LB_RESETCONTENT, 0, 0);
        for collection in &ui.collections {
            add_item(ui.collections_list, &format!("{}  ({} files)", collection.label, collection.files.len()), LB_ADDSTRING);
        }
        if !ui.collections.is_empty() { SendMessageW(ui.collections_list, LB_SETCURSEL, 0, 0); }
        populate_files(ui);
    }
    fn send_work(hwnd: usize, result: WorkResult) {
        let pointer = Box::into_raw(Box::new(result));
        unsafe {
            if PostMessageW(hwnd as HWND, WORK_DONE, 0, pointer as isize) == 0 { drop(Box::from_raw(pointer)); }
        }
    }
    unsafe fn begin_import(ui: &mut Ui) {
        if ui.busy { return; }
        let Some(store) = ui.store.clone() else { set_text(ui.status, "Firmware library unavailable."); return; };
        let path = text(ui.path);
        if path.is_empty() { set_text(ui.status, "Enter a file or folder path."); return; }
        let mode = match selected(ui.mode, CB_GETCURSEL).unwrap_or(0) { 1 => "manual", 2 => "advanced", _ => "automatic" }.to_owned();
        let source = if mode == "advanced" {
            Some(match selected(ui.source, CB_GETCURSEL).unwrap_or(0) { 1 => "NAND", 2 => "Flash", 3 => "HddX", _ => "other" }.to_owned())
        } else { None };
        let label = text(ui.label);
        let version = text(ui.version);
        let hwnd = ui.hwnd as usize;
        ui.busy = true;
        set_text(ui.status, "Scanning and hashing selected files. Originals remain unchanged…");
        std::thread::spawn(move || {
            let result = store.import_local(path, mode, source,
                (!label.is_empty()).then_some(label), (!version.is_empty()).then_some(version));
            send_work(hwnd, WorkResult::Import(result.is_ok()));
        });
    }
    unsafe fn begin_verify(ui: &mut Ui) {
        if ui.busy { return; }
        let Some(store) = ui.store.clone() else { set_text(ui.status, "Firmware library unavailable."); return; };
        let Some(index) = selected(ui.collections_list, LB_GETCURSEL) else { set_text(ui.status, "Select a collection first."); return; };
        let Some(collection) = ui.collections.get(index) else { return; };
        let id = collection.id.clone();
        let hwnd = ui.hwnd as usize;
        ui.busy = true;
        set_text(ui.status, "Rechecking source SHA-256 hashes…");
        std::thread::spawn(move || {
            let message = match store.verify_local(&id) {
                Ok(result) => format!("Verified {}/{} files; {} changed; {} missing or unreadable.",
                    result.verified, result.total, result.changed, result.missing_or_unreadable),
                Err(_) => "Verification unavailable.".into(),
            };
            send_work(hwnd, WorkResult::Verify(message));
        });
    }
    unsafe fn begin_preview(ui: &mut Ui) {
        if ui.busy { return; }
        let Some(store) = ui.store.clone() else { set_text(ui.status, "Firmware library unavailable."); return; };
        let Some(collection_index) = selected(ui.collections_list, LB_GETCURSEL) else { set_text(ui.status, "Select a collection first."); return; };
        let Some(file_index) = selected(ui.files_list, LB_GETCURSEL) else { set_text(ui.status, "Select a file first."); return; };
        let Some(collection) = ui.collections.get(collection_index) else { return; };
        let id = collection.id.clone();
        let hwnd = ui.hwnd as usize;
        ui.busy = true;
        set_text(ui.status, "Checking source hash before preview…");
        std::thread::spawn(move || {
            let content = store.preview_local(&id, file_index)
                .unwrap_or_else(|_| "Preview unavailable for this file type, or the source changed.".into());
            send_work(hwnd, WorkResult::Preview(content));
        });
    }
    unsafe fn begin_backend(ui: &mut Ui) {
        if ui.busy { return; }
        let path = text(ui.backend_path);
        if path.is_empty() { set_text(ui.status, "Enter a Xenia Canary executable path."); return; }
        let hwnd = ui.hwnd as usize;
        ui.busy = true;
        set_text(ui.status, "Inspecting backend header and SHA-256 without launching it…");
        std::thread::spawn(move || {
            let content = match inspect_backend_local(&path) {
                Ok(value) => format!("Backend: {}\r\nBytes: {}\r\nSHA-256: {}\r\nVersion: {}\r\nDashboard execution: {}",
                    value["backend"].as_str().unwrap_or("candidate"), value["bytes"],
                    value["sha256"].as_str().unwrap_or("unknown"),
                    value["version_status"].as_str().unwrap_or("unknown"),
                    value["dashboard_execution"].as_str().unwrap_or("Not Tested")),
                Err(_) => "No valid Xenia Canary executable at that path.".into(),
            };
            send_work(hwnd, WorkResult::Backend(content));
        });
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match message {
            WM_COMMAND => {
                let id = (wparam & 0xFFFF) as u16;
                let notification = ((wparam >> 16) & 0xFFFF) as u16;
                UI.with(|cell| {
                    if let Some(ui) = cell.borrow_mut().as_mut() {
                        match (id, notification) {
                            (MONITOR_TAB, 0) => { ShowWindow(ui.inspector_panel, SW_HIDE); ShowWindow(ui.monitor.panel, SW_SHOW); },
                            (FIRMWARE_TAB, 0) => { ShowWindow(ui.monitor.panel, SW_HIDE); ShowWindow(ui.inspector_panel, SW_SHOW); },
                            (WATCH_LOG, 0) => {
                                let path = text(ui.monitor.log_path);
                                *ui.watched_log.lock().unwrap() = path;
                                set_text(ui.monitor.log_status, "Checking selected log on next monitor update…");
                            },
                            (IMPORT, 0) => begin_import(ui), (REFRESH, 0) => refresh(ui),
                            (VERIFY, 0) => begin_verify(ui), (COLLECTIONS, 1) => populate_files(ui),
                            (FILES, 1) => populate_details(ui), (PREVIEW, 0) => begin_preview(ui),
                            (BACKEND, 0) => begin_backend(ui), _ => {}
                        }
                    }
                });
                0
            }
            WORK_DONE => {
                if lparam != 0 {
                    let result = *Box::from_raw(lparam as *mut WorkResult);
                    UI.with(|cell| {
                        if let Some(ui) = cell.borrow_mut().as_mut() {
                            match result {
                                WorkResult::Import(true) => { ui.busy = false; refresh(ui); set_text(ui.status, "Import complete."); },
                                WorkResult::Import(false) => { ui.busy = false; set_text(ui.status, "Import failed. Check the path, mode, and access permissions."); },
                                WorkResult::Verify(message) => { ui.busy = false; set_text(ui.status, &message); },
                                WorkResult::Preview(content) => { ui.busy = false; set_text(ui.details, &content); set_text(ui.status, "Preview complete."); },
                                WorkResult::Backend(content) => { ui.busy = false; set_text(ui.details, &content); set_text(ui.status, "Backend inspection complete."); },
                                WorkResult::Monitor(report) => {
                                    set_text(ui.monitor.state, &report.status);
                                    set_text(ui.monitor.processes, &report.processes);
                                    set_text(ui.monitor.log_status, &report.log_status);
                                    set_text(ui.monitor.diagnostics, &report.diagnostics);
                                    set_text(ui.monitor.events, &report.events);
                                },
                            }
                        }
                    });
                }
                0
            }
            WM_DESTROY => { UI.with(|cell| {
                if let Some(ui) = cell.borrow_mut().take() { ui.monitor_stop.store(true, Ordering::Relaxed); }
            }); PostQuitMessage(0); 0 }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    unsafe extern "system" fn panel_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if message == WM_COMMAND { return SendMessageW(GetParent(hwnd), message, wparam, lparam); }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }

    unsafe fn create_monitor_ui(parent: HWND, instance: HINSTANCE) -> MonitorControls {
        let label = WS_CHILD | WS_VISIBLE;
        let edit = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32;
        let output = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 |
            ES_AUTOVSCROLL as u32 | ES_READONLY as u32;
        control(parent, instance, "STATIC", "Xenia Canary live monitor", label, 20, 18, 600, 28, 0);
        control(parent, instance, "STATIC", "CPF Console Settings launches the emulator. This window observes it externally.", label, 20, 48, 950, 24, 0);
        let state = control(parent, instance, "STATIC", "Checking Xenia Canary…", label, 20, 88, 1000, 28, 0);
        control(parent, instance, "STATIC", "Process resources", label, 20, 124, 400, 20, 0);
        let processes = control(parent, instance, "EDIT", "Waiting for the first sample…", output, 20, 148, 1020, 104, 0);
        control(parent, instance, "STATIC", "Xenia log path (optional; clear to find the default log beside Xenia)", label, 20, 272, 800, 20, 0);
        let log_path = control(parent, instance, "EDIT", "", edit, 20, 296, 815, 26, 0);
        control(parent, instance, "BUTTON", "Watch log", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 850, 296, 190, 28, WATCH_LOG);
        let log_status = control(parent, instance, "STATIC", "Checking for an available log…", label, 20, 336, 1010, 24, 0);
        control(parent, instance, "STATIC", "Recent diagnostic log lines", label, 20, 372, 600, 20, 0);
        let diagnostics = control(parent, instance, "EDIT", "", output, 20, 396, 1020, 185, 0);
        control(parent, instance, "STATIC", "Observed launches and exits", label, 20, 599, 600, 20, 0);
        let events = control(parent, instance, "EDIT", "", output, 20, 623, 1020, 178, 0);
        MonitorControls { panel: parent, state, processes, log_path, log_status, diagnostics, events }
    }

    unsafe fn create_ui(parent: HWND, hwnd: HWND, store: Option<Arc<Store>>, instance: HINSTANCE,
        monitor: MonitorControls, watched_log: Arc<Mutex<String>>, monitor_stop: Arc<AtomicBool>) {
        let label_style = WS_CHILD | WS_VISIBLE;
        let edit_style = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32;
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        let list_style = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY as u32;
        control(parent, instance, "STATIC", "Xbox 360 preservation library", label_style, 20, 18, 400, 25, 0);
        control(parent, instance, "STATIC", "Read-only import and inspection", label_style, 20, 44, 700, 20, 0);
        control(parent, instance, "STATIC", "File or folder path", label_style, 20, 78, 250, 20, 0);
        let path = control(parent, instance, "EDIT", "", edit_style, 20, 101, 735, 25, 0);
        control(parent, instance, "BUTTON", "Import read-only", button_style, 770, 101, 135, 27, IMPORT);
        control(parent, instance, "BUTTON", "Refresh", button_style, 915, 101, 125, 27, REFRESH);
        control(parent, instance, "STATIC", "Mode", label_style, 20, 145, 140, 18, 0);
        let mode = control(parent, instance, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32, 20, 165, 170, 120, 0);
        for item in ["Automatic", "Manual", "Advanced"] { add_item(mode, item, CB_ADDSTRING); }
        SendMessageW(mode, CB_SETCURSEL, 0, 0);
        control(parent, instance, "STATIC", "Known source (Advanced)", label_style, 205, 145, 190, 18, 0);
        let source = control(parent, instance, "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32, 205, 165, 190, 120, 0);
        for item in ["Other / uncertain", "NAND", "Flash", "HddX"] { add_item(source, item, CB_ADDSTRING); }
        SendMessageW(source, CB_SETCURSEL, 0, 0);
        control(parent, instance, "STATIC", "Collection label", label_style, 410, 145, 200, 18, 0);
        let label = control(parent, instance, "EDIT", "", edit_style, 410, 165, 300, 25, 0);
        control(parent, instance, "STATIC", "Dashboard version (user supplied)", label_style, 725, 145, 300, 18, 0);
        let version = control(parent, instance, "EDIT", "", edit_style, 725, 165, 315, 25, 0);
        control(parent, instance, "STATIC", "Collections", label_style, 20, 207, 250, 20, 0);
        control(parent, instance, "STATIC", "Files", label_style, 320, 207, 250, 20, 0);
        let collections_list = control(parent, instance, "LISTBOX", "", list_style, 20, 230, 280, 265, COLLECTIONS);
        let files_list = control(parent, instance, "LISTBOX", "", list_style, 320, 230, 720, 265, FILES);
        control(parent, instance, "BUTTON", "Verify selected collection", button_style, 20, 507, 220, 28, VERIFY);
        control(parent, instance, "BUTTON", "Verified hex preview", button_style, 250, 507, 170, 28, PREVIEW);
        control(parent, instance, "STATIC", "File and collection details", label_style, 20, 548, 400, 20, 0);
        let details = control(parent, instance, "EDIT", "Select a collection to inspect its files.",
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
            20, 572, 1020, 130, 0);
        control(parent, instance, "STATIC", "Xenia Canary executable (inspection only)", label_style, 20, 715, 500, 20, 0);
        let backend_path = control(parent, instance, "EDIT", "", edit_style, 20, 738, 820, 25, 0);
        control(parent, instance, "BUTTON", "Inspect backend", button_style, 855, 738, 185, 27, BACKEND);
        let status = control(parent, instance, "STATIC", "Ready. Private files stay on this computer.", label_style, 20, 778, 1000, 24, 0);
        UI.with(|cell| {
            *cell.borrow_mut() = Some(Ui { hwnd, path, mode, source, label, version,
                collections_list, files_list, details, backend_path, status,
                inspector_panel: parent, monitor, watched_log, monitor_stop,
                store, collections: Vec::new(), busy: false });
            refresh(cell.borrow_mut().as_mut().unwrap());
        });
    }
    pub fn run() {
        unsafe {
            let store = Store::open().ok().map(Arc::new);
            let instance = GetModuleHandleW(null_mut());
            let class_name = wide("CpfXbox360NativeWindow");
            let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: instance,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW), hbrBackground: GetStockObject(WHITE_BRUSH),
                lpszClassName: class_name.as_ptr(), ..WNDCLASSW::default() };
            if RegisterClassW(&class) == 0 { return; }
            let panel_name = wide("CpfXbox360Panel");
            let panel_class = WNDCLASSW { lpfnWndProc: Some(panel_proc), hInstance: instance,
                hbrBackground: GetStockObject(WHITE_BRUSH), lpszClassName: panel_name.as_ptr(), ..WNDCLASSW::default() };
            if RegisterClassW(&panel_class) == 0 { return; }
            let hwnd = CreateWindowExW(0, class_name.as_ptr(), wide("Xbox 360 Monitor - CPF").as_ptr(),
                WS_OVERLAPPEDWINDOW, 100, 50, 1080, 940, null_mut(), null_mut(), instance, null_mut());
            if hwnd.is_null() { return; }
            control(hwnd, instance, "BUTTON", "Live monitor", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 20, 12, 160, 30, MONITOR_TAB);
            control(hwnd, instance, "BUTTON", "Firmware tools", WS_CHILD | WS_VISIBLE | WS_TABSTOP, 190, 12, 160, 30, FIRMWARE_TAB);
            let monitor_panel = control(hwnd, instance, "CpfXbox360Panel", "", WS_CHILD | WS_VISIBLE, 0, 50, 1060, 825, 0);
            let inspector_panel = control(hwnd, instance, "CpfXbox360Panel", "", WS_CHILD, 0, 50, 1060, 825, 0);
            let monitor = create_monitor_ui(monitor_panel, instance);
            let watched_log = Arc::new(Mutex::new(String::new()));
            let monitor_stop = Arc::new(AtomicBool::new(false));
            create_ui(inspector_panel, hwnd, store, instance, monitor, watched_log.clone(), monitor_stop.clone());
            let target = hwnd as usize;
            std::thread::spawn(move || {
                let mut monitor = Monitor::new();
                while !monitor_stop.load(Ordering::Relaxed) {
                    let path = watched_log.lock().unwrap().clone();
                    send_work(target, WorkResult::Monitor(monitor.poll(&path)));
                    for _ in 0..20 {
                        if monitor_stop.load(Ordering::Relaxed) { break; }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            });
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            let mut message = MSG::default();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

#[cfg(windows)]
fn main() { native::run(); }

#[cfg(not(windows))]
fn main() { eprintln!("Xbox.exe is a native Windows application."); }
