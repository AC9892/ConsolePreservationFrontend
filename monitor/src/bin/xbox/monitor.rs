//! External, read-only Xenia Canary process and log monitoring for Xbox.exe.
//! No emulator control, memory reads, or firmware data are exposed here.

use std::{collections::{HashMap, HashSet, VecDeque}, fs::File, io::{Read, Seek, SeekFrom}, path::{Path, PathBuf}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE},
    System::{
        Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::{GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
    },
};

const MAX_LOG_TAIL: u64 = 64 * 1024;
const MAX_EVENTS: usize = 16;

pub struct Report {
    pub status: String,
    pub processes: String,
    pub log_status: String,
    pub diagnostics: String,
    pub events: String,
}

struct Tracked {
    handle: HANDLE,
    image: Option<PathBuf>,
    started: Option<SystemTime>,
    last_cpu: Option<u64>,
    last_poll: Instant,
}

impl Drop for Tracked {
    fn drop(&mut self) { unsafe { CloseHandle(self.handle); } }
}

pub struct Monitor {
    tracked: HashMap<u32, Tracked>,
    events: VecDeque<String>,
    has_polled: bool,
    last_image: Option<PathBuf>,
    last_start: Option<SystemTime>,
}

impl Monitor {
    pub fn new() -> Self { Self { tracked: HashMap::new(), events: VecDeque::new(), has_polled: false, last_image: None, last_start: None } }

    fn event(&mut self, value: String) {
        self.events.push_front(value);
        self.events.truncate(MAX_EVENTS);
    }

    pub fn poll(&mut self, chosen_log: &str) -> Report {
        let found = match enumerate() {
            Ok(found) => found,
            Err(_) => return Report {
                status: "Process enumeration unavailable".into(), processes: "Windows did not provide a process snapshot.".into(),
                log_status: "Log monitoring paused.".into(), diagnostics: String::new(),
                events: self.events.iter().cloned().collect::<Vec<_>>().join("\r\n"),
            },
        };
        let now = Instant::now();
        let seen: HashSet<u32> = found.iter().map(|item| item.0).collect();
        for pid in self.tracked.keys().copied().filter(|pid| !seen.contains(pid)).collect::<Vec<_>>() {
            if let Some(old) = self.tracked.remove(&pid) {
                if old.image.is_some() { self.last_image = old.image.clone(); }
                if old.started.is_some() { self.last_start = old.started; }
                self.event(exit_event_from_handle(pid, old.handle));
            }
        }

        let mut lines = Vec::new();
        for (pid, threads) in found {
            let replace = self.tracked.get(&pid).is_some_and(|old| {
                let mut code = 0;
                unsafe { GetExitCodeProcess(old.handle, &mut code) == 0 || code != STILL_ACTIVE as u32 }
            });
            if replace {
                if let Some(old) = self.tracked.remove(&pid) { self.event(exit_event_from_handle(pid, old.handle)); }
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = self.tracked.entry(pid) {
                let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
                let handle = if handle.is_null() {
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }
                } else { handle };
                if handle.is_null() {
                    lines.push(format!("PID {pid}  access denied; metrics unavailable"));
                    continue;
                }
                let image = process_image(handle);
                if image.is_some() { self.last_image = image.clone(); }
                let started = process_start_time(handle);
                if started.is_some() { self.last_start = started; }
                entry.insert(Tracked { handle, image, started, last_cpu: None, last_poll: now });
                self.event(if self.has_polled { format!("Xenia launch observed: PID {pid}") }
                    else { format!("Xenia already running: PID {pid}") });
            }
            let tracked = self.tracked.get_mut(&pid).expect("inserted process");
            let cpu = process_cpu_time(tracked.handle);
            let cpu_percent = match (cpu, tracked.last_cpu) {
                (Some(current), Some(previous)) if current >= previous => {
                    let elapsed = now.duration_since(tracked.last_poll).as_secs_f64();
                    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
                    if elapsed > 0.0 { Some(((current - previous) as f64 / 10_000_000.0 / elapsed / cores * 100.0).clamp(0.0, 100.0)) } else { None }
                }
                _ => None,
            };
            tracked.last_cpu = cpu;
            tracked.last_poll = now;
            let memory = process_memory(tracked.handle);
            lines.push(format!("PID {pid}  CPU {}  RAM {}  threads {threads}",
                cpu_percent.map(|value| format!("{value:.1}%")).unwrap_or_else(|| "sampling…".into()),
                memory.map(|bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0)).unwrap_or_else(|| "unavailable".into())));
        }
        self.has_polled = true;
        lines.sort();
        let (log_status, diagnostics) = self.read_log(chosen_log);
        Report {
            status: if lines.is_empty() { "Xenia Canary is not running".into() }
                else { format!("Xenia Canary running: {} process{}", lines.len(), if lines.len() == 1 { "" } else { "es" }) },
            processes: if lines.is_empty() { "Waiting for CPF Console Settings to launch Xenia.".into() } else { lines.join("\r\n") },
            log_status, diagnostics,
            events: if self.events.is_empty() { "No launch or exit events observed yet.".into() }
                else { self.events.iter().cloned().collect::<Vec<_>>().join("\r\n") },
        }
    }

    fn read_log(&self, chosen_log: &str) -> (String, String) {
        let path = if chosen_log.trim().is_empty() {
            self.tracked.values().filter_map(|item| item.image.as_ref())
                .chain(self.last_image.as_ref())
                .flat_map(default_log_candidates)
                .filter_map(|path| std::fs::metadata(&path).ok().and_then(|m| m.modified().ok().map(|time| (path, time))))
                .max_by_key(|(_, modified)| *modified).map(|(path, _)| path)
        } else {
            let path = PathBuf::from(chosen_log.trim());
            if !path.is_absolute() { return ("Enter an absolute log path, or clear the field for auto detection.".into(), String::new()); }
            Some(path)
        };
        let Some(path) = path else {
            return ("No default Xenia log found. Enter the log path used by CPF's launcher if it overrides --log_file.".into(), String::new());
        };
        match log_tail(&path) {
            Ok((bytes, modified, lines)) => {
                let latest_start = self.tracked.values().filter_map(|item| item.started).max().or(self.last_start);
                if latest_start.is_some_and(|start| !log_is_current(modified, start)) {
                    return ("Available log predates the observed Xenia session; select the current --log_file path.".into(), String::new());
                }
                let age = SystemTime::now().duration_since(modified).map(|d| d.as_secs()).unwrap_or(0);
                let diagnostics = notable_lines(&lines);
                (format!("Log available: {bytes} bytes; last update {age}s ago. Showing sanitized diagnostic lines."), diagnostics)
            }
            Err(_) => ("Selected Xenia log cannot be read. Check its path and access permissions.".into(), String::new()),
        }
    }
}

