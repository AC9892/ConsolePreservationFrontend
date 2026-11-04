//! Small Windows status window for operations which run outside the HOME session.
#[cfg(windows)]
pub fn run<T: Send>(operation: impl FnOnce(Status) -> T + Send) -> std::io::Result<T> {
    use std::{io, mem::zeroed, ptr::null_mut, sync::mpsc, time::Duration};
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS},
        Graphics::Gdi::{GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn procedure(
        window: windows_sys::Win32::Foundation::HWND,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        // Installation cannot safely be cancelled during extraction. Keep the
        // message loop responsive, but do not imply that closing cancels it.
        if message == WM_CLOSE {
            return 0;
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("CPFOperationStatus");
        let mut class: WNDCLASSW = zeroed();
        class.lpfnWndProc = Some(procedure);
        class.hInstance = instance;
        class.lpszClassName = class_name.as_ptr();
        class.hbrBackground = (COLOR_WINDOW + 1) as _;
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(io::Error::last_os_error());
        }
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("CPF 2DS — Installing to SD").as_ptr(),
            WS_CAPTION | WS_BORDER,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            150,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        );
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        let label = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("Checking CIA…").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            18,
            430,
            24,
            window,
            null_mut(),
            instance,
            null_mut(),
        );
        let note = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("Please wait. HOME will reopen when this finishes.").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            55,
            430,
            35,
            window,
            null_mut(),
            instance,
            null_mut(),
        );
        if label.is_null() || note.is_null() {
            DestroyWindow(window);
            return Err(io::Error::other("unable to create operation status text"));
        }
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(label, WM_SETFONT, font as usize, 1);
        SendMessageW(note, WM_SETFONT, font as usize, 1);
        ShowWindow(window, SW_SHOW);
        let (sender, receiver) = mpsc::channel();
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(move || operation(Status(sender)));
            while !worker.is_finished() {
                let mut message: MSG = zeroed();
                while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                for status in receiver.try_iter() {
                    SetWindowTextW(label, wide(status).as_ptr());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            worker
                .join()
                .map_err(|_| io::Error::other("media operation worker failed"))
        });
        DestroyWindow(window);
        result
    }
}

/// Only fixed, non-sensitive descriptions are sent to the status window.
pub struct Status(std::sync::mpsc::Sender<&'static str>);

impl Status {
    pub fn set(&self, description: &'static str) {
        let _ = self.0.send(description);
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn status_window_returns_worker_result_and_handles_worker_failure() {
        assert_eq!(
            super::run(|status| {
                status.set("Verifying installed files…");
                std::thread::sleep(std::time::Duration::from_millis(50));
                42
            })
            .unwrap(),
            42
        );
        assert!(super::run(|_| -> () { panic!("worker test failure") }).is_err());
    }
}
