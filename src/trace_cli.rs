mod backend;
mod diagnostics;
mod ipmi;
#[cfg(all(windows, target_arch = "x86_64"))]
mod debugger {
    use crate::backend;
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        fs::{self, File},
        io::Read,
        os::windows::{io::AsRawHandle, process::CommandExt},
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::{
            Diagnostics::Debug::*,
            Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION},
            ProcessStatus::GetMappedFileNameW,
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::*,
        },
    };
    #[derive(Serialize)]
    struct Fault {
        thread_id: u32,
        first_chance: bool,
        code: String,
        exception_address: String,
        access_operation: Option<usize>,
        accessed_address: Option<String>,
        registers: BTreeMap<String, String>,
        instruction_bytes: String,
        stack_qwords: Vec<String>,
        instruction_allocation_base: Option<String>,
        instruction_image: Option<String>,
    }
    #[derive(Serialize)]
    struct Trace {
        backend_profile: String,
        backend_sha256: String,
        executable_sha256: String,
        lle_overrides: Vec<serde_json::Value>,
        ipmi_requests: Vec<IpmiRequest>,
        ipmi_probe_enabled: bool,
        available_commit_bytes: u64,
        required_backing_bytes: u64,
        stdout: String,
        stderr: String,
        exit_code: Option<u32>,
        timed_out: bool,
        timeout_seconds: u64,
        timeout_threads: Vec<Fault>,
        exception_counts: BTreeMap<String, usize>,
        last_faults: VecDeque<Fault>,
        unhandled_fault: Option<Fault>,
        diagnostics: Vec<String>,
        authentic_dashboard_verified: bool,
        runtime_diagnostics: crate::diagnostics::RuntimeDiagnostics,
    }
    #[derive(Serialize)]
    struct IpmiRequest {
        thread_id: u32,
        registers: BTreeMap<String, String>,
        request_hex: String,
        request: Option<crate::ipmi::CreateClientRequest>,
        initial_result_hex: String,
        request_pointer_1_text: String,
        request_pointer_2_hex: String,
        descriptor: Option<crate::ipmi::CreateDescriptor>,
        descriptor_error: Option<String>,
    }
    const IPMI_CALL: u64 = 0x8126b401c;
    #[repr(C, align(16))]
    struct ProbeContext(CONTEXT);
    // Debug events suspend the debuggee threads. Use hardware debug registers:
    // never patch guest instructions, requests, results or return values.
    fn arm_probe(thread: u32) -> Option<(u64, u64, u64)> {
        let handle = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, thread) };
        if handle.is_null() {
            return None;
        }
        let mut c = ProbeContext(CONTEXT {
            ContextFlags: CONTEXT_ALL_AMD64,
            ..Default::default()
        });
        let mut previous = None;
        if unsafe { GetThreadContext(handle, &mut c.0) } != 0 && c.0.Dr7 & 0xff == 0 {
            let saved = (c.0.Dr0, c.0.Dr6, c.0.Dr7);
            c.0.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            c.0.Dr0 = IPMI_CALL;
            c.0.Dr6 = 0;
            c.0.Dr7 = (c.0.Dr7 & !0xf0000) | 1;
            if unsafe { SetThreadContext(handle, &c.0) } != 0 {
                previous = Some(saved);
            }
        }
        unsafe {
            CloseHandle(handle);
        }
        previous
    }
    fn take_probe(
        process: HANDLE,
        thread: u32,
        saved: (u64, u64, u64),
        rearm: bool,
    ) -> Result<Option<IpmiRequest>, String> {
        let handle = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_SET_CONTEXT, 0, thread) };
        if handle.is_null() {
            return Err("Cannot open probe thread".into());
        }
        let mut c = ProbeContext(CONTEXT {
            ContextFlags: CONTEXT_ALL_AMD64,
            ..Default::default()
        });
        if unsafe { GetThreadContext(handle, &mut c.0) } == 0 {
            unsafe {
                CloseHandle(handle);
            }
            return Err("Cannot read probe context".into());
        }
        if c.0.Dr6 & 1 == 0 || c.0.Rip != IPMI_CALL {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        let hex = |b: Vec<u8>| {
            b.iter()
                .map(|v| format!("{v:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let valid = memory(process, IPMI_CALL, 5) == [0xe8, 0xe7, 0xc1, 0xff, 0xff]
            && c.0.Rdi == 2
            && c.0.R8 == 24;
        let bytes = if valid {
            memory(process, c.0.Rcx, 24)
        } else {
            vec![]
        };
        let pointer = |offset| {
            bytes
                .get(offset..offset + 8)
                .map(|v| u64::from_le_bytes(v.try_into().unwrap()))
        };
        let descriptor_bytes = pointer(16)
            .map(|p| memory(process, p, crate::ipmi::CREATE_DESCRIPTOR_BYTES))
            .unwrap_or_default();
        let decoded = crate::ipmi::decode_create_descriptor(&descriptor_bytes);
        let descriptor_error = decoded.as_ref().err().cloned();
        let request = valid.then(|| IpmiRequest {
            thread_id: thread,
            registers: [
                ("rip", c.0.Rip),
                ("rdi", c.0.Rdi),
                ("rsi", c.0.Rsi),
                ("rdx", c.0.Rdx),
                ("rcx", c.0.Rcx),
                ("r8", c.0.R8),
                ("r9", c.0.R9),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), format!("0x{v:x}")))
            .collect(),
            request_hex: hex(bytes.clone()),
            request: crate::ipmi::decode_create_request(&bytes).ok(),
            initial_result_hex: hex(memory(process, c.0.Rdx, 4)),
            request_pointer_1_text: pointer(8)
                .map(|p| {
                    let value = memory(process, p, 128);
                    String::from_utf8_lossy(
                        &value[..value.iter().position(|b| *b == 0).unwrap_or(value.len())],
                    )
                    .into_owned()
                })
                .unwrap_or_default(),
            request_pointer_2_hex: hex(descriptor_bytes),
            descriptor: decoded.ok(),
            descriptor_error,
        });
        if rearm && valid {
            c.0.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64 | CONTEXT_CONTROL_AMD64;
            c.0.Dr6 = 0;
            c.0.EFlags |= 0x10000; // Resume this instruction once without re-triggering DR0.
        } else {
            c.0.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
            (c.0.Dr0, c.0.Dr6, c.0.Dr7) = saved;
        }
        let restored = unsafe { SetThreadContext(handle, &c.0) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        if !restored {
            return Err("Cannot restore probe debug registers".into());
        }
        if !valid {
            return Err("IPMI probe hit a different instruction or ABI; sample rejected".into());
        }
        Ok(request)
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
    fn memory(process: HANDLE, address: u64, size: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; size];
        let mut read = 0;
        unsafe {
            ReadProcessMemory(
                process,
                address as *const _,
                bytes.as_mut_ptr() as *mut _,
                size,
                &mut read,
            );
        };
        bytes.truncate(read);
        bytes
    }
    fn fault(process: HANDLE, thread: u32, info: &EXCEPTION_DEBUG_INFO) -> Fault {
        let record = &info.ExceptionRecord;
        let mut result = Fault {
            thread_id: thread,
            first_chance: info.dwFirstChance != 0,
            code: format!("0x{:08x}", record.ExceptionCode as u32),
            exception_address: format!("0x{:x}", record.ExceptionAddress as usize),
            access_operation: None,
            accessed_address: None,
            registers: BTreeMap::new(),
            instruction_bytes: String::new(),
            stack_qwords: vec![],
            instruction_allocation_base: None,
            instruction_image: None,
        };
        if record.ExceptionCode == EXCEPTION_ACCESS_VIOLATION && record.NumberParameters >= 2 {
            result.access_operation = Some(record.ExceptionInformation[0]);
            result.accessed_address = Some(format!("0x{:x}", record.ExceptionInformation[1]));
        }
        #[repr(C, align(16))]
        struct Aligned(CONTEXT);
        let handle =
            unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, thread) };
        if !handle.is_null() {
            let mut context = Aligned(CONTEXT {
                ContextFlags: CONTEXT_ALL_AMD64,
                ..Default::default()
            });
            if unsafe { GetThreadContext(handle, &mut context.0) } != 0 {
                let c = &context.0;
                for (name, value) in [
                    ("rip", c.Rip),
                    ("rsp", c.Rsp),
                    ("rbp", c.Rbp),
                    ("rax", c.Rax),
                    ("rbx", c.Rbx),
                    ("rcx", c.Rcx),
                    ("rdx", c.Rdx),
                    ("rsi", c.Rsi),
                    ("rdi", c.Rdi),
                    ("r8", c.R8),
                    ("r9", c.R9),
                    ("r10", c.R10),
                    ("r11", c.R11),
                    ("r12", c.R12),
                    ("r13", c.R13),
                    ("r14", c.R14),
                    ("r15", c.R15),
                ] {
                    result.registers.insert(name.into(), format!("0x{value:x}"));
                }
                result.instruction_bytes = memory(process, c.Rip, 32)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut mapping = MEMORY_BASIC_INFORMATION::default();
                if unsafe {
                    VirtualQueryEx(
                        process,
                        c.Rip as *const _,
                        &mut mapping,
                        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                } != 0
                {
                    result.instruction_allocation_base =
                        Some(format!("0x{:x}", mapping.AllocationBase as usize));
                }
                let mut path = vec![0u16; 32768];
                let count = unsafe {
                    GetMappedFileNameW(
                        process,
                        c.Rip as *const _,
                        path.as_mut_ptr(),
                        path.len() as u32,
                    )
                };
                if count > 0 {
                    result.instruction_image =
                        Some(String::from_utf16_lossy(&path[..count as usize]));
                }
                result.stack_qwords = memory(process, c.Rsp, 256)
                    .chunks_exact(8)
                    .map(|v| format!("0x{:x}", u64::from_le_bytes(v.try_into().unwrap())))
                    .collect();
            }
            unsafe {
                CloseHandle(handle);
            }
        }
        result
    }
    pub fn run(
        base: &Path,
        backend_path: &Path,
        timeout_seconds: u64,
        ipmi_probe: bool,
    ) -> Result<(), String> {
        if !(1..=60).contains(&timeout_seconds) {
            return Err("Trace timeout must be between 1 and 60 seconds".into());
        }
        let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
        let backend_path = fs::canonicalize(backend_path).map_err(|e| e.to_string())?;
        let plan: serde_json::Value = serde_json::from_slice(
            &fs::read(base.join("plan.json"))
                .map_err(|_| "A prepared sandbox plan.json is required")?,
        )
        .map_err(|e| e.to_string())?;
        let source = fs::canonicalize(plan["source"].as_str().ok_or("Plan has no source root")?)
            .map_err(|e| e.to_string())?;
        if base.starts_with(&source) || source.starts_with(&base) {
            return Err("Debug sandbox must be separate from original dumps".into());
        }
        let backend_hash = hash(&backend_path)?;
        let profile = backend::profile_for_hash(&backend_hash)
            .ok_or("Backend executable is not in the verified profile registry")?;
        let executable_hash = hash(&base.join("app0/eboot.bin"))?;
        if Some(executable_hash.as_str()) != plan["executable_sha256"].as_str() {
            return Err("Sandbox entry point hash differs from plan".into());
        }
        let lle_overrides = match plan.get("lle_overrides") {
            None => vec![],
            Some(value) => value.as_array().ok_or("Invalid LLE override list")?.clone(),
        };
        for entry in &lle_overrides {
            let name = entry["module"]
                .as_str()
                .ok_or("Override has no module name")?;
            if name.is_empty() || name.contains(['/', '\\', ':']) || !name.ends_with(".sprx") {
                return Err("Invalid LLE module basename".into());
            }
            let module = base.join("user/sys_modules/NPXS20001").join(name);
            let resolved = fs::canonicalize(&module).map_err(|e| e.to_string())?;
            if !resolved.starts_with(&base) {
                return Err("LLE override must remain within the sandbox".into());
            }
            if Some(hash(&module)?.as_str()) != entry["sha256"].as_str() {
                return Err(format!("LLE override hash differs from plan: {name}"));
            }
        }
        if ipmi_probe
            && (profile.id != "shadps4-v0.20.0"
                || hash(&base.join("user/sys_modules/NPXS20001/libSceIpmi.sprx"))?
                    != "c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5")
        {
            return Err("IPMI probe requires the pinned v0.20 native-IPMI experiment".into());
        }
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return Err("Cannot read host commit status".into());
        }
        if status.ullAvailPageFile < profile.backing_bytes + profile.headroom_bytes {
            return Err(format!(
                "Insufficient available host commit: {:.2} GiB for {}",
                status.ullAvailPageFile as f64 / 1073741824.0,
                profile.id
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let output = base.join(format!("trace-{now}"));
        fs::create_dir(&output).map_err(|e| e.to_string())?;
        let stdout = output.join("stdout.txt");
        let stderr = output.join("stderr.txt");
        let mut child = Command::new(&backend_path)
            .current_dir(&base)
            .args([
                "--fullscreen",
                "false",
                "--ignore-game-patch",
                "app0/eboot.bin",
            ])
            .creation_flags(DEBUG_ONLY_THIS_PROCESS | CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).map_err(|e| e.to_string())?)
            .stderr(File::create(&stderr).map_err(|e| e.to_string())?)
            .spawn()
            .map_err(|e| e.to_string())?;
        let process = child.as_raw_handle() as HANDLE;
        let mut trace = Trace {
            backend_profile: profile.id.into(),
            backend_sha256: backend_hash,
            executable_sha256: executable_hash,
            lle_overrides,
            ipmi_requests: vec![],
            ipmi_probe_enabled: ipmi_probe,
            available_commit_bytes: status.ullAvailPageFile,
            required_backing_bytes: profile.backing_bytes,
            stdout: stdout.to_string_lossy().into_owned(),
            stderr: stderr.to_string_lossy().into_owned(),
            exit_code: None,
            timed_out: false,
            timeout_seconds,
            timeout_threads: vec![],
            exception_counts: BTreeMap::new(),
            last_faults: VecDeque::new(),
            unhandled_fault: None,
            diagnostics: vec![],
            authentic_dashboard_verified: false,
            runtime_diagnostics: Default::default(),
        };
        let start = Instant::now();
        let mut initial_breakpoint = true;
        let mut threads = BTreeSet::new();
        let mut timeout_break_requested = false;
        let mut probes = BTreeMap::new();
        loop {
            if start.elapsed() > Duration::from_secs(timeout_seconds) && !trace.timed_out {
                trace.timed_out = true;
                timeout_break_requested = unsafe { DebugBreakProcess(process) } != 0;
                if !timeout_break_requested {
                    trace.diagnostics.push(format!(
                        "Timeout break failed: {}",
                        std::io::Error::last_os_error()
                    ));
                    let _ = child.kill();
                }
            }
            if start.elapsed() > Duration::from_secs(timeout_seconds + 5) {
                trace
                    .diagnostics
                    .push("Debugger event loop timed out during cleanup".into());
                break;
            }
            let mut event = DEBUG_EVENT::default();
            if unsafe { WaitForDebugEventEx(&mut event, 100) } == 0 {
                continue;
            }
            let mut disposition = DBG_CONTINUE;
            let mut exited = false;
            let mut terminate_after_continue = false;
            match event.dwDebugEventCode {
                EXCEPTION_DEBUG_EVENT => {
                    let info = unsafe { event.u.Exception };
                    let code = info.ExceptionRecord.ExceptionCode;
                    *trace
                        .exception_counts
                        .entry(format!("0x{:08x}", code as u32))
                        .or_default() += 1;
                    let mut probe_handled = false;
                    if code == EXCEPTION_SINGLE_STEP {
                        if let Some(saved) = probes.get(&event.dwThreadId).copied() {
                            let rearm = trace.ipmi_requests.len() < 63;
                            match take_probe(process, event.dwThreadId, saved, rearm) {
                                Ok(Some(sample)) => {
                                    if !rearm {
                                        probes.remove(&event.dwThreadId);
                                    }
                                    if trace.ipmi_requests.len() < 64 {
                                        trace.ipmi_requests.push(sample);
                                    }
                                    probe_handled = true;
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    trace.diagnostics.push(e);
                                    terminate_after_continue = true;
                                }
                            }
                        }
                    }
                    if probe_handled {
                        // This hardware breakpoint belongs to our diagnostic probe.
                    } else if code == EXCEPTION_BREAKPOINT && timeout_break_requested {
                        timeout_break_requested = false;
                        for thread in &threads {
                            let mut snapshot = fault(process, *thread, &info);
                            snapshot.code =
                                "timeout-snapshot; not an exception in this thread".into();
                            trace.timeout_threads.push(snapshot);
                        }
                        terminate_after_continue = true;
                    } else if code == EXCEPTION_BREAKPOINT && initial_breakpoint {
                        initial_breakpoint = false;
                        if ipmi_probe {
                            for thread in &threads {
                                if let Some(saved) = arm_probe(*thread) {
                                    probes.insert(*thread, saved);
                                } else {
                                    trace.diagnostics.push(format!(
                                        "Could not arm IPMI probe on thread {thread}"
                                    ));
                                }
                            }
                        }
                    } else {
                        disposition = DBG_EXCEPTION_NOT_HANDLED;
                        if info.dwFirstChance == 0 {
                            trace.unhandled_fault = Some(fault(process, event.dwThreadId, &info));
                        } else if code == EXCEPTION_ACCESS_VIOLATION
                            || code == EXCEPTION_ILLEGAL_INSTRUCTION
                            || code == EXCEPTION_INT_DIVIDE_BY_ZERO
                        {
                            trace
                                .last_faults
                                .push_back(fault(process, event.dwThreadId, &info));
                            while trace.last_faults.len() > 8 {
                                trace.last_faults.pop_front();
                            }
                        }
                    }
                }
                CREATE_PROCESS_DEBUG_EVENT => {
                    threads.insert(event.dwThreadId);
                    let info = unsafe { event.u.CreateProcessInfo };
                    if !info.hFile.is_null() {
                        unsafe {
                            CloseHandle(info.hFile);
                        }
                    }
                }
                CREATE_THREAD_DEBUG_EVENT => {
                    threads.insert(event.dwThreadId);
                    if ipmi_probe
                        && !initial_breakpoint
                        && probes.len() + trace.ipmi_requests.len() < 64
                    {
                        if let Some(saved) = arm_probe(event.dwThreadId) {
                            probes.insert(event.dwThreadId, saved);
                        } else {
                            trace.diagnostics.push(format!(
                                "Could not arm IPMI probe on thread {}",
                                event.dwThreadId
                            ));
                        }
                    }
                }
                EXIT_THREAD_DEBUG_EVENT => {
                    threads.remove(&event.dwThreadId);
                    probes.remove(&event.dwThreadId);
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let info = unsafe { event.u.LoadDll };
                    if !info.hFile.is_null() {
                        unsafe {
                            CloseHandle(info.hFile);
                        }
                    }
                }
                EXIT_PROCESS_DEBUG_EVENT => {
                    trace.exit_code = Some(unsafe { event.u.ExitProcess.dwExitCode });
                    exited = true;
                }
                _ => {}
            }
            if unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, disposition) } == 0
            {
                trace.diagnostics.push(format!(
                    "ContinueDebugEvent failed: {}",
                    std::io::Error::last_os_error()
                ));
                let _ = child.kill();
                break;
            }
            if exited {
                break;
            }
            if terminate_after_continue {
                let _ = child.kill();
            }
        }
        if trace.exit_code.is_none() {
            let _ = child.kill();
        } else {
            let _ = child.wait();
        }
        let report = output.join("exception.json");
        trace.runtime_diagnostics =
            crate::diagnostics::analyze(&fs::read_to_string(&stdout).unwrap_or_default());
        fs::write(
            &report,
            serde_json::to_vec_pretty(&trace).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        println!("{}", report.display());
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        // A tiny synthetic child exercises actual Windows debug registers without
        // loading any console software or allocating the backend's large heap.
        #[test]
        #[ignore]
        fn synthetic_probe_child() {
            assert_eq!(std::env::var("PS4_TRACE_FIXTURE").as_deref(), Ok("1"));
            use windows_sys::Win32::System::Memory::*;
            let base = unsafe {
                VirtualAlloc(
                    0x8126b0000usize as *const _,
                    0x5000,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_EXECUTE_READWRITE,
                )
            };
            assert_eq!(base as usize, 0x8126b0000);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    [0xe8u8, 0xe7, 0xc1, 0xff, 0xff, 0xc3].as_ptr(),
                    IPMI_CALL as *mut u8,
                    6,
                );
                std::ptr::write((0x8126b0000usize + 0x208) as *mut u8, 0xc3);
                assert_ne!(FlushInstructionCache(GetCurrentProcess(), base, 0x5000), 0);
            }
            let name = b"SyntheticIpmiService\0";
            let config = [0x5au8; crate::ipmi::CREATE_DESCRIPTOR_BYTES];
            let request = [0x1234u64, name.as_ptr() as u64, config.as_ptr() as u64];
            let mut result = u32::MAX;
            let call: unsafe extern "sysv64" fn(u64, u64, *mut u32, *const u64, u64, u64) =
                unsafe { std::mem::transmute(IPMI_CALL as usize) };
            unsafe {
                call(2, 0, &mut result, request.as_ptr(), 24, 0xdeadbadecafebeaf);
                call(2, 0, &mut result, request.as_ptr(), 24, 0xdeadbadecafebeaf);
            }
            assert_eq!(result, u32::MAX, "Probe must not change output values");
            unsafe {
                assert_ne!(VirtualFree(base, 0, MEM_RELEASE), 0);
            }
        }
        #[test]
        fn hardware_probe_captures_request_without_patching() {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "debugger::tests::synthetic_probe_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("PS4_TRACE_FIXTURE", "1")
                .creation_flags(DEBUG_ONLY_THIS_PROCESS | CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let process = child.as_raw_handle() as HANDLE;
            let start = Instant::now();
            let mut probes = BTreeMap::new();
            let mut initial = true;
            let mut samples = vec![];
            let mut exit = None;
            while start.elapsed() < Duration::from_secs(10) {
                let mut event = DEBUG_EVENT::default();
                if unsafe { WaitForDebugEventEx(&mut event, 100) } == 0 {
                    continue;
                }
                let mut disposition = DBG_CONTINUE;
                match event.dwDebugEventCode {
                    CREATE_PROCESS_DEBUG_EVENT => {
                        probes.insert(
                            event.dwThreadId,
                            arm_probe(event.dwThreadId).expect("arm primary"),
                        );
                        let info = unsafe { event.u.CreateProcessInfo };
                        unsafe {
                            if !info.hFile.is_null() {
                                CloseHandle(info.hFile);
                            }
                        }
                    }
                    CREATE_THREAD_DEBUG_EVENT => {
                        probes.insert(
                            event.dwThreadId,
                            arm_probe(event.dwThreadId).expect("arm test thread"),
                        );
                    }
                    LOAD_DLL_DEBUG_EVENT => {
                        let info = unsafe { event.u.LoadDll };
                        unsafe {
                            if !info.hFile.is_null() {
                                CloseHandle(info.hFile);
                            }
                        }
                    }
                    EXCEPTION_DEBUG_EVENT => {
                        let info = unsafe { event.u.Exception };
                        if info.ExceptionRecord.ExceptionCode == EXCEPTION_BREAKPOINT && initial {
                            initial = false;
                        } else if info.ExceptionRecord.ExceptionCode == EXCEPTION_SINGLE_STEP {
                            let saved = *probes.get(&event.dwThreadId).expect("owned breakpoint");
                            let rearm = samples.is_empty();
                            samples.push(
                                take_probe(process, event.dwThreadId, saved, rearm)
                                    .expect("capture request")
                                    .expect("matching breakpoint"),
                            );
                            if !rearm {
                                probes.remove(&event.dwThreadId);
                            }
                        } else {
                            disposition = DBG_EXCEPTION_NOT_HANDLED;
                        }
                    }
                    EXIT_PROCESS_DEBUG_EVENT => {
                        exit = Some(unsafe { event.u.ExitProcess.dwExitCode });
                    }
                    _ => {}
                }
                assert_ne!(
                    unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, disposition) },
                    0
                );
                if exit.is_some() {
                    break;
                }
            }
            if exit.is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
            assert_eq!(exit, Some(0), "Synthetic debug child must exit normally");
            assert_eq!(
                samples.len(),
                2,
                "Rearming must capture a second call without a loop"
            );
            let captured = &samples[0];
            assert_eq!(captured.request_pointer_1_text, "SyntheticIpmiService");
            assert_eq!(captured.initial_result_hex, "ff ff ff ff");
            assert_eq!(
                captured.request_pointer_2_hex.split_whitespace().count(),
                crate::ipmi::CREATE_DESCRIPTOR_BYTES
            );
            assert!(captured
                .request_pointer_2_hex
                .split_whitespace()
                .all(|v| v == "5a"));
        }
    }
}
fn main() {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.len() == 2 && args[0] == "--diagnose-log" {
            use std::io::Read;
            let result = (|| -> Result<String, String> {
                let mut log = String::new();
                std::fs::File::open(&args[1])
                    .map_err(|e| e.to_string())?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_string(&mut log)
                    .map_err(|e| e.to_string())?;
                if log.len() > 16 * 1024 * 1024 {
                    return Err("Log exceeds 16 MiB diagnostic bound".into());
                }
                serde_json::to_string_pretty(&diagnostics::analyze(&log)).map_err(|e| e.to_string())
            })();
            match result {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        if !(2..=4).contains(&args.len()) || (args.len() == 4 && args[3] != "--ipmi-probe") {
            eprintln!("Usage: PS4Trace PREPARED_SANDBOX VERIFIED_BACKEND_EXE [TIMEOUT_SECONDS=20, max 60] [--ipmi-probe]");
            std::process::exit(2)
        }
        let timeout = match args.get(2).map(|value| value.parse::<u64>()) {
            None => 20,
            Some(Ok(value)) => value,
            Some(Err(_)) => {
                eprintln!("Timeout must be an integer from 1 to 60 seconds");
                std::process::exit(2)
            }
        };
        if let Err(e) = debugger::run(
            std::path::Path::new(&args[0]),
            std::path::Path::new(&args[1]),
            timeout,
            args.get(3).is_some(),
        ) {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        eprintln!("PS4Trace requires Windows x64");
        std::process::exit(2)
    }
}
