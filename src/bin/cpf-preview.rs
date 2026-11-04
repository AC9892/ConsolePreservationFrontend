#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("The native CPF shell preview requires Windows.");
}

#[cfg(windows)]
mod native {
    use cpf::azahar::{visible_window_for_pid, AzaharButtonKeys};
    use cpf::media_request::MediaRequest;
    use cpf::preview::{load_shell, ShellImage, ShellModel};
    use std::{
        ffi::c_void,
        io::{self, Write},
        mem::{size_of, zeroed},
        ptr::{copy_nonoverlapping, null, null_mut},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex, OnceLock,
        },
        time::{Duration, Instant},
    };
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateRectRgn, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, ScreenToClient, SelectObject, SetWindowRgn, AC_SRC_ALPHA, AC_SRC_OVER,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER,
        OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, EnumWindows, GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsWindowVisible, LoadCursorW, LoadImageW, MessageBoxW,
        PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow,
        SetWindowLongPtrW, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage,
        UpdateLayeredWindow, GWLP_HWNDPARENT, GWL_EXSTYLE, GWL_STYLE, HTCAPTION, HTCLIENT,
        HTTRANSPARENT, ICON_SMALL, IDC_ARROW, IDOK, IMAGE_ICON, LR_SHARED, MB_ICONERROR,
        MB_ICONQUESTION, MB_OK, MB_OKCANCEL, MF_STRING, MSG, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOW, TPM_RETURNCMD, ULW_ALPHA,
        WINDOWPOS, WM_APP, WM_CLOSE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOVE, WM_NCHITTEST, WM_NCRBUTTONUP, WM_RBUTTONUP,
        WM_SETICON, WM_WINDOWPOSCHANGING, WNDCLASSW, WS_EX_APPWINDOW, WS_EX_LAYERED,
        WS_EX_TOOLWINDOW, WS_POPUP,
    };

    static APP: OnceLock<Mutex<App>> = OnceLock::new();
    const WM_FRAME_READY: u32 = WM_APP + 1;
    const ORIGINAL_WIDTH: i32 = 1080;
    const ORIGINAL_HEIGHT: i32 = 720;
    const ORIGINAL_ART_WIDTH: i32 = 720;
    const ORIGINAL_ART_HEIGHT: i32 = 480;
    #[derive(Clone)]
    struct Image {
        width: i32,
        height: i32,
        bgra: Vec<u8>,
    }
    #[derive(Clone, Copy)]
    enum CaseControl {
        Key(usize),
        Power,
    }
    #[derive(Clone, Copy)]
    struct DirectWindows {
        top: usize,
        bottom: usize,
    }
    struct App {
        shells: [Image; 2],
        selected: usize,
        live_window: Option<usize>,
        direct: Option<DirectWindows>,
        media_request: Option<MediaRequest>,
        frame: Option<Image>,
        touch_active: bool,
        pressed_control: Option<CaseControl>,
        pressed_at: Option<Instant>,
        button_keys: AzaharButtonKeys,
        perf_report: bool,
        perf_since: Instant,
        perf_frames: u32,
        perf_capture_ns: u128,
        perf_print_ns: u128,
        perf_draw_ns: u128,
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    struct WindowLookup {
        pid: u32,
        windows: Vec<(usize, i32, i32)>,
    }
    unsafe extern "system" fn find_process_windows(hwnd: HWND, parameter: LPARAM) -> BOOL {
        let lookup = &mut *(parameter as *mut WindowLookup);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == lookup.pid && IsWindowVisible(hwnd) != 0 {
            let mut rect: RECT = zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 {
                lookup.windows.push((
                    hwnd as usize,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ));
            }
        }
        1
    }

    unsafe fn wait_for_direct_windows(pid: u32) -> io::Result<(usize, DirectWindows)> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut lookup = WindowLookup {
                pid,
                windows: Vec::new(),
            };
            EnumWindows(
                Some(find_process_windows),
                (&mut lookup as *mut WindowLookup) as LPARAM,
            );
            if let Some(windows) = classify_direct_windows(&lookup.windows) {
                return Ok(windows);
            }
            if Instant::now() >= deadline {
                let sizes = lookup
                    .windows
                    .iter()
                    .map(|(_, w, h)| format!("{w}x{h}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(io::Error::other(format!(
                    "Azahar render windows could not be identified (visible window sizes: {sizes})"
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn classify_direct_windows(windows: &[(usize, i32, i32)]) -> Option<(usize, DirectWindows)> {
        let main = windows
            .iter()
            .filter(|(_, w, h)| *w > 800 && *h > 400)
            .max_by_key(|(_, w, h)| i64::from(*w) * i64::from(*h))?
            .0;
        let mut render = windows
            .iter()
            .filter(|(hwnd, w, h)| {
                *hwnd != main && *w >= 300 && *w <= 1000 && *h >= 200 && *h <= 800
            })
            .copied()
            .collect::<Vec<_>>();
        render.sort_by_key(|(_, w, h)| std::cmp::Reverse(i64::from(*w) * i64::from(*h)));
        let [first, second, ..] = render.as_slice() else {
            return None;
        };
        Some((
            main,
            DirectWindows {
                top: first.0,
                bottom: second.0,
            },
        ))
    }

    #[cfg(test)]
    mod window_tests {
        use super::classify_direct_windows;

        #[test]
        fn main_window_is_never_selected_as_a_screen() {
            let (main, direct) =
                classify_direct_windows(&[(3, 416, 279), (1, 900, 600), (2, 416, 519)]).unwrap();
            assert_eq!(main, 1);
            assert_eq!(direct.top, 2);
            assert_eq!(direct.bottom, 3);
        }
    }

    unsafe fn align_direct_at(left: i32, top: i32, direct: DirectWindows) {
        // The two native render windows cover only the dark screen openings.
        SetWindowPos(
            direct.top as HWND,
            null_mut(),
            left + 336,
            top + 102,
            408,
            244,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        SetWindowPos(
            direct.bottom as HWND,
            null_mut(),
            left + 339,
            top + 384,
            400,
            240,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }

    unsafe fn align_direct(hwnd: HWND, direct: DirectWindows) {
        let mut rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) != 0 {
            align_direct_at(rect.left, rect.top, direct);
        }
    }

    unsafe fn attach_direct(hwnd: HWND, main: usize, direct: DirectWindows) -> io::Result<()> {
        for render in [direct.top, direct.bottom] {
            let window = render as HWND;
            let style = GetWindowLongPtrW(window, GWL_STYLE);
            SetWindowLongPtrW(window, GWL_STYLE, style & !0x00CF0000);
            SetWindowLongPtrW(window, GWLP_HWNDPARENT, hwnd as isize);
            if SetWindowPos(
                window,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        align_direct(hwnd, direct);
        let region = CreateRectRgn(40, 0, 360, 240);
        if region.is_null() || SetWindowRgn(direct.bottom as HWND, region, 1) == 0 {
            if !region.is_null() {
                DeleteObject(region);
            }
            return Err(io::Error::last_os_error());
        }
        let main = main as HWND;
        let style = GetWindowLongPtrW(main, GWL_EXSTYLE);
        SetWindowLongPtrW(
            main,
            GWL_EXSTYLE,
            (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
        );
        SetWindowPos(
            main,
            null_mut(),
            -5000,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        Ok(())
    }

    // Hit areas were measured at 720x480 and follow the displayed artwork.
    fn case_control_at(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        keys: AzaharButtonKeys,
    ) -> Option<CaseControl> {
        let x = x * ORIGINAL_ART_WIDTH / width;
        let y = y * ORIGINAL_ART_HEIGHT / height;
        let circle =
            |cx: i32, cy: i32, radius: i32| (x - cx).pow(2) + (y - cy).pow(2) <= radius.pow(2);
        if circle(572, 148, 13) {
            return Some(CaseControl::Key(keys.0[2] as usize));
        } // X
        if circle(541, 179, 13) {
            return Some(CaseControl::Key(keys.0[3] as usize));
        } // Y
        if circle(599, 179, 13) {
            return Some(CaseControl::Key(keys.0[0] as usize));
        } // A
        if circle(569, 205, 13) {
            return Some(CaseControl::Key(keys.0[1] as usize));
        } // B
        if circle(547, 278, 12) {
            return Some(CaseControl::Key(keys.0[8] as usize));
        } // Start
        if circle(547, 309, 12) {
            return Some(CaseControl::Key(keys.0[9] as usize));
        } // Select
        if circle(360, 444, 19) {
            return Some(CaseControl::Key(keys.0[10] as usize));
        } // Home
        if circle(546, 411, 13) {
            return Some(CaseControl::Power);
        }
        if (136..=161).contains(&x) && (230..=259).contains(&y) {
            return Some(CaseControl::Key(keys.0[4] as usize));
        } // Up
        if (136..=161).contains(&x) && (276..=303).contains(&y) {
            return Some(CaseControl::Key(keys.0[5] as usize));
        } // Down
        if (112..=141).contains(&x) && (252..=279).contains(&y) {
            return Some(CaseControl::Key(keys.0[6] as usize));
        } // Left
        if (158..=185).contains(&x) && (252..=279).contains(&y) {
            return Some(CaseControl::Key(keys.0[7] as usize));
        } // Right
        None
    }

    unsafe fn control_down(hwnd: HWND, x: i32, y: i32) -> bool {
        let Some(state) = APP.get() else {
            return false;
        };
        let Ok(mut app) = state.lock() else {
            return false;
        };
        let shell = &app.shells[app.selected];
        let Some(control) = case_control_at(x, y, shell.width, shell.height, app.button_keys)
        else {
            return false;
        };
        let Some(target) = app.live_window else {
            return false;
        };
        app.pressed_control = Some(control);
        app.pressed_at = Some(Instant::now());
        SetCapture(hwnd);
        if let CaseControl::Key(code) = control {
            PostMessageW(target as HWND, WM_KEYDOWN, code, 0);
        }
        true
    }

    unsafe fn control_up() -> bool {
        let Some(state) = APP.get() else {
            return false;
        };
        let Ok(mut app) = state.lock() else {
            return false;
        };
        let Some(control) = app.pressed_control.take() else {
            return false;
        };
        let pressed_at = app.pressed_at.take();
        ReleaseCapture();
        if let Some(target) = app.live_window {
            if let CaseControl::Key(code) = control {
                // Azahar polls guest input; a synthetic click can
                // press and release between polls. Keep this one key down
                // long enough for the guest input sampler to observe it.
                if let Some(started) = pressed_at {
                    let minimum = std::time::Duration::from_millis(80);
                    if let Some(remaining) = minimum.checked_sub(started.elapsed()) {
                        std::thread::sleep(remaining);
                    }
                }
                PostMessageW(target as HWND, WM_KEYUP, code, 0);
            }
        }
        matches!(control, CaseControl::Power)
    }

    fn resize(image: ShellImage, width: i32, height: i32) -> Image {
        let mut bgra = vec![0; width as usize * height as usize * 4];
        for y in 0..height as usize {
            let source_y = ((y as f32 + 0.5) * image.height as f32 / height as f32 - 0.5)
                .clamp(0.0, (image.height - 1) as f32);
            let top = source_y.floor() as usize;
            let bottom = (top + 1).min(image.height as usize - 1);
            let fy = source_y - top as f32;
            for x in 0..width as usize {
                let source_x = ((x as f32 + 0.5) * image.width as f32 / width as f32 - 0.5)
                    .clamp(0.0, (image.width - 1) as f32);
                let left = source_x.floor() as usize;
                let right = (left + 1).min(image.width as usize - 1);
                let fx = source_x - left as f32;
                let row = image.width as usize;
                let dst = (y * width as usize + x) * 4;
                for channel in 0..4 {
                    let sample =
                        |sx: usize, sy: usize| image.bgra[(sy * row + sx) * 4 + channel] as f32;
                    let top_value = sample(left, top) * (1.0 - fx) + sample(right, top) * fx;
                    let bottom_value =
                        sample(left, bottom) * (1.0 - fx) + sample(right, bottom) * fx;
                    bgra[dst + channel] =
                        (top_value * (1.0 - fy) + bottom_value * fy).round() as u8;
                }
            }
        }
        Image {
            width,
            height,
            bgra,
        }
    }

    // Azahar's current windowed layout places 400x240 and 320x240 screens
    // centered in its content area. Reject other layouts instead of silently
    // showing toolbar pixels in the shell.
    unsafe fn capture_screens(hwnd: HWND, shell: &Image) -> io::Result<(Image, u128)> {
        let mut rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(io::Error::last_os_error());
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width < 400 || height < 544 || width > 4096 || height > 2160 {
            return Err(io::Error::other("Azahar window layout is unsupported"));
        }
        let screen_dc = GetDC(null_mut());
        if screen_dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            ReleaseDC(null_mut(), screen_dc);
            return Err(io::Error::last_os_error());
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if bitmap.is_null() || pixels.is_null() {
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            return Err(io::Error::last_os_error());
        }
        let old = SelectObject(memory_dc, bitmap);
        let print_started = Instant::now();
        let ok = PrintWindow(hwnd, memory_dc, 0);
        let print_ns = print_started.elapsed().as_nanos();
        let mut output = Image {
            width: shell.width,
            height: shell.height,
            bgra: shell.bgra.clone(),
        };
        if ok != 0 {
            let source = std::slice::from_raw_parts(
                pixels.cast::<u8>(),
                width as usize * height as usize * 4,
            );
            for (source_rect, hole) in [
                (
                    ((width - 400) / 2, 64, 400, 240),
                    ShellModel::Original2ds.screens()[0],
                ),
                (
                    ((width - 320) / 2, 304, 320, 240),
                    ShellModel::Original2ds.screens()[1],
                ),
            ] {
                blit_screen(source, width, source_rect, &mut output, hole);
            }
        }
        SelectObject(memory_dc, old);
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        if ok == 0 {
            Err(io::Error::other("Azahar window capture failed"))
        } else {
            Ok((output, print_ns))
        }
    }

    fn blit_screen(
        source: &[u8],
        source_width: i32,
        area: (i32, i32, i32, i32),
        output: &mut Image,
        hole: (u32, u32, u32, u32),
    ) {
        let (source_x, source_y, sw, sh) = area;
        let (hx, hy, hw, hh) = hole;
        let scale_x = output.width as f32 / ShellModel::Original2ds.source_size().0 as f32;
        let scale_y = output.height as f32 / ShellModel::Original2ds.source_size().1 as f32;
        let hole_x = (hx as f32 * scale_x).round() as i32;
        let hole_y = (hy as f32 * scale_y).round() as i32;
        let hole_w = (hw as f32 * scale_x).round() as i32;
        let hole_h = (hh as f32 * scale_y).round() as i32;
        let scale = (hole_w as f32 / sw as f32).min(hole_h as f32 / sh as f32);
        let dw = (sw as f32 * scale).floor() as i32;
        let dh = (sh as f32 * scale).floor() as i32;
        let dx = hole_x + (hole_w - dw) / 2;
        let dy = hole_y + (hole_h - dh) / 2;
        for y in 0..dh {
            for x in 0..dw {
                let sx = source_x + x * sw / dw;
                let sy = source_y + y * sh / dh;
                let from = ((sy * source_width + sx) * 4) as usize;
                let to = (((dy + y) * output.width + dx + x) * 4) as usize;
                output.bgra[to..to + 3].copy_from_slice(&source[from..from + 3]);
                output.bgra[to + 3] = 255;
            }
        }
    }

    unsafe fn forward_touch(hwnd: HWND, msg: u32, lparam: LPARAM) {
        let Some(state) = APP.get() else {
            return;
        };
        let Ok(mut app) = state.lock() else {
            return;
        };
        if app.direct.is_some() {
            return;
        }
        let Some(target) = app.live_window.map(|h| h as HWND) else {
            return;
        };
        let shell = &app.shells[0];
        let (hx, hy, hw, hh) = ShellModel::Original2ds.screens()[1];
        let (sw, sh) = ShellModel::Original2ds.source_size();
        let hole_x = (hx as f32 * shell.width as f32 / sw as f32).round() as i32;
        let hole_y = (hy as f32 * shell.height as f32 / sh as f32).round() as i32;
        let hole_w = (hw as f32 * shell.width as f32 / sw as f32).round() as i32;
        let hole_h = (hh as f32 * shell.height as f32 / sh as f32).round() as i32;
        let scale = (hole_w as f32 / 320.0).min(hole_h as f32 / 240.0);
        let dw = (320.0 * scale).floor() as i32;
        let dh = (240.0 * scale).floor() as i32;
        let dx = hole_x + (hole_w - dw) / 2;
        let dy = hole_y + (hole_h - dh) / 2;
        let x = (lparam as i16) as i32;
        let y = ((lparam >> 16) as i16) as i32;
        if msg == WM_LBUTTONDOWN {
            if x < dx || x >= dx + dw || y < dy || y >= dy + dh {
                return;
            }
            app.touch_active = true;
            SetCapture(hwnd);
        } else if !app.touch_active {
            return;
        }
        if msg == WM_LBUTTONUP {
            app.touch_active = false;
            ReleaseCapture();
        }
        let mut rect: RECT = zeroed();
        if GetWindowRect(target, &mut rect) == 0 {
            return;
        }
        let tx = ((x - dx).clamp(0, dw - 1) * 320 / dw) + (rect.right - rect.left - 320) / 2;
        let ty = ((y - dy).clamp(0, dh - 1) * 240 / dh) + 304;
        let mut point = POINT {
            x: rect.left + tx,
            y: rect.top + ty,
        };
        if ScreenToClient(target, &mut point) == 0 {
            return;
        }
        let packed = (((point.y as u16 as u32) << 16) | point.x as u16 as u32) as LPARAM;
        PostMessageW(target, msg, if msg == WM_LBUTTONUP { 0 } else { 1 }, packed);
    }

    unsafe fn draw(hwnd: HWND) -> io::Result<()> {
        let app = APP
            .get()
            .unwrap()
            .lock()
            .map_err(|_| io::Error::other("preview state poisoned"))?;
        let image = app.frame.as_ref().unwrap_or(&app.shells[app.selected]);
        let mut rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(io::Error::last_os_error());
        }
        let screen_dc = GetDC(null_mut());
        if screen_dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_null() {
            ReleaseDC(null_mut(), screen_dc);
            return Err(io::Error::last_os_error());
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = image.width;
        info.bmiHeader.biHeight = -image.height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if bitmap.is_null() || pixels.is_null() {
            DeleteDC(memory_dc);
            ReleaseDC(null_mut(), screen_dc);
            return Err(io::Error::last_os_error());
        }
        copy_nonoverlapping(image.bgra.as_ptr(), pixels.cast(), image.bgra.len());
        let old = SelectObject(memory_dc, bitmap);
        let destination = POINT {
            x: rect.left,
            y: rect.top,
        };
        let size = SIZE {
            cx: image.width,
            cy: image.height,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = UpdateLayeredWindow(
            hwnd,
            screen_dc,
            &destination,
            &size,
            memory_dc,
            &source,
            0,
            &blend,
            ULW_ALPHA,
        );
        let error = if ok == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        SelectObject(memory_dc, old);
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(null_mut(), screen_dc);
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    unsafe fn select(hwnd: HWND, selected: usize) {
        if let Some(app) = APP.get() {
            if let Ok(mut app) = app.lock() {
                if app.live_window.is_some() {
                    return;
                }
                if app.selected == selected {
                    return;
                }
                app.selected = selected;
                let image = &app.shells[selected];
                let mut rect: RECT = zeroed();
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    let x = rect.left + (rect.right - rect.left - image.width) / 2;
                    let y = rect.top + (rect.bottom - rect.top - image.height) / 2;
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        x,
                        y,
                        image.width,
                        image.height,
                        SWP_NOZORDER,
                    );
                }
            }
        }
        if let Err(e) = draw(hwnd) {
            eprintln!("Unable to draw shell: {e}");
        }
    }

    unsafe fn menu(hwnd: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let live = APP
            .get()
            .and_then(|state| state.lock().ok())
            .map(|state| state.live_window.is_some())
            .unwrap_or(false);
        let items: &[(&str, usize)] = if live {
            &[
                ("Insert cartridge…", 4),
                ("Eject cartridge", 5),
                ("Install CIA to SD…", 6),
                ("Close  (Esc)", 3),
            ]
        } else {
            &[
                ("Original 2DS  (1)", 1),
                ("New 2DS XL  (2)", 2),
                ("Close  (Esc)", 3),
            ]
        };
        for &(label, id) in items {
            AppendMenuW(menu, MF_STRING, id, wide(label).as_ptr());
        }
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(menu, TPM_RETURNCMD, point.x, point.y, 0, hwnd, null());
        DestroyMenu(menu);
        match choice {
            1 => select(hwnd, 0),
            2 => select(hwnd, 1),
            3 => {
                DestroyWindow(hwnd);
            }
            4 | 5 | 6 => request_media_change(hwnd, choice),
            _ => {}
        }
    }

    unsafe fn pick_media_file(hwnd: HWND, install: bool) -> Option<std::path::PathBuf> {
        let filter = if install {
            "3DS installation package (*.cia)\0*.cia\0\0"
        } else {
            "3DS cartridge (*.cci;*.3ds)\0*.cci;*.3ds\0\0"
        };
        let filter: Vec<u16> = filter.encode_utf16().collect();
        let title = wide(if install {
            "Install CIA to SD"
        } else {
            "Insert cartridge"
        });
        let mut path = vec![0u16; 32768];
        let mut dialog: OPENFILENAMEW = zeroed();
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = path.as_mut_ptr();
        dialog.nMaxFile = path.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_DONTADDTORECENT;
        if GetOpenFileNameW(&mut dialog) == 0 {
            if CommDlgExtendedError() != 0 {
                MessageBoxW(
                    hwnd,
                    wide("Unable to open the file picker.").as_ptr(),
                    wide("CPF 2DS").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            return None;
        }
        let length = path.iter().position(|value| *value == 0)?;
        use std::os::windows::ffi::OsStringExt;
        Some(std::ffi::OsString::from_wide(&path[..length]).into())
    }

    unsafe fn request_media_change(hwnd: HWND, choice: i32) {
        let request = match choice {
            4 => {
                let Some(path) = pick_media_file(hwnd, false) else {
                    return;
                };
                if let Err(error) = cpf::cartridge::CartridgeImage::open(&path) {
                    use cpf::cartridge::CartridgeError;
                    let message = match error {
                        CartridgeError::EncryptedContent => "This cartridge is encrypted. The current build requires a decrypted image.",
                        CartridgeError::SourceUnavailable => "Unable to read the selected cartridge.",
                        _ => "Choose a supported .cci or .3ds cartridge image with valid content.",
                    };
                    MessageBoxW(
                        hwnd,
                        wide(message).as_ptr(),
                        wide("CPF 2DS").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                    return;
                }
                MediaRequest::InsertCartridge(path)
            }
            5 => MediaRequest::EjectCartridge,
            6 => {
                let Some(path) = pick_media_file(hwnd, true) else {
                    return;
                };
                MediaRequest::InstallSd(path)
            }
            _ => return,
        };
        let prompt = match request {
            MediaRequest::InsertCartridge(_) => "Restart the 2DS with this cartridge? Save your progress before continuing.",
            MediaRequest::EjectCartridge => "Restart the 2DS with the cartridge removed? Save your progress before continuing.",
            MediaRequest::InstallSd(_) => "Close this session, install the CIA, and reopen HOME? Save your progress before continuing.",
        };
        if MessageBoxW(
            hwnd,
            wide(prompt).as_ptr(),
            wide("CPF 2DS").as_ptr(),
            MB_OKCANCEL | MB_ICONQUESTION,
        ) != IDOK
        {
            return;
        }
        if let Some(state) = APP.get() {
            if let Ok(mut app) = state.lock() {
                app.media_request = Some(request);
            } else {
                return;
            }
        } else {
            return;
        }
        DestroyWindow(hwnd);
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_NCHITTEST => {
                let px = (lparam as i16) as i32;
                let py = ((lparam >> 16) as i16) as i32;
                let mut rect: RECT = zeroed();
                if GetWindowRect(hwnd, &mut rect) == 0 {
                    return HTTRANSPARENT as LRESULT;
                }
                let (x, y) = (px - rect.left, py - rect.top);
                if let Some(app) = APP.get() {
                    if let Ok(app) = app.lock() {
                        let image = &app.shells[app.selected];
                        if x < 0 || y < 0 || x >= image.width || y >= image.height {
                            return HTTRANSPARENT as LRESULT;
                        }
                        if image.bgra[((y * image.width + x) * 4 + 3) as usize] < 16 {
                            return HTTRANSPARENT as LRESULT;
                        }
                        if app.live_window.is_some()
                            && case_control_at(x, y, image.width, image.height, app.button_keys)
                                .is_some()
                        {
                            return HTCLIENT as LRESULT;
                        }
                        let model = if app.selected == 0 {
                            ShellModel::Original2ds
                        } else {
                            ShellModel::New2dsXl
                        };
                        let (sw, sh) = model.source_size();
                        let screens = model.screens();
                        let (sx, sy) = (
                            x as u32 * sw / image.width as u32,
                            y as u32 * sh / image.height as u32,
                        );
                        if screens
                            .iter()
                            .any(|&(l, t, w, h)| sx >= l && sx < l + w && sy >= t && sy < t + h)
                        {
                            return HTCLIENT as LRESULT;
                        }
                        return HTCAPTION as LRESULT;
                    }
                }
                HTTRANSPARENT as LRESULT
            }
            WM_KEYDOWN => {
                if wparam == 0x1b {
                    DestroyWindow(hwnd);
                } else if let Some(target) = APP
                    .get()
                    .and_then(|app| app.lock().ok())
                    .and_then(|app| app.live_window)
                {
                    PostMessageW(target as HWND, msg, wparam, lparam);
                } else {
                    match wparam {
                        0x31 => select(hwnd, 0),
                        0x32 => select(hwnd, 1),
                        _ => {}
                    }
                }
                0
            }
            WM_KEYUP => {
                if let Some(target) = APP
                    .get()
                    .and_then(|app| app.lock().ok())
                    .and_then(|app| app.live_window)
                {
                    PostMessageW(target as HWND, msg, wparam, lparam);
                }
                0
            }
            WM_FRAME_READY => {
                let draw_started = Instant::now();
                if let Err(e) = draw(hwnd) {
                    eprintln!("Unable to draw live shell: {e}");
                    DestroyWindow(hwnd);
                }
                if let Some(app) = APP.get() {
                    if let Ok(mut app) = app.lock() {
                        if app.perf_report {
                            app.perf_draw_ns += draw_started.elapsed().as_nanos();
                            app.perf_frames += 1;
                            let elapsed = app.perf_since.elapsed();
                            if elapsed.as_secs_f64() >= 2.0 {
                                eprintln!("CPF shell frames per second: {:.1}; capture: {:.1} ms; print: {:.1} ms; draw: {:.1} ms", app.perf_frames as f64 / elapsed.as_secs_f64(),
                                    app.perf_capture_ns as f64 / app.perf_frames as f64 / 1_000_000.0,
                                    app.perf_print_ns as f64 / app.perf_frames as f64 / 1_000_000.0,
                                    app.perf_draw_ns as f64 / app.perf_frames as f64 / 1_000_000.0);
                                app.perf_since = Instant::now();
                                app.perf_frames = 0;
                                app.perf_capture_ns = 0;
                                app.perf_print_ns = 0;
                                app.perf_draw_ns = 0;
                            }
                        }
                    }
                }
                0
            }
            WM_LBUTTONDOWN => {
                let x = (lparam as i16) as i32;
                let y = ((lparam >> 16) as i16) as i32;
                if !control_down(hwnd, x, y) {
                    forward_touch(hwnd, msg, lparam);
                }
                0
            }
            WM_LBUTTONUP => {
                let pressed = APP
                    .get()
                    .and_then(|app| app.lock().ok())
                    .map(|app| app.pressed_control.is_some())
                    .unwrap_or(false);
                if pressed {
                    if control_up() {
                        DestroyWindow(hwnd);
                    }
                } else {
                    forward_touch(hwnd, msg, lparam);
                }
                0
            }
            WM_MOUSEMOVE => {
                forward_touch(hwnd, msg, lparam);
                0
            }
            WM_WINDOWPOSCHANGING => {
                if let Some(direct) = APP
                    .get()
                    .and_then(|app| app.lock().ok())
                    .and_then(|app| app.direct)
                {
                    let proposed = &*(lparam as *const WINDOWPOS);
                    if proposed.flags & SWP_NOMOVE == 0 {
                        align_direct_at(proposed.x, proposed.y, direct);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_MOVE => {
                if let Some(direct) = APP
                    .get()
                    .and_then(|app| app.lock().ok())
                    .and_then(|app| app.direct)
                {
                    align_direct(hwnd, direct);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_RBUTTONUP | WM_NCRBUTTONUP => {
                menu(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    pub fn run() -> io::Result<()> {
        let mut args = std::env::args();
        args.next();
        let (live_window, direct_pid, button_keys) = match (
            args.next().as_deref(),
            args.next(),
            args.next(),
            args.next(),
            args.next(),
        ) {
            (None, None, None, None, None) => (None, None, AzaharButtonKeys::DEFAULT),
            (Some("--attach-pid"), Some(pid), Some(flag), Some(keys), None)
                if flag == "--buttons" =>
            {
                let pid = pid
                    .parse::<u32>()
                    .map_err(|_| io::Error::other("invalid Azahar PID"))?;
                let keys = AzaharButtonKeys::decode(&keys)
                    .ok_or_else(|| io::Error::other("invalid button keys"))?;
                (
                    Some(
                        visible_window_for_pid(pid)
                            .ok_or_else(|| io::Error::other("Azahar window unavailable"))?
                            as usize,
                    ),
                    None,
                    keys,
                )
            }
            (Some("--direct-pid"), Some(pid), Some(flag), Some(keys), None)
                if flag == "--buttons" =>
            {
                let pid = pid
                    .parse::<u32>()
                    .map_err(|_| io::Error::other("invalid Azahar PID"))?;
                let keys = AzaharButtonKeys::decode(&keys)
                    .ok_or_else(|| io::Error::other("invalid button keys"))?;
                (None, Some(pid), keys)
            }
            _ => {
                return Err(io::Error::other(
                    "usage: cpf-preview [--direct-pid|--attach-pid <pid> --buttons <keys>]",
                ))
            }
        };
        let original = resize(
            load_shell(ShellModel::Original2ds).map_err(io::Error::other)?,
            ORIGINAL_WIDTH,
            ORIGINAL_HEIGHT,
        );
        let new = resize(
            load_shell(ShellModel::New2dsXl).map_err(io::Error::other)?,
            855,
            855,
        );
        APP.set(Mutex::new(App {
            shells: [original, new],
            selected: 0,
            live_window,
            direct: None,
            media_request: None,
            frame: None,
            touch_active: false,
            pressed_control: None,
            pressed_at: None,
            button_keys,
            perf_report: std::env::var_os("CPF_PREVIEW_PERF").is_some(),
            perf_since: Instant::now(),
            perf_frames: 0,
            perf_capture_ns: 0,
            perf_print_ns: 0,
            perf_draw_ns: 0,
        }))
        .map_err(|_| io::Error::other("preview already initialized"))?;
        unsafe {
            let instance = GetModuleHandleW(null());
            if instance.is_null() {
                return Err(io::Error::last_os_error());
            }
            let class = wide("CPFNativeShellPreview");
            let title = wide("CPF - 2DS family preview");
            let icon = 1usize as *const u16;
            let large = LoadImageW(instance, icon, IMAGE_ICON, 32, 32, LR_SHARED);
            let small = LoadImageW(instance, icon, IMAGE_ICON, 16, 16, LR_SHARED);
            if large.is_null() || small.is_null() {
                return Err(io::Error::last_os_error());
            }
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: large,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return Err(io::Error::last_os_error());
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_APPWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                100,
                70,
                ORIGINAL_WIDTH,
                ORIGINAL_HEIGHT,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                return Err(io::Error::last_os_error());
            }
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
            draw(hwnd)?;
            ShowWindow(hwnd, SW_SHOW);
            if let Some(pid) = direct_pid {
                let (main, direct) = wait_for_direct_windows(pid)?;
                attach_direct(hwnd, main, direct)?;
                if let Ok(mut app) = APP.get().unwrap().lock() {
                    app.live_window = Some(direct.top);
                    app.direct = Some(direct);
                }
            }
            let running = Arc::new(AtomicBool::new(true));
            let worker = live_window.map(|target| {
                let shell = APP.get().unwrap().lock().unwrap().shells[0].clone();
                let running = Arc::clone(&running);
                let destination = hwnd as usize;
                std::thread::spawn(move || {
                    while running.load(Ordering::Acquire) {
                        let started = Instant::now();
                        match capture_screens(target as HWND, &shell) {
                            Ok((frame, print_ns)) => {
                                if let Some(state) = APP.get() {
                                    if let Ok(mut app) = state.lock() {
                                        app.frame = Some(frame);
                                        if app.perf_report {
                                            app.perf_capture_ns += started.elapsed().as_nanos();
                                            app.perf_print_ns += print_ns;
                                        }
                                    }
                                }
                                if PostMessageW(destination as HWND, WM_FRAME_READY, 0, 0) == 0 {
                                    break;
                                }
                            }
                            Err(_) => {
                                PostMessageW(destination as HWND, WM_CLOSE, 0, 0);
                                break;
                            }
                        }
                        let remaining = Duration::from_millis(16).saturating_sub(started.elapsed());
                        if !remaining.is_zero() {
                            std::thread::sleep(remaining);
                        }
                    }
                })
            });
            let mut message: MSG = zeroed();
            let mut loop_error = None;
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result == 0 {
                    break;
                }
                if result == -1 {
                    loop_error = Some(io::Error::last_os_error());
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            running.store(false, Ordering::Release);
            if let Some(worker) = worker {
                let _ = worker.join();
            }
            if let Some(error) = loop_error {
                return Err(error);
            }
        }
        if let Some(request) = APP
            .get()
            .and_then(|state| state.lock().ok())
            .and_then(|mut app| app.media_request.take())
        {
            let text = request
                .encode()
                .ok_or_else(|| io::Error::other("media request could not be encoded"))?;
            let mut output = io::stdout().lock();
            output.write_all(text.as_bytes())?;
            output.flush()?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn controls_follow_the_displayed_shell_size() {
            let keys = AzaharButtonKeys::DEFAULT;
            assert!(matches!(
                case_control_at(171, 265, 720, 480, keys),
                Some(CaseControl::Key(0x48))
            ));
            assert!(matches!(
                case_control_at(256, 398, 1080, 720, keys),
                Some(CaseControl::Key(0x48))
            ));
            assert!(matches!(
                case_control_at(899, 269, 1080, 720, keys),
                Some(CaseControl::Key(0x41))
            ));
            assert!(matches!(
                case_control_at(819, 616, 1080, 720, keys),
                Some(CaseControl::Power)
            ));
            assert!(matches!(
                case_control_at(540, 666, 1080, 720, keys),
                Some(CaseControl::Key(0x42))
            ));
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(e) = native::run() {
        eprintln!("Unable to start native CPF preview: {e}");
        std::process::exit(1);
    }
}
