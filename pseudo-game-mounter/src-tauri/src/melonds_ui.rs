use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct EmulatorSession {
    pub pid: u32,
    pub title: String,
}

#[derive(Clone, Serialize)]
pub struct SlotObservation {
    pub inserted: bool,
    pub label: String,
}

#[cfg(windows)]
mod platform {
    use super::{EmulatorSession, SlotObservation};
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    use windows::core::BSTR;
    use windows::Win32::Foundation::HWND as AutomationHwnd;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationCondition, IUIAutomationElement,
        IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern, IUIAutomationValuePattern,
        TreeScope_Children, TreeScope_Descendants, UIA_ExpandCollapsePatternId,
        UIA_InvokePatternId, UIA_ValuePatternId,
    };


    struct MenuEntry {
        name: String,
        enabled: bool,
        element: IUIAutomationElement,
    }

    pub fn sessions() -> Vec<EmulatorSession> {
        unsafe extern "system" fn collect(hwnd: HWND, context: LPARAM) -> i32 {
            if IsWindowVisible(hwnd) == 0 { return 1; }
            let sessions = &mut *(context as *mut Vec<EmulatorSession>);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || sessions.iter().any(|session| session.pid == pid) { return 1; }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() { return 1; }
            let mut path = [0u16; 1024];
            let mut size = path.len() as u32;
            let matched = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut size) != 0
                && std::path::Path::new(&String::from_utf16_lossy(&path[..size as usize]))
                    .file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("melonDS.exe"));
            CloseHandle(process);
            if !matched { return 1; }
            let mut title = [0u16; 512];
            let length = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            if length <= 0 { return 1; }
            sessions.push(EmulatorSession { pid, title: String::from_utf16_lossy(&title[..length as usize]) });
            1
        }
        let mut result: Vec<EmulatorSession> = Vec::new();
        unsafe { EnumWindows(Some(collect), &mut result as *mut Vec<EmulatorSession> as LPARAM); }
        result.sort_by_key(|session| session.pid);
        result
    }

    fn entries(ui: &IUIAutomation, condition: &IUIAutomationCondition, pid: u32) -> Vec<MenuEntry> {
        let mut result = Vec::new();
        unsafe {
            let Ok(desktop) = ui.GetRootElement() else { return result };
            let Ok(windows) = desktop.FindAll(TreeScope_Children, condition) else { return result };
            let Ok(count) = windows.Length() else { return result };
            for window_index in 0..count {
                let Ok(window) = windows.GetElement(window_index) else { continue };
                if window.CurrentProcessId().ok() != Some(pid as i32) { continue; }
                let Ok(items) = window.FindAll(TreeScope_Descendants, condition) else { continue };
                let Ok(item_count) = items.Length() else { continue };
                for item_index in 0..item_count {
                    let Ok(element) = items.GetElement(item_index) else { continue };
                    let Ok(name) = element.CurrentName() else { continue };
                    if name.is_empty() { continue; }
                    let enabled = element.CurrentIsEnabled().is_ok_and(|value| value.as_bool());
                    result.push(MenuEntry { name: name.to_string(), enabled, element });
                }
            }
        }
        result
    }

    fn read_menu(
        ui: &IUIAutomation,
        condition: &IUIAutomationCondition,
        pid: u32,
    ) -> Result<(SlotObservation, Option<IUIAutomationElement>, Option<IUIAutomationExpandCollapsePattern>), String> {
        let mut items = entries(ui, condition, pid);
        let mut opened = None;
        if !items.iter().any(|item| item.name.starts_with("DS slot:")) {
            let file = items.iter().find(|item| item.name == "File")
                .ok_or("Could not find the melonDS File menu for this session")?;
            let menu: IUIAutomationExpandCollapsePattern = unsafe {
                file.element.GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
            }.map_err(|error| format!("Could not open the melonDS File menu: {error}"))?;
            unsafe { menu.Expand() }
                .map_err(|error| format!("Could not expand the melonDS File menu: {error}"))?;
            opened = Some(menu);
            for _ in 0..6 {
                thread::sleep(Duration::from_millis(180));
                items = entries(ui, condition, pid);
                if items.iter().any(|item| item.name.starts_with("DS slot:")) { break; }
            }
        }
        let slot_index = match items.iter().position(|item| item.name.starts_with("DS slot:")) {
            Some(index) => index,
            None => {
                if let Some(menu) = opened { unsafe { let _ = menu.Collapse(); } }
                return Err("melonDS did not expose its DS cartridge slot".into());
            }
        };
        let label = items[slot_index].name.trim_start_matches("DS slot:").trim().to_string();
        let eject = items.iter().skip(slot_index + 1)
            .take_while(|item| !item.name.starts_with("GBA slot:"))
            .find(|item| item.name == "Eject cart");
        let inserted = eject.is_some_and(|item| item.enabled);
        if !inserted && label != "(none)" {
            return Err(format!("melonDS reported an inconsistent cartridge state: {label}"));
        }
        Ok((SlotObservation { inserted, label }, eject.map(|item| item.element.clone()), opened))
    }

    fn read_menu_retry(
        ui: &IUIAutomation,
        condition: &IUIAutomationCondition,
        pid: u32,
    ) -> Result<(SlotObservation, Option<IUIAutomationElement>, Option<IUIAutomationExpandCollapsePattern>), String> {
        let mut last_error = String::new();
        for _ in 0..4 {
            match read_menu(ui, condition, pid) {
                Ok(value) => return Ok(value),
                Err(error) => last_error = error,
            }
            thread::sleep(Duration::from_millis(300));
        }
        Err(last_error)
    }

    fn picker(ui: &IUIAutomation, pid: u32) -> Option<IUIAutomationElement> {
        struct Target { pid: u32, hwnd: HWND }
        unsafe extern "system" fn find_window(hwnd: HWND, context: LPARAM) -> i32 {
            let target = &mut *(context as *mut Target);
            let mut process = 0;
            GetWindowThreadProcessId(hwnd, &mut process);
            if process == target.pid {
                let mut title = [0u16; 64];
                let mut class = [0u16; 64];
                let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
                let class_len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
                if title_len > 0 && class_len > 0
                    && String::from_utf16_lossy(&title[..title_len as usize]) == "Open DS ROM"
                    && String::from_utf16_lossy(&class[..class_len as usize]) == "#32770" {
                    target.hwnd = hwnd;
                    return 0;
                }
            }
            1
        }
        unsafe {
            let mut target = Target { pid, hwnd: std::ptr::null_mut() };
            EnumWindows(Some(find_window), &mut target as *mut Target as LPARAM);
            if target.hwnd.is_null() { return None; }
            ui.ElementFromHandle(AutomationHwnd(target.hwnd)).ok()
        }
    }

    fn post_menu_key(hwnd: HWND, key: usize) -> Result<(), String> {
        unsafe {
            if PostMessageW(hwnd, 0x0100, key, 0) == 0
                || PostMessageW(hwnd, 0x0101, key, 0) == 0 {
                return Err("Could not send a key to melonDS's File menu".into());
            }
        }
        Ok(())
    }

    fn open_insert_picker(ui: &IUIAutomation, condition: &IUIAutomationCondition, pid: u32) -> Result<(), String> {
        unsafe {
            let item = entries(ui, condition, pid).into_iter()
                .find(|item| item.name == "Insert cart..." && item.enabled)
                .ok_or("melonDS did not expose an enabled Insert cart action")?;
            let walker = ui.ControlViewWalker().map_err(|error| error.to_string())?;
            let parent = walker.GetParentElement(&item.element).map_err(|error| error.to_string())?;
            if parent.CurrentClassName().map_err(|error| error.to_string())?.to_string() != "QMenu" {
                return Err("melonDS Insert cart is not in the expected File menu".into());
            }
            let hwnd = parent.CurrentNativeWindowHandle().map_err(|error| error.to_string())?.0;
            let mut menu_pid = 0;
            GetWindowThreadProcessId(hwnd, &mut menu_pid);
            if menu_pid != pid { return Err("The Insert cart menu belongs to another process".into()); }
            for _ in 0..16 {
                post_menu_key(hwnd, 0x28)?; // Down in melonDS's open File menu.
                thread::sleep(Duration::from_millis(90));
                let focused = entries(ui, condition, pid).into_iter().any(|item| {
                    item.name == "Insert cart..." && item.enabled
                        && item.element.CurrentHasKeyboardFocus().is_ok_and(|value| value.as_bool())
                });
                if focused {
                    post_menu_key(hwnd, 0x0D)?; // Enter opens the native picker without blocking Qt accessibility.
                    return Ok(());
                }
            }
            Err("Could not select melonDS Insert cart from its File menu".into())
        }
    }

    fn act_insert(pid: u32, path: &Path) -> Result<SlotObservation, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()
                .map_err(|error| format!("Windows UI Automation is unavailable: {error}"))?;
            let result = (|| {
                let ui: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("Could not connect to Windows UI Automation: {error}"))?;
                let condition = ui.CreateTrueCondition().map_err(|error| error.to_string())?;
                let (before, _, menu) = read_menu_retry(&ui, &condition, pid)?;
                if before.inserted {
                    if let Some(menu) = menu { let _ = menu.Collapse(); }
                    return Err("Eject the current cartridge before inserting another one".into());
                }
                open_insert_picker(&ui, &condition, pid)?;
                let mut dialog = None;
                for _ in 0..70 {
                    dialog = picker(&ui, pid);
                    if dialog.is_some() { break; }
                    thread::sleep(Duration::from_millis(100));
                }
                if dialog.is_none() {
                    return Err("melonDS did not open its Insert cart picker within seven seconds".into());
                }
                let mut file_name = None;
                let mut open = None;
                let mut file_controls = Vec::new();
                let mut last_count = 0;
                for _ in 0..50 {
                    let dialog = picker(&ui, pid).ok_or("melonDS closed its cartridge picker before a file was selected")?;
                    let controls = dialog.FindAll(TreeScope_Descendants, &condition)
                        .map_err(|error| format!("Could not inspect melonDS's cartridge picker: {error}"))?;
                    last_count = controls.Length().map_err(|error| error.to_string())?;
                    file_controls.clear();
                    for index in 0..last_count {
                        let element = controls.GetElement(index).map_err(|error| error.to_string())?;
                        let id = element.CurrentAutomationId().ok().map(|value| value.to_string());
                        let class = element.CurrentClassName().ok().map(|value| value.to_string());
                        if id.as_deref() == Some("1148") && class.as_deref() == Some("Edit") {
                            file_name = Some(element);
                        } else if id.as_deref() == Some("1") && class.as_deref() == Some("Button")
                            && element.CurrentName().ok().is_some_and(|name| name.to_string() == "Open") {
                            open = Some(element);
                        }
                        if id.as_deref() == Some("1148") { file_controls.push(class.unwrap_or_default()); }
                    }
                    if file_name.is_some() && open.is_some() { break; }
                    thread::sleep(Duration::from_millis(100));
                }
                let file_name = file_name.ok_or_else(|| format!(
                    "melonDS's cartridge picker has no file-name field ({} controls; 1148 classes: {})",
                    last_count, file_controls.join(", ")
                ))?;
                let open = open.ok_or("melonDS's cartridge picker has no Open button")?;
                let value: IUIAutomationValuePattern = file_name.GetCurrentPatternAs(UIA_ValuePatternId)
                    .map_err(|error| format!("Could not enter the cartridge path: {error}"))?;
                value.SetValue(&BSTR::from(path.to_string_lossy().as_ref()))
                    .map_err(|error| format!("Could not enter the cartridge path: {error}"))?;
                let invoke: IUIAutomationInvokePattern = open.GetCurrentPatternAs(UIA_InvokePatternId)
                    .map_err(|error| format!("Could not activate the cartridge picker: {error}"))?;
                invoke.Invoke().map_err(|error| format!("melonDS's cartridge picker failed: {error}"))?;
                for _ in 0..50 {
                    if picker(&ui, pid).is_none() { break; }
                    thread::sleep(Duration::from_millis(100));
                }
                if picker(&ui, pid).is_some() {
                    return Err("melonDS kept its cartridge picker open; check its window for an error".into());
                }
                thread::sleep(Duration::from_millis(250));
                let (after, _, after_menu) = read_menu_retry(&ui, &condition, pid)?;
                if let Some(menu) = after_menu { let _ = menu.Collapse(); }
                if !after.inserted {
                    return Err("melonDS closed the picker but Slot-1 is still empty".into());
                }
                Ok(after)
            })();
            CoUninitialize();
            result
        }
    }

    fn act(pid: u32, eject: bool) -> Result<SlotObservation, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()
                .map_err(|error| format!("Windows UI Automation is unavailable: {error}"))?;
            let result = (|| {
                let ui: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("Could not connect to Windows UI Automation: {error}"))?;
                let condition = ui.CreateTrueCondition().map_err(|error| error.to_string())?;
                let (observed, action, menu) = read_menu_retry(&ui, &condition, pid)?;
                if !eject || !observed.inserted {
                    if let Some(menu) = menu { let _ = menu.Collapse(); }
                    return Ok(observed);
                }
                let action = action.ok_or("melonDS did not expose an enabled DS Eject cart action")?;
                let invoke: IUIAutomationInvokePattern = action.GetCurrentPatternAs(UIA_InvokePatternId)
                    .map_err(|error| format!("Could not access melonDS eject control: {error}"))?;
                invoke.Invoke().map_err(|error| format!("melonDS eject failed: {error}"))?;
                thread::sleep(Duration::from_millis(250));
                let (after, _, after_menu) = read_menu_retry(&ui, &condition, pid)
                    .map_err(|error| format!("Cartridge ejected, but Slot-1 verification failed: {error}"))?;
                if let Some(menu) = after_menu { let _ = menu.Collapse(); }
                if after.inserted {
                    return Err("melonDS still reports a cartridge in Slot-1 after Eject".into());
                }
                Ok(after)
            })();
            CoUninitialize();
            result
        }
    }

    pub fn check(pid: u32) -> Result<SlotObservation, String> {
        thread::spawn(move || act(pid, false)).join()
            .map_err(|_| "Cartridge status check stopped unexpectedly".to_string())?
    }

    pub fn eject(pid: u32) -> Result<SlotObservation, String> {
        thread::spawn(move || act(pid, true)).join()
            .map_err(|_| "Cartridge eject stopped unexpectedly".to_string())?
    }

    pub fn insert(pid: u32, path: &Path) -> Result<SlotObservation, String> {
        let path = path.to_path_buf();
        thread::spawn(move || act_insert(pid, &path)).join()
            .map_err(|_| "Cartridge insertion stopped unexpectedly".to_string())?
    }

}

#[cfg(windows)]
pub use platform::{check, eject, insert, sessions};

#[cfg(not(windows))]
pub fn sessions() -> Vec<EmulatorSession> { Vec::new() }

#[cfg(not(windows))]
pub fn check(_pid: u32) -> Result<SlotObservation, String> {
    Err("Live cartridge status requires Windows UI Automation".into())
}

#[cfg(not(windows))]
pub fn eject(_pid: u32) -> Result<SlotObservation, String> {
    Err("Live cartridge ejection requires Windows UI Automation".into())
}

#[cfg(not(windows))]
pub fn insert(_pid: u32, _path: &std::path::Path) -> Result<SlotObservation, String> {
    Err("Live cartridge insertion requires Windows UI Automation".into())
}