fn log_is_current(modified: SystemTime, started: SystemTime) -> bool {
    modified + Duration::from_secs(2) >= started
}

fn exit_event_from_handle(pid: u32, handle: HANDLE) -> String {
    let mut code = 0;
    if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 || code == STILL_ACTIVE as u32 {
        format!("PID {pid} exited; exit code unavailable")
    } else if code == 0 {
        format!("PID {pid} exited normally (0x00000000)")
    } else { exit_event(pid, code) }
}

fn exit_event(pid: u32, code: u32) -> String {
    let cause = match code {
        0xC0000005 => "access violation",
        0xC000001D => "illegal instruction",
        0xC00000FD => "stack overflow",
        0xC0000409 => "security check failure",
        0xC0000374 => "heap corruption",
        _ => return format!("PID {pid} abnormal exit 0x{code:08X} (possible crash or forced stop)"),
    };
    format!("PID {pid} crash-like exit 0x{code:08X} ({cause}); confirm with Xenia log")
}

fn default_log_candidates(image: &PathBuf) -> impl Iterator<Item = PathBuf> {
    let parent = image.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    ["xenia_canary.log", "xenia-canary.log", "xenia.log"].into_iter().map(move |name| parent.join(name))
}

fn enumerate() -> std::io::Result<Vec<(u32, u32)>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE { return Err(std::io::Error::last_os_error()); }
    let mut result = Vec::new();
    let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if matches!(name.to_ascii_lowercase().as_str(), "xenia_canary.exe" | "xenia-canary.exe") {
            result.push((entry.th32ProcessID, entry.cntThreads));
        }
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot); }
    Ok(result)
}

