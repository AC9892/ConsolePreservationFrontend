use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
mod loader {
    use std::ffi::{c_char, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }
    pub struct Library(*mut c_void);
    impl Library {
        pub unsafe fn new(path: &Path) -> Result<Self, String> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let handle = LoadLibraryW(wide.as_ptr());
            if handle.is_null() {
                Err(format!(
                    "Could not load {}: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(Self(handle))
            }
        }
        pub unsafe fn get<T: Copy>(&self, name: &[u8]) -> Result<T, String> {
            let mut bytes = name.to_vec();
            bytes.push(0);
            let address = GetProcAddress(self.0, bytes.as_ptr() as *const c_char);
            if address.is_null() {
                Err(format!(
                    "Missing libretro symbol: {}",
                    String::from_utf8_lossy(name)
                ))
            } else {
                Ok(std::mem::transmute_copy(&address))
            }
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
}
#[cfg(windows)]
use loader::Library;

#[repr(C)]
struct Geometry {
    base_width: u32,
    base_height: u32,
    max_width: u32,
    max_height: u32,
    aspect_ratio: f32,
}
#[repr(C)]
struct Timing {
    fps: f64,
    sample_rate: f64,
}
#[repr(C)]
struct AvInfo {
    geometry: Geometry,
    timing: Timing,
}
#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}
#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}

// libretro RETRO_ENVIRONMENT_GET_DEVICE_POWER uses this C layout.
#[repr(C)]
struct DevicePower {
    state: i32,
    seconds: i32,
    percent: i8,
}

const GET_DEVICE_POWER: u32 = 77;
const POWERSTATE_CHARGED: i32 = 3;

type Env = unsafe extern "C" fn(u32, *mut c_void) -> bool;
type Video = unsafe extern "C" fn(*const c_void, u32, u32, usize);
type Audio = unsafe extern "C" fn(i16, i16);
type AudioBatch = unsafe extern "C" fn(*const i16, usize) -> usize;
type Poll = unsafe extern "C" fn();
type Input = unsafe extern "C" fn(u32, u32, u32, u32) -> i16;

#[derive(Default)]
pub struct Frame {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pitch: usize,
}
struct Callbacks {
    paths: HashMap<u32, CString>,
    options: HashMap<String, CString>,
    buttons: [bool; 12],
    pointer: (i16, i16, bool),
    frame: Option<Frame>,
    audio: Vec<u8>,
}
static STATE: OnceLock<Mutex<Callbacks>> = OnceLock::new();
fn state() -> &'static Mutex<Callbacks> {
    STATE.get().expect("libretro callbacks initialized")
}

unsafe extern "C" fn environment(command: u32, data: *mut c_void) -> bool {
    let command = command & 0xffff;
    let Ok(s) = state().lock() else { return false };
    if data.is_null() {
        return matches!(command, 1 | 6 | 16 | 18 | 21 | 32 | 34 | 37 | 53 | GET_DEVICE_POWER);
    }
    if let Some(path) = s.paths.get(&command) {
        *(data as *mut *const c_char) = path.as_ptr();
        return true;
    }
    match command {
        3 => {
            *(data as *mut bool) = true;
            true
        }
        10 => *(data as *const i32) == 1,
        15 => {
            let v = &mut *(data as *mut Variable);
            if v.key.is_null() {
                return false;
            }
            let key = CStr::from_ptr(v.key).to_string_lossy();
            if let Some(value) = s.options.get(key.as_ref()) {
                v.value = value.as_ptr();
                true
            } else {
                v.value = std::ptr::null();
                false
            }
        }
        17 => {
            *(data as *mut bool) = false;
            true
        }
        39 | 52 | 59 => {
            *(data as *mut u32) = 0;
            true
        }
        50 => {
            *(data as *mut f32) = 60.0;
            true
        }
        GET_DEVICE_POWER => {
            *(data as *mut DevicePower) = DevicePower {
                state: POWERSTATE_CHARGED,
                seconds: -1,
                percent: 100,
            };
            true
        }
        1 | 6 | 16 | 18 | 21 | 32 | 34 | 37 | 53 => true,
        _ => false,
    }
}
unsafe extern "C" fn video(data: *const c_void, width: u32, height: u32, pitch: usize) {
    if data.is_null() || width == 0 || height == 0 || pitch > 16384 || height > 4096 {
        return;
    }
    if let Ok(mut s) = state().lock() {
        s.frame = Some(Frame {
            bytes: std::slice::from_raw_parts(data as *const u8, pitch * height as usize).to_vec(),
            width,
            height,
            pitch,
        });
    }
}
unsafe extern "C" fn audio(left: i16, right: i16) {
    if let Ok(mut s) = state().lock() {
        s.audio.extend_from_slice(&left.to_le_bytes());
        s.audio.extend_from_slice(&right.to_le_bytes());
    }
}
unsafe extern "C" fn audio_batch(data: *const i16, frames: usize) -> usize {
    if !data.is_null() && frames < 1_000_000 {
        if let Ok(mut s) = state().lock() {
            s.audio
                .extend_from_slice(std::slice::from_raw_parts(data as *const u8, frames * 4));
        }
    }
    frames
}
unsafe extern "C" fn poll() {}
unsafe extern "C" fn input(port: u32, device: u32, index: u32, id: u32) -> i16 {
    if port != 0 || index != 0 {
        return 0;
    }
    let Ok(s) = state().lock() else { return 0 };
    if device == 1 {
        return s.buttons.get(id as usize).copied().unwrap_or(false) as i16;
    }
    if device == 6 {
        return match id {
            0 => s.pointer.0,
            1 => s.pointer.1,
            2 => s.pointer.2 as i16,
            _ => 0,
        };
    }
    0
}

pub struct Core {
    _library: Library,
    run: unsafe extern "C" fn(),
    reset: unsafe extern "C" fn(),
    unload: unsafe extern "C" fn(),
    deinit: unsafe extern "C" fn(),
    live_cart: Option<LiveCart>,
    pub sample_rate: u32,
}

struct LiveCart {
    insert: unsafe extern "C" fn(*const u8, usize, *const u8, usize) -> bool,
    eject: unsafe extern "C" fn() -> bool,
    inserted: unsafe extern "C" fn() -> bool,
    save_size: unsafe extern "C" fn() -> usize,
    copy_save: unsafe extern "C" fn(*mut u8, usize) -> bool,
}
impl Core {
    pub fn open(
        dll: &Path,
        system: &Path,
        save: &Path,
        sd_enabled: bool,
        content: Option<&Path>,
    ) -> Result<Self, String> {
        let mut paths = HashMap::new();
        for id in [9, 30] {
            paths.insert(
                id,
                CString::new(system.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?,
            );
        }
        paths.insert(
            31,
            CString::new(save.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?,
        );
        let mut options = HashMap::new();
        for (key, value) in [
            ("melonds_console_mode", "dsi"),
            ("melonds_boot_mode", "native"),
            ("melonds_sysfile_mode", "native"),
            ("melonds_dsi_nand_path", "melonDS DS/dsi_nand.bin"),
            ("melonds_firmware_dsi_path", "melonDS DS/dsi_firmware.bin"),
            (
                "melonds_dsi_sdcard",
                if sd_enabled { "enabled" } else { "disabled" },
            ),
            (
                "melonds_dsi_sdcard_sync_sdcard_to_host",
                if sd_enabled { "enabled" } else { "disabled" },
            ),
            ("melonds_dsi_sdcard_readonly", "disabled"),
            ("melonds_number_of_screen_layouts", "1"),
            ("melonds_screen_layout1", "top-bottom"),
            ("melonds_screen_gap", "0"),
            ("melonds_touch_mode", "touch"),
        ] {
            options.insert(key.to_owned(), CString::new(value).unwrap());
        }
        STATE
            .set(Mutex::new(Callbacks {
                paths,
                options,
                buttons: [false; 12],
                pointer: (0, 0, false),
                frame: None,
                audio: Vec::new(),
            }))
            .map_err(|_| "Core already initialized".to_string())?;
        let library = unsafe { Library::new(dll) }.map_err(|e| e.to_string())?;
        unsafe {
            let api: unsafe extern "C" fn() -> u32 = library.get(b"retro_api_version")?;
            if api() != 1 {
                return Err("Unsupported libretro API version".into());
            }
            macro_rules! setter {
                ($name:expr,$callback:expr,$ty:ty) => {{
                    let f: unsafe extern "C" fn($ty) = library.get($name)?;
                    f($callback);
                }};
            }
            setter!(b"retro_set_environment", environment, Env);
            setter!(b"retro_set_video_refresh", video, Video);
            setter!(b"retro_set_audio_sample", audio, Audio);
            setter!(b"retro_set_audio_sample_batch", audio_batch, AudioBatch);
            setter!(b"retro_set_input_poll", poll, Poll);
            setter!(b"retro_set_input_state", input, Input);
            let init: unsafe extern "C" fn() = library.get(b"retro_init")?;
            init();
            let bytes = content
                .map(std::fs::read)
                .transpose()
                .map_err(|e| e.to_string())?;
            let path_bytes = content
                .map(|path| {
                    CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())
                })
                .transpose()?;
            let info = match (&bytes, &path_bytes) {
                (Some(bytes), Some(path)) => Some(GameInfo {
                    path: path.as_ptr(),
                    data: bytes.as_ptr() as *const c_void,
                    size: bytes.len(),
                    meta: std::ptr::null(),
                }),
                _ => None,
            };
            let load: unsafe extern "C" fn(*const GameInfo) -> bool =
                library.get(b"retro_load_game")?;
            if !load(info.as_ref().map_or(std::ptr::null(), |i| i as *const _)) {
                return Err("melonDS rejected the boot request".into());
            }
            let mut av = std::mem::MaybeUninit::<AvInfo>::uninit();
            let get_av: unsafe extern "C" fn(*mut AvInfo) =
                library.get(b"retro_get_system_av_info")?;
            get_av(av.as_mut_ptr());
            let sample_rate = av.assume_init().timing.sample_rate.round() as u32;
            let run = library.get::<unsafe extern "C" fn()>(b"retro_run")?;
            let reset = library.get::<unsafe extern "C" fn()>(b"retro_reset")?;
            let unload = library.get::<unsafe extern "C" fn()>(b"retro_unload_game")?;
            let deinit = library.get::<unsafe extern "C" fn()>(b"retro_deinit")?;
            let live_cart = (|| Some(LiveCart {
                insert: library.get(b"cpf_live_insert_nds").ok()?,
                eject: library.get(b"cpf_live_eject_nds").ok()?,
                inserted: library.get(b"cpf_live_nds_inserted").ok()?,
                save_size: library.get(b"cpf_live_nds_save_size").ok()?,
                copy_save: library.get(b"cpf_live_copy_nds_save").ok()?,
            }))();
            Ok(Self {
                _library: library,
                run,
                reset,
                unload,
                deinit,
                live_cart,
                sample_rate,
            })
        }
    }
    pub fn run(&self) {
        unsafe { (self.run)() }
    }
    pub fn reset(&self) {
        unsafe { (self.reset)() }
    }
    pub fn live_cart_supported(&self) -> bool { self.live_cart.is_some() }
    pub fn cart_inserted(&self) -> Option<bool> {
        self.live_cart.as_ref().map(|cart| unsafe { (cart.inserted)() })
    }
    pub fn insert_cart(&self, rom: &[u8], save: Option<&[u8]>) -> Result<(), String> {
        let cart = self.live_cart.as_ref().ok_or("This melonDS core has no live Slot-1 adapter")?;
        let save = save.unwrap_or(&[]);
        if !unsafe { (cart.insert)(rom.as_ptr(), rom.len(), save.as_ptr(), save.len()) } {
            return Err("melonDS rejected the live cartridge insertion".into());
        }
        Ok(())
    }
    pub fn copy_cart_save(&self) -> Result<Option<Vec<u8>>, String> {
        let cart = self.live_cart.as_ref().ok_or("This melonDS core has no live Slot-1 adapter")?;
        let size = unsafe { (cart.save_size)() };
        if size == 0 { return Ok(None); }
        if size > 32 * 1024 * 1024 { return Err("Cartridge save is unexpectedly large".into()); }
        let mut save = vec![0; size];
        if !unsafe { (cart.copy_save)(save.as_mut_ptr(), save.len()) } {
            return Err("Could not read the cartridge save before ejection".into());
        }
        Ok(Some(save))
    }
    pub fn eject_cart(&self) -> Result<(), String> {
        let cart = self.live_cart.as_ref().ok_or("This melonDS core has no live Slot-1 adapter")?;
        if !unsafe { (cart.eject)() } { return Err("melonDS could not eject Slot-1".into()); }
        Ok(())
    }
    pub fn take_output() -> (Option<Frame>, Vec<u8>) {
        let mut s = state().lock().unwrap();
        (s.frame.take(), std::mem::take(&mut s.audio))
    }
    pub fn button(name: &str, pressed: bool) {
        let id = match name {
            "B" => 0,
            "Y" => 1,
            "SELECT" => 2,
            "START" => 3,
            "UP" => 4,
            "DOWN" => 5,
            "LEFT" => 6,
            "RIGHT" => 7,
            "A" => 8,
            "X" => 9,
            "L" => 10,
            "R" => 11,
            _ => return,
        };
        state().lock().unwrap().buttons[id] = pressed;
    }
    pub fn touch(x: i64, y: i64, pressed: bool) {
        let x = x.clamp(0, 255);
        let y = y.clamp(0, 191);
        let px = ((x as f64 / 255.0) * 65535.0 - 32768.0) as i16;
        let py = (((192 + y) as f64 / 383.0) * 65535.0 - 32768.0) as i16;
        state().lock().unwrap().pointer = (px, py, pressed);
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        unsafe {
            (self.unload)();
            (self.deinit)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_dsi_battery_reports_full_and_charged() {
        STATE.get_or_init(|| Mutex::new(Callbacks {
            paths: HashMap::new(), options: HashMap::new(), buttons: [false; 12],
            pointer: (0, 0, false), frame: None, audio: Vec::new(),
        }));
        assert_eq!(std::mem::size_of::<DevicePower>(), 12);
        let mut power = DevicePower { state: 0, seconds: 0, percent: 0 };
        unsafe {
            assert!(environment(GET_DEVICE_POWER, std::ptr::null_mut()));
            assert!(environment(GET_DEVICE_POWER, &mut power as *mut _ as *mut c_void));
        }
        assert_eq!(power.state, POWERSTATE_CHARGED);
        assert_eq!(power.seconds, -1);
        assert_eq!(power.percent, 100);
    }
}