fn process_image(handle: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 { return None; }
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
}

fn filetime(value: FILETIME) -> u64 { ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64 }

fn process_start_time(handle: HANDLE) -> Option<SystemTime> {
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 { return None; }
    let ticks = filetime(created);
    const EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    ticks.checked_sub(EPOCH_TICKS).map(|value| UNIX_EPOCH + Duration::from_nanos(value.saturating_mul(100)))
}

fn process_cpu_time(handle: HANDLE) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 { return None; }
    Some(filetime(kernel).saturating_add(filetime(user)))
}

fn process_memory(handle: HANDLE) -> Option<usize> {
    let mut counters = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    if unsafe { GetProcessMemoryInfo(handle, &mut counters, counters.cb) } == 0 { return None; }
    Some(counters.WorkingSetSize)
}

fn log_tail(path: &Path) -> std::io::Result<(u64, SystemTime, Vec<String>)> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let start = metadata.len().saturating_sub(MAX_LOG_TAIL);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(MAX_LOG_TAIL).read_to_end(&mut bytes)?;
    let value = String::from_utf8_lossy(&bytes);
    let mut lines = value.lines();
    if start > 0 { lines.next(); } // The first line may begin in the middle.
    Ok((metadata.len(), metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), lines.map(str::to_owned).collect()))
}

fn notable_lines(lines: &[String]) -> String {
    let selected = lines.iter().filter(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("error") || lower.contains("failed") || lower.contains("warning") ||
        lower.contains("fatal") || lower.contains("exception") || lower.contains("crash") ||
        lower.contains("undefined extern") || lower.starts_with("w>") || lower.starts_with("!>")
    }).rev().take(18).map(|line| sanitize_line(line)).collect::<Vec<_>>();
    if selected.is_empty() { "No diagnostic warnings or errors in the recent log tail.".into() }
    else { selected.into_iter().rev().collect::<Vec<_>>().join("\r\n") }
}

fn sanitize_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if ["cpu key", "cpukey", "keyvault", "console id", "console serial", "xuid", "device id"]
        .iter().any(|word| lower.contains(word)) { return "[sensitive log line hidden]".into(); }
    let mut output = Vec::new();
    for token in line.split_whitespace() {
        let chars = token.as_bytes();
        if token.starts_with("\\\\") || chars.windows(3).any(|part| part[0].is_ascii_alphabetic() && part[1] == b':' && matches!(part[2], b'\\' | b'/')) {
            output.push("[local path]".to_owned());
            continue;
        }
        let mut sanitized = String::new();
        let mut hex_run = String::new();
        for ch in token.chars().chain(std::iter::once(' ')) {
            if ch.is_ascii_hexdigit() { hex_run.push(ch); }
            else {
                if hex_run.len() >= 12 { sanitized.push_str("[hex]"); }
                else { sanitized.push_str(&hex_run); }
                hex_run.clear();
                if ch != ' ' { sanitized.push(ch); }
            }
        }
        output.push(sanitized);
    }
    let joined = output.join(" ");
    joined.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_redact_paths_and_long_identifiers() {
        let lines = vec!["w> failed to load R:\\private\\dash.xex for AABBCCDDEEFF00112233".into()];
        let result = notable_lines(&lines);
        assert!(result.contains("[local path]"));
        assert!(result.contains("[hex]"));
        assert!(!result.contains("private"));
        assert!(!result.contains("AABBCCDDEEFF"));
    }

    #[test]
    fn sensitive_lines_are_hidden() {
        assert_eq!(sanitize_line("console serial: 123456789012"), "[sensitive log line hidden]");
    }

    #[test]
    fn distinguishes_known_crash_codes_from_other_nonzero_exits() {
        assert!(exit_event(42, 0xC0000005).contains("access violation"));
        assert!(exit_event(42, 1).contains("possible crash or forced stop"));
    }

    #[test]
    fn previous_session_log_is_not_current() {
        let start = UNIX_EPOCH + Duration::from_secs(100);
        assert!(!log_is_current(UNIX_EPOCH + Duration::from_secs(90), start));
        assert!(log_is_current(UNIX_EPOCH + Duration::from_secs(99), start));
    }
}
