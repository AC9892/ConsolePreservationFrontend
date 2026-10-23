//! Narrow native IPMI component test. Not a firmware loader or a system backend.
mod ipmi;
mod ipmi_channel;
mod ipmi_connect;
mod ipmi_kernel;
mod ipmi_rpc;
mod ipmi_server;
#[cfg(all(windows, target_arch = "x86_64"))]
mod native {
    use super::ipmi;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{
        cell::{Cell, RefCell, UnsafeCell},
        collections::BTreeMap,
        fs,
        io::Read,
        path::Path,
    };
    use windows_sys::Win32::System::{
        Diagnostics::Debug::FlushInstructionCache, Memory::*, Threading::GetCurrentProcess,
    };
    const HASH: &str = "c9081d79493740a55fc3d40e160e906c17879abeca47a9315188656113853ae5";
    static GUARD: u64 = 0x5a72183c992db467;
    static NEED_LIBC: u64 = 0;
    static MUTEXES: std::sync::Mutex<BTreeMap<usize, (u32, u32)>> =
        std::sync::Mutex::new(BTreeMap::new());
    struct SharedKernel(std::sync::OnceLock<std::sync::Mutex<crate::ipmi_kernel::Kernel>>);
    impl SharedKernel {
        fn with<R>(&self, function: impl FnOnce(&Self) -> R) -> R {
            function(self)
        }
        fn borrow(&self) -> std::sync::MutexGuard<'_, crate::ipmi_kernel::Kernel> {
            self.0
                .get_or_init(|| std::sync::Mutex::new(crate::ipmi_kernel::Kernel::new(4096)))
                .lock()
                .unwrap()
        }
        fn borrow_mut(&self) -> std::sync::MutexGuard<'_, crate::ipmi_kernel::Kernel> {
            self.borrow()
        }
    }
    static CLIENTS: SharedKernel = SharedKernel(std::sync::OnceLock::new());
    static CHANNEL: std::sync::OnceLock<crate::ipmi_channel::Channel> = std::sync::OnceLock::new();
    fn channel() -> &'static crate::ipmi_channel::Channel {
        CHANNEL.get_or_init(Default::default)
    }
    const THREAD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
    static RPC: std::sync::OnceLock<crate::ipmi_rpc::Rpc> = std::sync::OnceLock::new();
    fn rpc() -> &'static crate::ipmi_rpc::Rpc {
        RPC.get_or_init(Default::default)
    }
    thread_local! {
        // Opt-in synchronous dispatch probe; never represents a completed connection.
        static DISPATCH: RefCell<Option<(usize, usize, u32, usize)>> = const { RefCell::new(None) };
        static CONNECT_PACKET: RefCell<Option<(u32, Vec<u8>)>> = const { RefCell::new(None) };
        static SESSION_EXPORTS: RefCell<Option<(usize, usize)>> = const { RefCell::new(None) };
        static DELIVERED_CLIENT: RefCell<Option<(u32, u32)>> = const { RefCell::new(None) };
        static HANDSHAKE: Cell<bool> = const { Cell::new(false) };
        static RETAINED_SESSION: RefCell<Option<(Block, u32)>> = const { RefCell::new(None) };
        static COMPLETION: Cell<Option<(u32, i32)>> = const { Cell::new(None) };
        static ERRNO: UnsafeCell<i32> = const { UnsafeCell::new(0) };
        static THREADED: Cell<bool> = const { Cell::new(false) };
        static REQUEST_PROBE: Cell<bool> = const { Cell::new(false) };
        static MULTIBUFFER_PROBE: Cell<bool> = const { Cell::new(false) };
        static RPC_ENABLED: Cell<bool> = const { Cell::new(false) };
        static RPC_ERROR: Cell<bool> = const { Cell::new(false) };
        static RPC_ACTIVE: Cell<Option<u32>> = const { Cell::new(None) };
        static RESPOND_EXPORT: Cell<usize> = const { Cell::new(0) };
        static GENERIC_RESPOND_EXPORT: Cell<usize> = const { Cell::new(0) };
        static GENERIC_DELIVERY: Cell<bool> = const { Cell::new(false) };
        static REGION: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
        static IMPORTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    unsafe extern "sysv64" fn error_address() -> *mut i32 {
        ERRNO.with(|e| e.get())
    }
    unsafe extern "sysv64" fn current_thread_id() -> i32 {
        let id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        if id == 0 || id > i32::MAX as u32 {
            stop("Host thread identifier is outside component ABI range");
        }
        id as i32
    }
    unsafe extern "sysv64" fn unsupported_auth_info(pid: i32, output: usize) -> i32 {
        if !RPC_ENABLED.get() || pid != 1 || !can_access(output, 136, true) {
            stop("Authorization query is outside the synthetic RPC process contract");
        }
        ERRNO.with(|e| unsafe {
            *e.get() = 78;
        });
        event(
            json!({"event":"synthetic-process-authinfo-unavailable","pid":pid,"posix_errno":78,
            "return_value":-1,"authorization_data_written":false}),
        );
        -1
    }
    unsafe extern "sysv64" fn kernel_error(error: i32) -> i32 {
        if !(0..=255).contains(&error) {
            stop("Unsupported errno conversion in component bridge");
        }
        if error == 0 {
            0
        } else {
            (0x80020000u32 + error as u32) as i32
        }
    }
    fn event(value: serde_json::Value) {
        println!("{value}");
    }
    fn stop(reason: &str) -> ! {
        event(json!({"event":"failure","reason":reason}));
        std::process::exit(86)
    }
    unsafe extern "sysv64" fn unsupported(index: usize) -> ! {
        let name = IMPORTS.with(|names| names.borrow().get(index).cloned().unwrap_or_default());
        stop(&format!("Unimplemented native import: {name}"))
    }
    fn allowed(address: usize, size: usize) -> bool {
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        if REGION.with(|r| {
            r.borrow()
                .iter()
                .any(|(start, len)| address >= *start && end <= start + len)
        }) {
            return true;
        }
        // The native wrapper builds its request and descriptor on this worker's
        // stack. No arbitrary host heap or process-wide mapped memory is accepted.
        let mut low = 0;
        let mut high = 0;
        unsafe {
            windows_sys::Win32::System::Threading::GetCurrentThreadStackLimits(&mut low, &mut high);
        }
        address >= low && end <= high
    }
    fn read(address: usize, size: usize) -> Vec<u8> {
        if !can_access(address, size, false) {
            stop("Native import requested memory outside owned regions/worker stack");
        }
        unsafe { std::slice::from_raw_parts(address as *const u8, size).to_vec() }
    }
    fn can_access(address: usize, size: usize, write: bool) -> bool {
        if !allowed(address, size) {
            return false;
        }
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        let mut at = address;
        while at < end {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            if unsafe {
                VirtualQuery(
                    at as *const _,
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            } == 0
                || info.State != MEM_COMMIT
                || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
            {
                return false;
            }
            if write
                && ![
                    PAGE_READWRITE,
                    PAGE_WRITECOPY,
                    PAGE_EXECUTE_READWRITE,
                    PAGE_EXECUTE_WRITECOPY,
                ]
                .contains(&info.Protect)
            {
                return false;
            }
            let Some(next) = (info.BaseAddress as usize).checked_add(info.RegionSize) else {
                return false;
            };
            if next <= at {
                return false;
            }
            at = next;
        }
        true
    }
    fn string(address: usize) -> String {
        let mut bytes = vec![];
        for offset in 0..25 {
            let b = read(
                address
                    .checked_add(offset)
                    .unwrap_or_else(|| stop("String address overflow")),
                1,
            )[0];
            if b == 0 {
                return String::from_utf8(bytes)
                    .unwrap_or_else(|_| stop("Service name is not UTF-8"));
            }
            bytes.push(b);
        }
        stop("Service name exceeds the component test's 24-byte bound")
    }
    unsafe extern "sysv64" fn compare(left: usize, right: usize, size: usize) -> i32 {
        if size > 256 {
            stop("strncmp length exceeds component bound");
        }
        for offset in 0..size {
            let a = read(left + offset, 1)[0];
            let b = read(right + offset, 1)[0];
            if a != b {
                return a as i32 - b as i32;
            }
            if a == 0 {
                return 0;
            }
        }
        0
    }
    unsafe extern "sysv64" fn format_name(
        dest: usize,
        size: usize,
        format: usize,
        index: i32,
        name: usize,
    ) -> i32 {
        if string(format) != "SceIpmi%02d_%s" {
            stop("Unsupported snprintf format in native component");
        }
        let value = format!("SceIpmi{index:02}_{}", string(name));
        if size > 1024 {
            stop("snprintf destination exceeds component bound");
        }
        if size > 0 {
            if !can_access(dest, size, true) {
                stop("snprintf output is not writable");
            }
            let count = value.len().min(size - 1);
            unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), dest as *mut u8, count);
                (dest as *mut u8).add(count).write(0);
            }
        }
        value.len() as i32
    }
    unsafe extern "sysv64" fn mutex_init(output: usize, attributes: usize, name: usize) -> i32 {
        use windows_sys::Win32::System::Threading::CreateMutexW;
        if attributes != 0 || !can_access(output, 8, true) {
            stop("Unsupported native mutex attributes/output");
        }
        let title = string(name);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, std::ptr::null()) };
        if handle.is_null() {
            stop("Windows mutex creation failed");
        }
        MUTEXES.lock().unwrap().insert(handle as usize, (0, 0));
        unsafe {
            (output as *mut usize).write_unaligned(handle as usize);
        }
        event(json!({"event":"native-mutex-initialized","name":title}));
        0
    }
    fn mutex_handle(address: usize) -> usize {
        usize::from_le_bytes(read(address, 8).try_into().unwrap())
    }
    unsafe extern "sysv64" fn mutex_lock(address: usize) -> i32 {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{GetCurrentThreadId, WaitForSingleObject, INFINITE},
        };
        let id = mutex_handle(address);
        let thread = unsafe { GetCurrentThreadId() };
        {
            let mut table = MUTEXES.lock().unwrap();
            let entry = table
                .get_mut(&id)
                .unwrap_or_else(|| stop("Unknown native mutex"));
            if entry.0 == thread {
                stop("Recursive native mutex behavior is not implemented");
            }
            entry.1 += 1;
        }
        if unsafe { WaitForSingleObject(id as _, INFINITE) } != WAIT_OBJECT_0 {
            stop("Native mutex wait failed");
        }
        let mut table = MUTEXES.lock().unwrap();
        let entry = table.get_mut(&id).unwrap();
        entry.1 -= 1;
        entry.0 = thread;
        0
    }
    unsafe extern "sysv64" fn mutex_unlock(address: usize) -> i32 {
        use windows_sys::Win32::System::Threading::{GetCurrentThreadId, ReleaseMutex};
        let id = mutex_handle(address);
        let mut table = MUTEXES.lock().unwrap();
        let entry = table
            .get_mut(&id)
            .unwrap_or_else(|| stop("Unknown native mutex"));
        if entry.0 != unsafe { GetCurrentThreadId() } {
            stop("Native mutex unlock owner mismatch");
        }
        if unsafe { ReleaseMutex(id as _) } == 0 {
            stop("Native mutex release failed");
        }
        entry.0 = 0;
        0
    }
    unsafe extern "sysv64" fn mutex_destroy(address: usize) -> i32 {
        let id = mutex_handle(address);
        let mut table = MUTEXES.lock().unwrap();
        if table.get(&id) != Some(&(0, 0)) || !can_access(address, 8, true) {
            stop("Native mutex is unknown or busy");
        }
        if unsafe { windows_sys::Win32::Foundation::CloseHandle(id as _) } == 0 {
            stop("Native mutex close failed");
        }
        table.remove(&id);
        unsafe {
            (address as *mut usize).write_unaligned(0);
        }
        event(json!({"event":"native-mutex-destroyed"}));
        0
    }
    unsafe extern "sysv64" fn transport(
        op: u32,
        kid: u32,
        output: usize,
        params: usize,
        bytes: usize,
        sentinel: u64,
    ) -> i32 {
        if op == 0x232 && RPC_ENABLED.get() {
            if bytes != 24 || RPC_ACTIVE.get() != Some(kid) || !can_access(output, 4, true) {
                stop("RPC response does not match the active dispatch session");
            }
            let args = read(params, 24);
            let status = i32::from_le_bytes(args[..4].try_into().unwrap());
            let count = u32::from_le_bytes(args[4..8].try_into().unwrap());
            let buffers = usize::from_le_bytes(args[8..16].try_into().unwrap());
            let flags = u32::from_le_bytes(args[16..20].try_into().unwrap());
            if flags == 1 && GENERIC_RESPOND_EXPORT.get() != 0 {
                if count != 1 || status != if RPC_ERROR.get() { -123 } else { 0 } {
                    stop("Generic reply capture count/status mismatch");
                }
                let descriptor = read(buffers, 24);
                let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
                let capacity = usize::from_le_bytes(descriptor[8..16].try_into().unwrap());
                let length = usize::from_le_bytes(descriptor[16..24].try_into().unwrap());
                if capacity != 4
                    || length != if RPC_ERROR.get() { 0 } else { 4 }
                    || (!RPC_ERROR.get() && read(pointer, length) != 0x12345678u32.to_le_bytes())
                {
                    stop("Generic reply capture descriptor mismatch");
                }
                event(
                    json!({"event":"native-generic-reply-captured","operation":op,"parameter_bytes":bytes,
                    "flags":flags,"count":count,"method_status":status,"capacity":capacity,"length":length,
                    "exact_payload_match":true,"client_outputs_written":false,"transport_result_written":false,
                    "original_shell_service":false,"authentic_dashboard_verified":false}),
                );
                if !GENERIC_DELIVERY.get() {
                    stop("Generic native reply captured; delivery remains unsupported");
                }
            }
            if count != 1 || flags != u32::from(GENERIC_DELIVERY.get()) {
                stop("RPC probe supports one native response buffer only");
            }
            let descriptor = read(buffers, 24);
            let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
            let capacity = usize::from_le_bytes(descriptor[8..16].try_into().unwrap());
            let length = usize::from_le_bytes(descriptor[16..24].try_into().unwrap());
            if length > capacity || capacity > 4096 {
                stop("Native RPC response exceeds its buffer bounds");
            }
            let response = crate::ipmi_rpc::Response {
                status,
                bytes: read(pointer, length),
            };
            rpc()
                .respond(
                    kid,
                    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
                    response,
                )
                .unwrap_or_else(|e| stop(e));
            unsafe {
                (output as *mut u32).write_unaligned(0);
            }
            event(
                json!({"event":"native-sync-response-accepted","session_handle":kid,"method_status":status,
                "response_bytes":length,"synthetic_test_endpoint":true}),
            );
            return 0;
        }
        if op == 0x212 && HANDSHAKE.get() {
            if bytes != 4 || !can_access(output, 4, true) {
                stop("Invalid connection-completion arguments");
            }
            let status = i32::from_le_bytes(read(params, 4).try_into().unwrap());
            let (client, thread) = DELIVERED_CLIENT
                .with(|d| *d.borrow())
                .unwrap_or_else(|| stop("Completion requires delivered request"));
            let session = RETAINED_SESSION
                .with(|s| s.borrow().as_ref().map(|(_, id)| *id))
                .unwrap_or_else(|| stop("Completion requires a verified native session"));
            if status != 0
                || kid != client
                || COMPLETION.get().is_some()
                || thread != unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
                || !CLIENTS.with(|k| {
                    k.borrow()
                        .session(1, session)
                        .is_ok_and(|s| s.client == client)
                })
            {
                stop("Completion does not match the active native session");
            }
            COMPLETION.set(Some((client, status)));
            if THREADED.get() {
                channel()
                    .complete(client, status)
                    .unwrap_or_else(|e| stop(e));
            }
            unsafe {
                (output as *mut u32).write_unaligned(0);
            }
            event(
                json!({"event":"native-connection-completion-accepted","client_handle":client,"session_handle":session,
                "callback_status":status,"synthetic_test_endpoint":true}),
            );
            return 0;
        }
        if op == 0x320 && REQUEST_PROBE.get() {
            let request = crate::ipmi_connect::decode_sync_call(&read(params, bytes))
                .unwrap_or_else(|e| stop(&e));
            if COMPLETION.get() != Some((kid, 0))
                || !can_access(output, 4, true)
                || !can_access(request.method_result as usize, 4, true)
            {
                stop("Synchronous method requires this client's verified connection and writable outputs");
            }
            let mut input_bytes = Vec::new();
            let mut payloads = Vec::new();
            for index in 0..request.input_count as usize {
                let descriptor = read(request.inputs as usize + index * 16, 16);
                let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
                let bytes = usize::from_le_bytes(descriptor[8..16].try_into().unwrap());
                if bytes > 4096 || (bytes != 0 && !can_access(pointer, bytes, false)) {
                    stop("Sync input is outside probe memory bounds");
                }
                input_bytes.push(bytes);
                payloads.push(if bytes == 0 {
                    vec![]
                } else {
                    read(pointer, bytes)
                });
            }
            let mut output_capacities = Vec::new();
            let mut targets = Vec::new();
            for index in 0..request.output_count as usize {
                let descriptor = read(request.outputs as usize + index * 24, 24);
                let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
                let capacity = usize::from_le_bytes(descriptor[8..16].try_into().unwrap());
                if capacity > 4096 || (capacity != 0 && !can_access(pointer, capacity, true)) {
                    stop("Sync output is outside probe memory bounds");
                }
                output_capacities.push(capacity);
                if !can_access(request.outputs as usize + index * 24, 24, true) {
                    stop("RPC output descriptor is not writable");
                }
                targets.push((request.outputs as usize + index * 24, pointer, capacity));
            }
            event(
                json!({"event":"native-sync-request-decoded","request":request,"input_bytes":input_bytes,
                "output_capacities":output_capacities,"method_result_written":false,"response_written":false,
                "synthetic_test_endpoint":true,"original_shell_service":false}),
            );
            if RPC_ENABLED.get() {
                if !(if MULTIBUFFER_PROBE.get() {
                    request.input_count == 3
                        && request.output_count == 1
                        && request.flags == 1
                        && request.method == 0x03299000
                        && input_bytes == [8, 16, 100]
                        && output_capacities == [4]
                } else {
                    request.input_count == 1
                        && request.output_count == 1
                        && request.flags == 0
                        && request.method == 0
                }) {
                    stop("RPC probe only supports test method zero and one input/output");
                }
                let (session, implementation) = CLIENTS.with(|k| {
                    let kernel = k.borrow();
                    let (handle, session) = kernel
                        .client_session(1, kid)
                        .unwrap_or_else(|e| stop(&format!("RPC session lookup failed: {e:?}")));
                    (handle, session.implementation)
                });
                let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                rpc()
                    .enqueue(crate::ipmi_rpc::Request {
                        client: kid,
                        session,
                        caller_thread: thread,
                        session_impl: implementation,
                        method: request.method,
                        input: if MULTIBUFFER_PROBE.get() {
                            vec![]
                        } else {
                            payloads.remove(0)
                        },
                        generic_inputs: if MULTIBUFFER_PROBE.get() {
                            Some(payloads)
                        } else {
                            None
                        },
                        capacity: output_capacities[0],
                    })
                    .unwrap_or_else(|e| stop(e));
                let response = rpc()
                    .wait_response(kid, thread, THREAD_WAIT)
                    .unwrap_or_else(|e| stop(e));
                let (descriptor, pointer, capacity) = targets[0];
                if response.bytes.len() > capacity {
                    stop("RPC response exceeds original client capacity");
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        response.bytes.as_ptr(),
                        pointer as *mut u8,
                        response.bytes.len(),
                    );
                    ((descriptor + 16) as *mut usize).write_unaligned(response.bytes.len());
                    (request.method_result as *mut i32).write_unaligned(response.status);
                    (output as *mut u32).write_unaligned(0);
                }
                event(
                    json!({"event":"native-sync-client-outputs-written","response_bytes":response.bytes.len(),
                    "method_status":response.status,"host_thread":thread,"synthetic_test_endpoint":true}),
                );
                return 0;
            }
        }
        if op == 4 && SESSION_EXPORTS.with(|e| e.borrow().is_some()) {
            event(
                json!({"event":"native-session-create-request","kid":kid,"parameter_bytes":bytes,"result_writable":can_access(output,4,true)}),
            );
            if kid != u32::MAX || bytes != 16 || !can_access(output, 4, true) {
                stop("Invalid session-create request");
            }
            let args = read(params, 16);
            let implementation = u64::from_le_bytes(args[..8].try_into().unwrap());
            let descriptor = usize::from_le_bytes(args[8..16].try_into().unwrap());
            let data = read(descriptor, 16);
            let size = u64::from_le_bytes(data[..8].try_into().unwrap());
            let user_data = u64::from_le_bytes(data[8..16].try_into().unwrap());
            let (client, thread) = DELIVERED_CLIENT.with(|d| *d.borrow()).unwrap_or_else(|| {
                stop("Session creation requires a delivered connection request")
            });
            if size != 16
                || user_data != 0
                || !can_access(implementation as usize, 16, true)
                || thread != unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
            {
                stop("Session descriptor or dispatch thread does not match the probe contract");
            }
            let server = DISPATCH.with(|d| d.borrow().unwrap().2);
            let handle = CLIENTS
                .with(|k| {
                    k.borrow_mut()
                        .create_session(1, server, client, implementation, user_data)
                })
                .unwrap_or_else(|e| stop(&format!("Session registration rejected: {e:?}")));
            unsafe {
                (output as *mut u32).write_unaligned(handle);
            }
            event(
                json!({"event":"ipmi-session-registered","handle":handle,"client_handle":client,
                "server_handle":server,"synthetic_test_endpoint":true,"connection_completed":false}),
            );
            return 0;
        }
        if op == 5 && SESSION_EXPORTS.with(|e| e.borrow().is_some()) {
            if bytes != 0 || params != 0 || !can_access(output, 4, true) {
                stop("Invalid session-destroy request");
            }
            CLIENTS
                .with(|k| k.borrow_mut().destroy_session(1, kid))
                .unwrap_or_else(|e| stop(&format!("Session destruction rejected: {e:?}")));
            unsafe {
                (output as *mut u32).write_unaligned(0);
            }
            event(json!({"event":"ipmi-session-destroyed","handle":kid}));
            return 0;
        }
        if op == 0x201 && DISPATCH.with(|d| d.borrow().is_some()) {
            if bytes != 32 || !can_access(output, 4, true) || !can_access(params, 32, true) {
                stop("Invalid server receive argument block");
            }
            let context = DISPATCH.with(|d| d.borrow().unwrap());
            if kid != context.2 {
                stop("Receive requested a different test server");
            }
            let args = read(params, 32);
            let word = |at| usize::from_le_bytes(args[at..at + 8].try_into().unwrap());
            let (buffer, capacity, info) = (word(0), word(8), word(16));
            let connection = if THREADED.get() {
                channel().receive()
            } else {
                CONNECT_PACKET.with(|p| p.borrow_mut().take())
            };
            let queued = connection
                .map(|(client, packet)| (client, packet, 1u32, 0u64))
                .or_else(|| {
                    if !RPC_ENABLED.get() {
                        return None;
                    }
                    let thread =
                        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
                    rpc().receive(thread).map(|request| {
                        let valid = CLIENTS.with(|k| {
                            k.borrow().session(1, request.session).is_ok_and(|s| {
                                s.client == request.client
                                    && s.server == kid
                                    && s.implementation == request.session_impl
                            })
                        });
                        if !valid {
                            stop("Queued RPC no longer matches its live native session");
                        }
                        RPC_ACTIVE.set(Some(request.session));
                        let packet = request.packet();
                        let input_size = packet.len() as u64 - 4;
                        let packet_type = if request.generic_inputs.is_some() {
                            0x8041
                        } else {
                            0x8051
                        };
                        (request.client, packet, packet_type, input_size)
                    })
                });
            if queued.is_none() && HANDSHAKE.get() && COMPLETION.get().is_some() && word(24) != 0 {
                if !can_access(word(24), 4, false) {
                    stop("Invalid nonblocking receive flag");
                }
                ERRNO.with(|e| unsafe {
                    *e.get() = 35;
                });
                event(
                    json!({"event":"native-server-receive-empty","posix_errno":35,"transport_return":-1}),
                );
                return -1;
            }
            let (client, packet, kind, input_size) = queued.unwrap_or_else(|| {
                stop("No queued connection packet; empty receive is not implemented")
            });
            if capacity < packet.len()
                || !can_access(buffer, packet.len(), true)
                || !can_access(info, 24, true)
            {
                stop("Receive output cannot hold the queued connection packet");
            }
            let mut metadata = [0u8; 24];
            metadata[..8].copy_from_slice(&input_size.to_le_bytes());
            metadata[8..12].copy_from_slice(&kind.to_le_bytes());
            metadata[12..16].copy_from_slice(&client.to_le_bytes());
            metadata[16..24].copy_from_slice(&context.3.to_le_bytes());
            unsafe {
                std::ptr::copy_nonoverlapping(packet.as_ptr(), buffer as *mut u8, packet.len());
                std::ptr::copy_nonoverlapping(metadata.as_ptr(), info as *mut u8, metadata.len());
                ((params + 8) as *mut usize).write_unaligned(packet.len());
                (output as *mut u32).write_unaligned(0);
            }
            event(
                json!({"event":if kind==1 {"native-server-received-connection-packet"}else{"native-server-received-sync-packet"},"client_handle":client,
                "server_handle":kid,"packet_bytes":packet.len(),"packet_type":kind,"input_size":input_size,"synthetic_test_endpoint":true}),
            );
            DELIVERED_CLIENT.with(|d| {
                *d.borrow_mut() = Some((client, unsafe {
                    windows_sys::Win32::System::Threading::GetCurrentThreadId()
                }))
            });
            return 0;
        }
        if op == 0 {
            if kid != 0 || bytes != 24 || !can_access(output, 4, true) {
                stop("Invalid server-create request");
            }
            let request =
                ipmi::decode_create_request(&read(params, bytes)).unwrap_or_else(|e| stop(&e));
            let descriptor =
                crate::ipmi_server::decode_server_descriptor(&read(request.config as usize, 56))
                    .unwrap_or_else(|e| stop(&e));
            if !can_access(request.client_impl as usize, 16, false)
                || !can_access(descriptor.event_handler as usize, 8, false)
            {
                stop("Server implementation/handler is outside owned memory");
            }
            let service = string(request.service_name as usize);
            let id = CLIENTS
                .with(|kernel| {
                    kernel.borrow_mut().create_server(
                        1,
                        request.client_impl,
                        service.clone(),
                        descriptor,
                    )
                })
                .unwrap_or_else(|e| stop(&format!("Server registration rejected: {e:?}")));
            unsafe {
                (output as *mut u32).write_unaligned(id);
            }
            let server = CLIENTS.with(|kernel| {
                serde_json::to_value(kernel.borrow().server(1, id).unwrap()).unwrap()
            });
            event(
                json!({"event":"ipmi-server-registered","handle":id,"service":service,"server":server,"synthetic_test_endpoint":true,"original_shell_service":false}),
            );
            return 0;
        }
        if op == 1 {
            if bytes != 0 || params != 0 || !can_access(output, 4, true) {
                stop("Invalid server-destroy request");
            }
            CLIENTS
                .with(|kernel| kernel.borrow_mut().destroy_server(1, kid))
                .unwrap_or_else(|e| stop(&format!("Server destruction rejected: {e:?}")));
            unsafe {
                (output as *mut u32).write_unaligned(0);
            }
            event(json!({"event":"ipmi-server-destroyed","handle":kid}));
            return 0;
        }
        if op == 3 {
            if bytes != 0 || params != 0 || !can_access(output, 4, true) {
                stop("Invalid destroy-client arguments");
            }
            let removed = CLIENTS.with(|clients| clients.borrow_mut().destroy(1, kid));
            if removed.is_err() {
                stop("Destroy requested an unknown client handle");
            }
            unsafe {
                (output as *mut u32).write_unaligned(0);
            }
            event(json!({"event":"ipmi-client-destroyed","handle":kid}));
            return 0;
        }
        if op != 2 {
            if op == 0x400 {
                if bytes != 32 || !can_access(output, 4, true) {
                    stop("Invalid native connection request");
                }
                let request = crate::ipmi_connect::decode_connect_request(&read(params, bytes))
                    .unwrap_or_else(|e| stop(&e));
                if request.user_data_bytes > 4096
                    || (request.user_data_bytes != 0
                        && !can_access(
                            request.user_data as usize,
                            request.user_data_bytes as usize,
                            false,
                        ))
                    || !can_access(request.callback_status as usize, 4, true)
                    || (request.retry_flag != 0
                        && !can_access(request.retry_flag as usize, 4, false))
                {
                    stop("Connection payload or output is outside the bounded memory contract");
                }
                event(
                    json!({"event":"native-connection-request","request":request,
                    "payload_bytes_validated":request.user_data_bytes,"callback_status_writable":true,
                    "transport_result_written":false,"callback_status_written":false,
                    "missing_protocol_operations":if SESSION_EXPORTS.with(|e|e.borrow().is_some()) {
                        vec!["0x212 connection completion"]
                    } else if DISPATCH.with(|d|d.borrow().is_some()) {
                        vec!["0x4 session create","0x212 connection completion"]
                    } else { vec!["0x201 server receive","0x4 session create","0x212 connection completion"] }}),
                );
                let server = CLIENTS.with(|kernel| kernel.borrow().resolve_server(1, kid));
                event(
                    json!({"event":"native-connection-server-lookup","client_handle":kid,"server_handle":server.as_ref().ok(),"lookup_error":server.as_ref().err().map(|e|format!("{e:?}")),"session_created":false}),
                );
                if let Some((entry, object, server_id, _)) = DISPATCH.with(|d| *d.borrow()) {
                    if server != Ok(server_id) || request.user_data_bytes != 0 {
                        stop("Dispatch probe requires its registered server and an empty payload");
                    }
                    let packet = CLIENTS.with(|k| {
                        let kernel = k.borrow();
                        let descriptor = &kernel.client(1, kid).unwrap().descriptor;
                        // Queue and event semantics are deliberately excluded from this probe.
                        if descriptor.field_38 != 0
                            || descriptor.rounded_fields_40_138.iter().any(|v| *v != 0)
                        {
                            stop("Dispatch probe does not implement event flags or message queues");
                        }
                        let mut packet = vec![0u8; 0x164];
                        packet[..4].copy_from_slice(&1u32.to_le_bytes());
                        packet[4..8].copy_from_slice(&kid.to_le_bytes());
                        packet[8..12].copy_from_slice(&descriptor.field_08.to_le_bytes());
                        packet[0x10..0x18]
                            .copy_from_slice(&descriptor.fields_10_18[0].to_le_bytes());
                        packet[0x18..0x20]
                            .copy_from_slice(&descriptor.fields_10_18[1].to_le_bytes());
                        packet[0x20..0x24].copy_from_slice(&descriptor.field_20.to_le_bytes());
                        packet[0x28..0x30]
                            .copy_from_slice(&descriptor.fields_28_30[0].to_le_bytes());
                        packet[0x30..0x38]
                            .copy_from_slice(&descriptor.fields_28_30[1].to_le_bytes());
                        packet
                    });
                    if THREADED.get() {
                        channel().enqueue(kid, packet).unwrap_or_else(|e| stop(e));
                        event(
                            json!({"event":"native-client-waiting-for-server","client_handle":kid,
                            "host_thread":unsafe{windows_sys::Win32::System::Threading::GetCurrentThreadId()}}),
                        );
                        let status = channel()
                            .wait_completion(kid, THREAD_WAIT)
                            .unwrap_or_else(|e| stop(e));
                        COMPLETION.set(Some((kid, status)));
                        unsafe {
                            (request.callback_status as *mut i32).write_unaligned(status);
                            (output as *mut u32).write_unaligned(0);
                        }
                        event(
                            json!({"event":"ipmi-connect-results-written","client_handle":kid,"callback_status":status,
                            "concurrent_dispatch_verified":true,"authentic_dashboard_verified":false}),
                        );
                        return 0;
                    }
                    CONNECT_PACKET.with(|p| *p.borrow_mut() = Some((kid, packet)));
                    let buffer = Block::new(16384).unwrap_or_else(|e| stop(&e));
                    event(
                        json!({"event":"calling-native-server-try-dispatch","synchronous_probe":true,
                        "concurrent_dispatch_verified":false,"connection_completed":false}),
                    );
                    let dispatch: unsafe extern "sysv64" fn(usize, usize, usize) -> i32 =
                        unsafe { std::mem::transmute(entry) };
                    let status = unsafe { dispatch(object, buffer.address as usize, buffer.size) };
                    event(json!({"event":"native-server-try-dispatch-returned","status":status}));
                    if HANDSHAKE.get() {
                        let (client, callback_status) = COMPLETION.get().unwrap_or_else(|| {
                            stop("No native connection completion was observed")
                        });
                        if status != 0 || client != kid {
                            stop("Dispatch did not complete this client connection");
                        }
                        unsafe {
                            (request.callback_status as *mut i32).write_unaligned(callback_status);
                            (output as *mut u32).write_unaligned(0);
                        }
                        event(
                            json!({"event":"ipmi-connect-results-written","client_handle":client,"callback_status":callback_status,
                            "native_session_verified":true,"authentic_dashboard_verified":false}),
                        );
                        return 0;
                    }
                    stop("Dispatch returned without a verified session; connection remains unsupported");
                }
            }
            event(
                json!({"event":"unsupported-ipmi-operation","operation":op,"kid":kid,"parameter_bytes":bytes,
                "parameters_hex":if bytes<=256 {Some(read(params,bytes).iter().map(|b|format!("{b:02x}")).collect::<Vec<_>>().join(" "))}else{None}}),
            );
            stop(&format!(
                "Unsupported IPMI operation {op:#x}; no success response fabricated"
            ));
        }
        if kid != 0 || bytes != 24 || !can_access(output, 4, true) {
            stop("Invalid client-create transport arguments");
        }
        let request =
            ipmi::decode_create_request(&read(params, bytes)).unwrap_or_else(|e| stop(&e));
        let descriptor = ipmi::decode_create_descriptor(&read(
            request.config as usize,
            ipmi::CREATE_DESCRIPTOR_BYTES,
        ))
        .unwrap_or_else(|e| stop(&e));
        if !descriptor.diagnostics.is_empty() {
            stop("Descriptor does not match the recovered native ABI");
        }
        let name = string(request.service_name as usize);
        if !allowed(request.client_impl as usize, 16) {
            stop("Client object is not in an owned region");
        }
        let id = CLIENTS
            .with(|clients| {
                clients
                    .borrow_mut()
                    .create(1, request.client_impl, name.clone(), descriptor)
            })
            .unwrap_or_else(|error| stop(&format!("Client registry rejected creation: {error:?}")));
        unsafe {
            (output as *mut u32).write_unaligned(id);
        }
        let client = CLIENTS
            .with(|clients| serde_json::to_value(clients.borrow().client(1, id).unwrap()).unwrap());
        event(
            json!({"event":"ipmi-client-created","operation":op,"handle":id,"service":name,
            "request":request,"client":client,"sentinel":format!("0x{sentinel:x}"),
            "connection_state":"not-connected","service_running":false}),
        );
        0
    }
    struct Block {
        address: *mut u8,
        size: usize,
    }
    impl Block {
        fn new(size: usize) -> Result<Self, String> {
            let address = unsafe {
                VirtualAlloc(
                    std::ptr::null(),
                    size,
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_READWRITE,
                )
            } as *mut u8;
            if address.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            REGION.with(|r| r.borrow_mut().push((address as usize, size)));
            Ok(Self { address, size })
        }
    }
    impl Drop for Block {
        fn drop(&mut self) {
            REGION.with(|r| r.borrow_mut().retain(|(a, _)| *a != self.address as usize));
            unsafe {
                VirtualFree(self.address as *mut _, 0, MEM_RELEASE);
            }
        }
    }
    fn number(b: &[u8], offset: usize, size: usize) -> Result<u64, String> {
        let slice = b
            .get(offset..offset.checked_add(size).ok_or("Offset overflow")?)
            .ok_or("Truncated ELF")?;
        let mut value = [0u8; 8];
        value[..size].copy_from_slice(slice);
        Ok(u64::from_le_bytes(value))
    }
    #[derive(Clone)]
    struct Symbol {
        name: String,
        value: u64,
        defined: bool,
        kind: u8,
    }
    struct Module {
        image: Block,
        exports: BTreeMap<String, usize>,
        init: usize,
    }
    struct ModuleFunctions {
        exports: BTreeMap<String, usize>,
        image_size: usize,
    }
    impl ModuleFunctions {
        fn export(&self, nid: &str) -> Result<usize, String> {
            self.exports
                .get(nid)
                .copied()
                .ok_or_else(|| format!("Required export absent: {nid}"))
        }
    }
    impl Module {
        fn load(path: &Path) -> Result<Self, String> {
            let mut b = vec![];
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut b)
                .map_err(|e| e.to_string())?;
            if b.len() > 1024 * 1024 {
                return Err("Native module exceeds test size bound".into());
            }
            if format!("{:x}", Sha256::digest(&b)) != HASH {
                return Err("Only the researched native IPMI module is accepted".into());
            }
            if !b.starts_with(b"\x7fELF\x02\x01") {
                return Err("Expected ELF64".into());
            }
            let ph = number(&b, 32, 8)? as usize;
            let stride = number(&b, 54, 2)? as usize;
            let count = number(&b, 56, 2)? as usize;
            let mut segments = vec![];
            let mut dynamic = None;
            let mut data = None;
            let mut limit = 0;
            for i in 0..count {
                let p = ph + i * stride;
                let kind = number(&b, p, 4)?;
                let offset = number(&b, p + 8, 8)? as usize;
                let va = number(&b, p + 16, 8)? as usize;
                let file = number(&b, p + 32, 8)? as usize;
                let mem = number(&b, p + 40, 8)? as usize;
                if kind == 1 || kind == 0x61000010 {
                    if file > mem || va.checked_add(mem).is_none_or(|v| v > 1024 * 1024) {
                        return Err("Unsupported module layout".into());
                    }
                    limit = limit.max(va + mem);
                    segments.push((kind, number(&b, p + 4, 4)?, offset, va, file, mem));
                }
                if kind == 2 {
                    dynamic = Some((offset, file));
                }
                if kind == 0x61000000 {
                    data = Some(offset);
                }
            }
            let tramp_start = (limit + 0xfff) & !0xfff;
            let image = Block::new(tramp_start + 0x1000)?;
            for (_, _, offset, va, file, _) in &segments {
                let source = b
                    .get(*offset..offset + file)
                    .ok_or("Segment outside file")?;
                unsafe {
                    std::ptr::copy_nonoverlapping(source.as_ptr(), image.address.add(*va), *file);
                }
            }
            let base = image.address as usize;
            let (dynamic, bytes) = dynamic.ok_or("No dynamic table")?;
            let data = data.ok_or("No SCE dynamic data")?;
            let mut tags = BTreeMap::new();
            for pos in (dynamic..dynamic + bytes).step_by(16) {
                let tag = number(&b, pos, 8)?;
                if tag == 0 {
                    break;
                }
                tags.insert(tag, number(&b, pos + 8, 8)? as usize);
            }
            let tag = |key| {
                tags.get(&key)
                    .copied()
                    .ok_or_else(|| format!("Missing ELF tag {key:#x}"))
            };
            let strings = data + tag(0x61000035)?;
            let string_end = strings + tag(0x61000037)?;
            let syms = data + tag(0x61000039)?;
            let size = tag(0x6100003f)?;
            let mut symbols = vec![];
            let mut exports = BTreeMap::new();
            for pos in (syms..syms + size).step_by(24) {
                let offset = number(&b, pos, 4)? as usize;
                let tail = b
                    .get(strings + offset..string_end)
                    .ok_or("Invalid symbol name")?;
                let end = tail
                    .iter()
                    .position(|v| *v == 0)
                    .ok_or("Unterminated symbol")?;
                let name = String::from_utf8(tail[..end].to_vec()).map_err(|e| e.to_string())?;
                let symbol = Symbol {
                    name: name.split('#').next().unwrap_or_default().into(),
                    value: number(&b, pos + 8, 8)?,
                    defined: number(&b, pos + 6, 2)? != 0,
                    kind: (number(&b, pos + 4, 1)? & 15) as u8,
                };
                if symbol.defined {
                    exports.insert(symbol.name.clone(), base + symbol.value as usize);
                }
                symbols.push(symbol);
            }
            let mut resolved = BTreeMap::new();
            let mut tramp_used = 0;
            for (address_tag, size_tag) in [(0x6100002f, 0x61000031), (0x61000029, 0x6100002d)] {
                let start = data + tag(address_tag)?;
                let size = tag(size_tag)?;
                for pos in (start..start + size).step_by(24) {
                    let address = number(&b, pos, 8)? as usize;
                    let info = number(&b, pos + 8, 8)?;
                    let addend = number(&b, pos + 16, 8)?;
                    if address.checked_add(8).is_none_or(|end| end > tramp_start) {
                        return Err("Relocation outside image".into());
                    }
                    let kind = info as u32;
                    let value = match kind {
                        8 => (base as u64).wrapping_add(addend),
                        1 | 6 | 7 => {
                            let symbol = symbols
                                .get((info >> 32) as usize)
                                .ok_or("Invalid relocation symbol")?;
                            let pointer = if symbol.defined {
                                base + symbol.value as usize
                            } else if let Some(value) = resolved.get(&symbol.name) {
                                *value
                            } else {
                                let value = match symbol.name.as_str() {
                                    "Hk7iHmGxB18" => transport as *const () as usize,
                                    "aesyjrHVWy4" => compare as *const () as usize,
                                    "eLdDw6l0-bU" => format_name as *const () as usize,
                                    "cmo1RIYva9o" => mutex_init as *const () as usize,
                                    "2Of0f+3mhhE" => mutex_destroy as *const () as usize,
                                    "7H0iTOciTLo" => mutex_lock as *const () as usize,
                                    "2Z+PpY6CaJg" => mutex_unlock as *const () as usize,
                                    "9BcDykPmo1I" => error_address as *const () as usize,
                                    "D4yla3vx4tY" => kernel_error as *const () as usize,
                                    "EI-5-jlq2dE" => current_thread_id as *const () as usize,
                                    "igMefp4SAv0" => unsupported_auth_info as *const () as usize,
                                    "f7uOxY9mM1U" => &GUARD as *const _ as usize,
                                    "ZT4ODD2Ts9o" => &NEED_LIBC as *const _ as usize,
                                    _ if symbol.kind == 1 => {
                                        return Err(format!(
                                            "Unsupported object import: {}",
                                            symbol.name
                                        ))
                                    }
                                    _ => {
                                        let index = IMPORTS.with(|names| {
                                            let mut names = names.borrow_mut();
                                            let index = names.len();
                                            names.push(symbol.name.clone());
                                            index
                                        });
                                        if tramp_used + 22 > 0x1000 {
                                            return Err("Import trampoline page exhausted".into());
                                        }
                                        let dest = base + tramp_start + tramp_used;
                                        let mut code = vec![0x48, 0xbf];
                                        code.extend_from_slice(&(index as u64).to_le_bytes());
                                        code.extend_from_slice(&[0x48, 0xb8]);
                                        code.extend_from_slice(
                                            &(unsupported as *const () as u64).to_le_bytes(),
                                        );
                                        code.extend_from_slice(&[0xff, 0xe0]);
                                        unsafe {
                                            std::ptr::copy_nonoverlapping(
                                                code.as_ptr(),
                                                dest as *mut u8,
                                                code.len(),
                                            );
                                        }
                                        tramp_used += code.len();
                                        dest
                                    }
                                };
                                resolved.insert(symbol.name.clone(), value);
                                value
                            };
                            (pointer as u64).wrapping_add(if kind == 1 { addend } else { 0 })
                        }
                        _ => return Err(format!("Unsupported relocation type {kind}")),
                    };
                    unsafe {
                        (base.wrapping_add(address) as *mut u64).write_unaligned(value);
                    }
                }
            }
            for (kind, flags, _, va, _, mem) in &segments {
                let protection = if flags & 1 != 0 {
                    PAGE_EXECUTE_READ
                } else if *kind == 0x61000010 || flags & 2 == 0 {
                    PAGE_READONLY
                } else {
                    PAGE_READWRITE
                };
                let mut old = 0;
                if unsafe {
                    VirtualProtect(
                        image.address.add(*va) as *const _,
                        *mem,
                        protection,
                        &mut old,
                    )
                } == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
            let mut old = 0;
            if unsafe {
                VirtualProtect(
                    image.address.add(tramp_start) as *const _,
                    0x1000,
                    PAGE_EXECUTE_READ,
                    &mut old,
                )
            } == 0
            {
                return Err("Cannot protect trampolines".into());
            }
            unsafe {
                FlushInstructionCache(GetCurrentProcess(), image.address as *const _, image.size);
            }
            let init = base + tags.get(&12).copied().unwrap_or_default();
            event(
                json!({"event":"module-loaded","sha256":HASH,"image_bytes":image.size,"image_base":format!("0x{base:x}"),"import_bindings":resolved.len(),"fail_stop_function_imports":tramp_used/22,"implemented_function_nids":["Hk7iHmGxB18","aesyjrHVWy4","eLdDw6l0-bU","cmo1RIYva9o","2Of0f+3mhhE","7H0iTOciTLo","2Z+PpY6CaJg","9BcDykPmo1I","D4yla3vx4tY","EI-5-jlq2dE","igMefp4SAv0"],"export_count":exports.len()}),
            );
            Ok(Self {
                image,
                exports,
                init,
            })
        }
        fn export(&self, nid: &str) -> Result<usize, String> {
            self.exports
                .get(nid)
                .copied()
                .ok_or_else(|| format!("Required export absent: {nid}"))
        }
        fn functions(&self) -> ModuleFunctions {
            ModuleFunctions {
                exports: self.exports.clone(),
                image_size: self.image.size,
            }
        }
    }
    unsafe extern "sysv64" fn unexpected_test_callback() -> ! {
        stop("Synthetic test callback invoked before session dispatch was implemented")
    }
    unsafe extern "sysv64" fn generic_test_callback(
        handler: usize,
        session: usize,
        method: u32,
        inputs: usize,
        input_count: u32,
        outputs: usize,
        output_count: u32,
    ) -> i32 {
        let context = DISPATCH
            .with(|d| *d.borrow())
            .unwrap_or_else(|| stop("Generic callback lacks context"));
        let retained = RETAINED_SESSION
            .with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|(block, id)| (block.address as usize, *id))
            })
            .unwrap_or_else(|| stop("Generic callback lacks retained session"));
        if !MULTIBUFFER_PROBE.get()
            || !RPC_ENABLED.get()
            || handler != context.3
            || session != retained.0
            || RPC_ACTIVE.get() != Some(retained.1)
            || method != 0x03299000
            || input_count != 3
            || output_count != 1
        {
            stop("Generic callback identity or counts differ from test request");
        }
        let mut payloads = Vec::new();
        for (index, expected) in [8usize, 16, 100].into_iter().enumerate() {
            let descriptor = read(inputs + index * 16, 16);
            let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
            let length = usize::from_le_bytes(descriptor[8..].try_into().unwrap());
            if length != expected {
                stop("Generic native input descriptor length mismatch");
            }
            payloads.push(read(pointer, length));
        }
        let descriptor = read(outputs, 24);
        let pointer = usize::from_le_bytes(descriptor[..8].try_into().unwrap());
        let capacity = usize::from_le_bytes(descriptor[8..16].try_into().unwrap());
        let mut expected_name = vec![0u8; 16];
        expected_name[..10].copy_from_slice(b"PS4IpcTest");
        if payloads[0] != 1u64.to_le_bytes()
            || payloads[1] != expected_name
            || payloads[2] != vec![0u8; 100]
            || capacity != 4
            || !can_access(pointer, capacity, true)
        {
            stop("Generic native callback bytes or output bounds mismatch");
        }
        event(
            json!({"event":"native-generic-dispatch-verified","method_id":method,
            "input_bytes":[8,16,100],"output_capacity":capacity,"exact_payload_match":true,
            "callback_slot":0x10,"response_written":false,"original_logger_library_executed":false,
            "original_shell_service":false,"authentic_dashboard_verified":false}),
        );
        if GENERIC_RESPOND_EXPORT.get() != 0 {
            let payload = Block::new(4).unwrap_or_else(|e| stop(&e));
            let descriptor = Block::new(24).unwrap_or_else(|e| stop(&e));
            unsafe {
                (payload.address as *mut u32).write_unaligned(0x12345678);
                (descriptor.address as *mut usize).write_unaligned(payload.address as usize);
                (descriptor.address.add(8) as *mut usize).write_unaligned(4);
                (descriptor.address.add(16) as *mut usize).write_unaligned(if RPC_ERROR.get() {
                    0
                } else {
                    4
                });
            }
            let respond: unsafe extern "sysv64" fn(usize, i32, usize, u32) -> i32 =
                unsafe { std::mem::transmute(GENERIC_RESPOND_EXPORT.get()) };
            event(
                json!({"event":"calling-native-generic-response","export_nid":"AYyxvDy-SFU",
                "descriptor_fully_initialized":true,"synthetic_test_endpoint":true}),
            );
            let status = unsafe {
                respond(
                    session,
                    if RPC_ERROR.get() { -123 } else { 0 },
                    descriptor.address as usize,
                    1,
                )
            };
            event(json!({"event":"native-generic-response-returned","status":status}));
            if !GENERIC_DELIVERY.get() || status != 0 {
                stop("Generic response returned without verified delivery");
            }
            return 0;
        }
        stop("Generic native dispatch verified; reply ABI remains unsupported");
    }
    // Eight arguments are required by the recovered native raw-buffer callback ABI.
    #[allow(clippy::too_many_arguments)]
    unsafe extern "sysv64" fn sync_test_callback(
        handler: usize,
        session: usize,
        method: u32,
        input: usize,
        length: usize,
        requested_capacity: usize,
        output: usize,
        actual_capacity: usize,
    ) -> i32 {
        let context = DISPATCH
            .with(|d| *d.borrow())
            .unwrap_or_else(|| stop("RPC callback lacks server context"));
        let native_session = RETAINED_SESSION
            .with(|s| {
                s.borrow()
                    .as_ref()
                    .map(|(block, id)| (block.address as usize, *id))
            })
            .unwrap_or_else(|| stop("RPC callback lacks retained native session"));
        if !RPC_ENABLED.get()
            || handler != context.3
            || session != native_session.0
            || RPC_ACTIVE.get() != Some(native_session.1)
            || method != 0
            || length > 4096
            || requested_capacity != 16
            || actual_capacity != 16
            || length > actual_capacity
            || !can_access(output, actual_capacity, true)
        {
            stop("Raw RPC callback does not match the bounded test request");
        }
        let mut payload = read(input, length);
        payload.reverse();
        let method_status = if RPC_ERROR.get() {
            payload.clear();
            -123
        } else {
            0
        };
        unsafe {
            std::ptr::copy_nonoverlapping(payload.as_ptr(), output as *mut u8, payload.len());
        }
        event(
            json!({"event":"native-sync-test-callback-reached","method_id":method,"input_bytes":length,
            "response_bytes":payload.len(),"synthetic_test_endpoint":true,"original_shell_service":false}),
        );
        let respond: unsafe extern "sysv64" fn(usize, i32, usize, usize) -> i32 =
            unsafe { std::mem::transmute(RESPOND_EXPORT.get()) };
        let status = unsafe { respond(session, method_status, output, payload.len()) };
        event(json!({"event":"native-sync-respond-returned","status":status}));
        if status != 0 {
            stop("Original native synchronous response failed");
        }
        0
    }
    unsafe extern "sysv64" fn connection_test_callback(
        handler: usize,
        server: usize,
        packet: usize,
        data: usize,
        bytes: i32,
    ) -> i32 {
        let context = DISPATCH
            .with(|d| *d.borrow())
            .unwrap_or_else(|| stop("Missing dispatch probe context"));
        if handler != context.3 || server != context.1 || bytes != 0 || data != 0 {
            stop("Native connection callback arguments differ from the probe contract");
        }
        let header = read(packet, 0x154);
        let client = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let memory_size = u64::from_le_bytes(header[0x148..0x150].try_into().unwrap());
        let payload_size = u32::from_le_bytes(header[0x150..0x154].try_into().unwrap());
        if memory_size != 16
            || payload_size != 0
            || CLIENTS.with(|k| k.borrow().resolve_server(1, client)) != Ok(context.2)
            || CONNECT_PACKET.with(|p| p.borrow().is_some())
        {
            stop("Native callback packet did not match the delivered connection request");
        }
        event(
            json!({"event":"native-server-connection-callback-reached","client_handle":client,
            "native_memory_size_field":memory_size,"payload_bytes":bytes,"synthetic_test_endpoint":true,
            "session_created":false,"original_shell_service_verified":false,"authentic_dashboard_verified":false}),
        );
        if let Some((create, destroy)) = SESSION_EXPORTS.with(|e| *e.borrow()) {
            let working = Block::new(4096).unwrap_or_else(|e| stop(&e));
            let output = Block::new(8).unwrap_or_else(|e| stop(&e));
            let create: unsafe extern "sysv64" fn(usize, usize, usize, usize) -> i32 =
                unsafe { std::mem::transmute(create) };
            event(json!({"event":"calling-native-session-create","working_bytes":working.size}));
            let status =
                unsafe { create(server, output.address as usize, 0, working.address as usize) };
            let pointer =
                usize::from_le_bytes(read(output.address as usize, 8).try_into().unwrap());
            let handle =
                u32::from_le_bytes(read(working.address as usize + 8, 4).try_into().unwrap());
            let registered = CLIENTS.with(|k| {
                k.borrow().session(1, handle).is_ok_and(|s| {
                    s.implementation == pointer as u64
                        && s.client == client
                        && s.server == context.2
                })
            });
            event(
                json!({"event":"native-session-create-returned","status":status,"handle":handle,
                "registry_matches_native_object":registered,"connection_completed":false,
                "synthetic_test_endpoint":true,"authentic_dashboard_verified":false}),
            );
            if status != 0 || pointer != working.address as usize || !registered {
                stop("Native session creation did not match its registry object");
            }
            if HANDSHAKE.get() {
                RETAINED_SESSION.with(|s| *s.borrow_mut() = Some((working, handle)));
                event(
                    json!({"event":"test-connection-callback-returning","status":0,"session_retained":true}),
                );
                return 0;
            }
            let destroy: unsafe extern "sysv64" fn(usize) -> i32 =
                unsafe { std::mem::transmute(destroy) };
            let status = unsafe { destroy(pointer) };
            let invalidated =
                u32::from_le_bytes(read(pointer + 8, 4).try_into().unwrap()) == u32::MAX;
            let removed = CLIENTS.with(|k| {
                matches!(
                    k.borrow().session(1, handle),
                    Err(crate::ipmi_kernel::Error::UnknownHandle)
                )
            });
            event(
                json!({"event":"native-session-destroy-returned","status":status,"native_handle_invalidated":invalidated,
                "session_registry_entry_removed":removed}),
            );
            if status != 0 || !invalidated || !removed {
                stop("Native session cleanup did not match its registry");
            }
            stop("Native session lifecycle verified; connection completion and concurrent dispatch remain unsupported");
        }
        stop("Original server dispatch reached the test callback; native session allocation is not implemented")
    }
    fn server_test(module: &Module, connect: bool, dispatch: bool) -> Result<(), String> {
        let config = Block::new(64)?;
        let ctor: unsafe extern "sysv64" fn(usize) =
            unsafe { std::mem::transmute(module.export("vo2FiTXAekk")?) };
        unsafe {
            ctor(config.address as usize);
        }
        let handler = Block::new(8)?;
        let vtable = Block::new(128)?;
        unsafe {
            for i in 0..16 {
                (vtable.address as *mut usize)
                    .add(i)
                    .write_unaligned(unexpected_test_callback as *const () as usize);
            }
            (handler.address as *mut usize).write_unaligned(vtable.address as usize);
            if dispatch {
                (vtable.address.add(0x20) as *mut usize)
                    .write_unaligned(connection_test_callback as *const () as usize);
            }
            if RPC_ENABLED.get() {
                (vtable.address.add(0x10) as *mut usize)
                    .write_unaligned(generic_test_callback as *const () as usize);
                (vtable.address.add(0x38) as *mut usize)
                    .write_unaligned(sync_test_callback as *const () as usize);
            }
            (config.address.add(0x10) as *mut usize).write_unaligned(handler.address as usize);
            let name = b"PS4IpcTest\0";
            std::ptr::copy_nonoverlapping(name.as_ptr(), config.address.add(0x19), name.len());
        }
        let estimate: unsafe extern "sysv64" fn(usize) -> usize =
            unsafe { std::mem::transmute(module.export("OXc1BksfEIg")?) };
        let bytes = unsafe { estimate(config.address as usize) };
        if !(0x100..=1024 * 1024).contains(&bytes) {
            return Err("Native server memory estimate exceeds component bound".into());
        }
        let working = Block::new(bytes)?;
        let output = Block::new(8)?;
        event(
            json!({"event":"calling-native-server-create","service":"PS4IpcTest","working_bytes":bytes,"synthetic_test_endpoint":true}),
        );
        let create: unsafe extern "sysv64" fn(usize, usize, usize, usize) -> i32 =
            unsafe { std::mem::transmute(module.export("hoWSsRT0ktA")?) };
        let status = unsafe {
            create(
                output.address as usize,
                config.address as usize,
                0,
                working.address as usize,
            )
        };
        let pointer = usize::from_le_bytes(read(output.address as usize, 8).try_into().unwrap());
        let id = u32::from_le_bytes(read(working.address as usize + 8, 4).try_into().unwrap());
        let registered = CLIENTS.with(|kernel| {
            kernel
                .borrow()
                .server(1, id)
                .is_ok_and(|s| s.implementation == pointer as u64 && s.service == "PS4IpcTest")
        });
        event(
            json!({"event":"native-server-create-returned","status":status,"handle":id,"registry_matches_native_object":registered}),
        );
        if status != 0 || pointer != working.address as usize || !registered {
            return Err("Native server creation failed verification".into());
        }
        if connect {
            if dispatch {
                let entry = module.export("GIniJssLcts")?;
                DISPATCH.with(|d| {
                    *d.borrow_mut() = Some((entry, pointer, id, handler.address as usize))
                });
            }
            let result = if THREADED.get() {
                let functions = module.functions();
                let regions = REGION.with(|r| r.borrow().clone());
                let imports = IMPORTS.with(|i| i.borrow().clone());
                let context = DISPATCH.with(|d| *d.borrow());
                let session_exports = SESSION_EXPORTS.with(|e| *e.borrow());
                let request_probe = REQUEST_PROBE.get();
                let multibuffer_probe = MULTIBUFFER_PROBE.get();
                let rpc_enabled = RPC_ENABLED.get();
                let rpc_error = RPC_ERROR.get();
                let generic_delivery = GENERIC_DELIVERY.get();
                std::thread::scope(|scope| {
                    let client = scope.spawn(move || {
                        // Parent allocations stay alive until this scoped thread joins.
                        REGION.with(|r| *r.borrow_mut() = regions);
                        IMPORTS.with(|i| *i.borrow_mut() = imports);
                        DISPATCH.with(|d| *d.borrow_mut() = context);
                        SESSION_EXPORTS.with(|e| *e.borrow_mut() = session_exports);
                        HANDSHAKE.set(true);
                        THREADED.set(true);
                        REQUEST_PROBE.set(request_probe);
                        MULTIBUFFER_PROBE.set(multibuffer_probe);
                        RPC_ENABLED.set(rpc_enabled);
                        RPC_ERROR.set(rpc_error);
                        GENERIC_DELIVERY.set(generic_delivery);
                        client_test(&functions, "--connect", b"PS4IpcTest\0")
                    });
                    channel()
                        .wait_ready(THREAD_WAIT)
                        .unwrap_or_else(|e| stop(e));
                    let buffer = Block::new(16384).unwrap_or_else(|e| stop(&e));
                    let dispatch: unsafe extern "sysv64" fn(usize, usize, usize) -> i32 =
                        unsafe { std::mem::transmute(context.unwrap().0) };
                    event(
                        json!({"event":"calling-native-server-try-dispatch","synchronous_probe":false,
                        "host_thread":unsafe{windows_sys::Win32::System::Threading::GetCurrentThreadId()}}),
                    );
                    let status = unsafe { dispatch(pointer, buffer.address as usize, buffer.size) };
                    event(json!({"event":"native-server-try-dispatch-returned","status":status}));
                    if status != 0 {
                        stop("Concurrent native dispatcher failed");
                    }
                    if RPC_ENABLED.get() {
                        rpc().wait_ready(THREAD_WAIT).unwrap_or_else(|e| stop(e));
                        let status =
                            unsafe { dispatch(pointer, buffer.address as usize, buffer.size) };
                        event(
                            json!({"event":"native-server-rpc-dispatch-returned","status":status}),
                        );
                        if status != 0 {
                            stop("Native RPC dispatch failed");
                        }
                    }
                    channel()
                        .wait_client_returned(THREAD_WAIT)
                        .unwrap_or_else(|e| stop(e));
                    cleanup_session(&module.functions()).unwrap_or_else(|e| stop(&e));
                    channel().allow_teardown().unwrap_or_else(|e| stop(e));
                    client
                        .join()
                        .unwrap_or_else(|_| stop("Native client worker panicked"))
                })
            } else {
                client_test(&module.functions(), "--connect", b"PS4IpcTest\0")
            };
            if !HANDSHAKE.get() || result.is_err() {
                return result;
            }
        }
        let destroy: unsafe extern "sysv64" fn(usize) -> i32 =
            unsafe { std::mem::transmute(module.export("dPOHeORqhbw")?) };
        let status = unsafe { destroy(pointer) };
        let invalidated = u32::from_le_bytes(read(pointer + 8, 4).try_into().unwrap()) == u32::MAX;
        let empty = CLIENTS.with(|kernel| kernel.borrow().is_empty());
        let mutexes_empty = MUTEXES.lock().unwrap().is_empty();
        event(
            json!({"event":"native-server-destroy-returned","status":status,"native_handle_invalidated":invalidated,"registry_empty":empty,"mutex_registry_empty":mutexes_empty}),
        );
        if status != 0 || !invalidated || !empty || !mutexes_empty {
            return Err("Native server cleanup failed verification".into());
        }
        event(
            json!({"event":"server-component-test-complete","native_ipmi_server_lifecycle_verified":true,
                "native_synthetic_connection_verified":HANDSHAKE.get(),"concurrent_native_dispatch_verified":THREADED.get(),
                "native_synthetic_rpc_verified":RPC_ENABLED.get(),
                "synthetic_test_endpoint":true,"original_shell_service_verified":false,"authentic_dashboard_verified":false}),
        );
        Ok(())
    }
    pub fn worker(path: &Path, mode: &str) -> Result<(), String> {
        let module = Module::load(path)?;
        event(json!({"event":"calling-module-init"}));
        let init: unsafe extern "sysv64" fn(u64, usize, usize) -> i32 =
            unsafe { std::mem::transmute(module.init) };
        let status = unsafe { init(0, 0, 0) };
        event(json!({"event":"module-init-returned","status":status}));
        if status != 0 {
            return Err("Native module initialization failed".into());
        }
        GENERIC_DELIVERY.set(matches!(
            mode,
            "--threaded-multibuffer-rpc" | "--threaded-multibuffer-rpc-error"
        ));
        if matches!(
            mode,
            "--threaded-rpc"
                | "--threaded-rpc-error"
                | "--threaded-multibuffer-dispatch"
                | "--threaded-multibuffer-reply-probe"
        ) || GENERIC_DELIVERY.get()
        {
            RPC_ENABLED.set(true);
            RESPOND_EXPORT.set(module.export("r0t1NxWUNwI")?);
        }
        RPC_ERROR.set(matches!(
            mode,
            "--threaded-rpc-error" | "--threaded-multibuffer-rpc-error"
        ));
        if mode == "--threaded-multibuffer-reply-probe" || GENERIC_DELIVERY.get() {
            GENERIC_RESPOND_EXPORT.set(module.export("AYyxvDy-SFU")?);
        }
        MULTIBUFFER_PROBE.set(
            matches!(
                mode,
                "--threaded-multibuffer-request"
                    | "--threaded-multibuffer-dispatch"
                    | "--threaded-multibuffer-reply-probe"
            ) || GENERIC_DELIVERY.get(),
        );
        if [
            "--handshake",
            "--threaded-handshake",
            "--threaded-request",
            "--threaded-multibuffer-request",
            "--threaded-multibuffer-dispatch",
            "--threaded-multibuffer-reply-probe",
            "--threaded-multibuffer-rpc",
            "--threaded-multibuffer-rpc-error",
            "--threaded-rpc",
            "--threaded-rpc-error",
        ]
        .contains(&mode)
        {
            HANDSHAKE.set(true);
        }
        if mode == "--threaded-handshake"
            || (mode == "--threaded-request" || MULTIBUFFER_PROBE.get())
            || RPC_ENABLED.get()
        {
            THREADED.set(true);
        }
        if (mode == "--threaded-request" || MULTIBUFFER_PROBE.get()) || RPC_ENABLED.get() {
            REQUEST_PROBE.set(true);
        }
        if [
            "--server",
            "--server-connect",
            "--server-dispatch",
            "--server-session",
            "--handshake",
            "--threaded-handshake",
            "--threaded-request",
            "--threaded-multibuffer-request",
            "--threaded-multibuffer-dispatch",
            "--threaded-multibuffer-reply-probe",
            "--threaded-multibuffer-rpc",
            "--threaded-multibuffer-rpc-error",
            "--threaded-rpc",
            "--threaded-rpc-error",
        ]
        .contains(&mode)
        {
            if mode == "--server-session" || HANDSHAKE.get() {
                let exports = (module.export("O+Rk6bioKMY")?, module.export("YWOjuEzZ3Ic")?);
                SESSION_EXPORTS.with(|e| *e.borrow_mut() = Some(exports));
            }
            return server_test(
                &module,
                mode != "--server",
                [
                    "--server-dispatch",
                    "--server-session",
                    "--handshake",
                    "--threaded-handshake",
                    "--threaded-request",
                    "--threaded-multibuffer-request",
                    "--threaded-multibuffer-dispatch",
                    "--threaded-multibuffer-reply-probe",
                    "--threaded-multibuffer-rpc",
                    "--threaded-multibuffer-rpc-error",
                    "--threaded-rpc",
                    "--threaded-rpc-error",
                ]
                .contains(&mode),
            );
        }
        client_test(&module.functions(), mode, b"SceSystemLoggerService\0")
    }
    fn cleanup_session(module: &ModuleFunctions) -> Result<(), String> {
        let (session, handle) = RETAINED_SESSION
            .with(|s| s.borrow_mut().take())
            .ok_or("Native session disappeared before cleanup")?;
        let destroy: unsafe extern "sysv64" fn(usize) -> i32 =
            unsafe { std::mem::transmute(module.export("YWOjuEzZ3Ic")?) };
        let status = unsafe { destroy(session.address as usize) };
        let invalidated =
            u32::from_le_bytes(read(session.address as usize + 8, 4).try_into().unwrap())
                == u32::MAX;
        if status != 0 || !invalidated || CLIENTS.with(|k| k.borrow().session(1, handle).is_ok()) {
            return Err("Connected session cleanup failed".into());
        }
        event(
            json!({"event":"connected-session-destroyed","native_handle_invalidated":true,"status":status}),
        );
        Ok(())
    }
    // SystemLogger-shaped synthetic request through the original generic vtable
    // entry. Deliberately stops at the unimplemented transport, without a reply.
    fn multibuffer_request_probe(client: usize) {
        let count = Block::new(8).unwrap_or_else(|e| stop(&e));
        let name = Block::new(16).unwrap_or_else(|e| stop(&e));
        let rows = Block::new(100).unwrap_or_else(|e| stop(&e));
        let inputs = Block::new(48).unwrap_or_else(|e| stop(&e));
        let outputs = Block::new(24).unwrap_or_else(|e| stop(&e));
        let response = Block::new(36).unwrap_or_else(|e| stop(&e));
        let result = Block::new(4).unwrap_or_else(|e| stop(&e));
        unsafe {
            std::ptr::write_bytes(response.address, 0xa5, 36);
            (count.address as *mut u64).write_unaligned(1);
            std::ptr::copy_nonoverlapping(b"PS4IpcTest".as_ptr(), name.address, 10);
            for (index, (buffer, size)) in [
                (count.address, 8usize),
                (name.address, 16),
                (rows.address, 100),
            ]
            .into_iter()
            .enumerate()
            {
                (inputs.address.add(index * 16) as *mut usize).write_unaligned(buffer as usize);
                (inputs.address.add(index * 16 + 8) as *mut usize).write_unaligned(size);
            }
            (outputs.address as *mut usize).write_unaligned(response.address as usize + 16);
            (outputs.address.add(8) as *mut usize).write_unaligned(4);
            (outputs.address.add(16) as *mut usize).write_unaligned(usize::MAX);
            (result.address as *mut i32).write_unaligned(i32::MAX);
        }
        let vtable = usize::from_le_bytes(read(client, 8).try_into().unwrap());
        let entry = usize::from_le_bytes(read(vtable + 0x58, 8).try_into().unwrap());
        if !can_access(entry, 1, false) {
            stop("Generic IPMI vtable entry is outside owned mappings");
        }
        let invoke: unsafe extern "sysv64" fn(usize, u32, usize, u32, usize, usize, u32) -> i32 =
            unsafe { std::mem::transmute(entry) };
        event(
            json!({"event":"calling-native-multibuffer-method","method_id":0x03299000u32,
            "vtable_slot":0x58,"input_bytes":[8,16,100],"output_capacities":[4],
            "synthetic_test_endpoint":true,"original_logger_library_executed":false,
            "original_shell_service":false,"authentic_dashboard_verified":false}),
        );
        let status = unsafe {
            invoke(
                client,
                0x03299000,
                inputs.address as usize,
                3,
                result.address as usize,
                outputs.address as usize,
                1,
            )
        };
        if !GENERIC_DELIVERY.get() {
            stop("Multi-buffer probe returned without verified service response");
        }
        let method_status =
            i32::from_le_bytes(read(result.address as usize, 4).try_into().unwrap());
        let length =
            usize::from_le_bytes(read(outputs.address as usize + 16, 8).try_into().unwrap());
        let expected_status = if RPC_ERROR.get() { -123 } else { 0 };
        let expected_length = if RPC_ERROR.get() { 0 } else { 4 };
        let guards = read(response.address as usize, 16) == vec![0xa5; 16]
            && read(response.address as usize + 20, 16) == vec![0xa5; 16];
        if status != 0
            || method_status != expected_status
            || length != expected_length
            || !guards
            || (length == 4
                && read(response.address as usize + 16, 4) != 0x12345678u32.to_le_bytes())
            || (length == 0 && read(response.address as usize + 16, 4) != vec![0xa5; 4])
        {
            stop("Generic native client result, length, payload or guard mismatch");
        }
        event(
            json!({"event":"native-generic-roundtrip-verified","transport_status":status,
            "method_status":method_status,"response_bytes":length,"exact_payload_match":true,
            "guard_bytes_intact":guards,"empty_response_buffer_unchanged":length != 0 || read(response.address as usize+16,4)==vec![0xa5;4],
            "synthetic_test_endpoint":true,"original_shell_service":false,
            "original_logger_library_executed":false,"authentic_dashboard_verified":false}),
        );
    }
    fn client_test(module: &ModuleFunctions, mode: &str, name: &[u8]) -> Result<(), String> {
        let config = Block::new(0x160)?;
        let ctor: unsafe extern "sysv64" fn(*mut u8) =
            unsafe { std::mem::transmute(module.export("O1lQ2+do5r4")?) };
        unsafe {
            ctor(config.address);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(name.as_ptr(), config.address, name.len());
        }
        let estimate: unsafe extern "sysv64" fn(*mut u8) -> usize =
            unsafe { std::mem::transmute(module.export("Rih7HnIM3JQ")?) };
        let bytes = unsafe { estimate(config.address) };
        if bytes == 0 || bytes > 1024 * 1024 {
            return Err("Native client memory estimate exceeds test bound".into());
        }
        let working = Block::new(bytes)?;
        let output = Block::new(8)?;
        event(json!({"event":"calling-native-client-create","working_bytes":bytes}));
        let create: unsafe extern "sysv64" fn(*mut usize, *const u8, usize, *mut u8) -> i32 =
            unsafe { std::mem::transmute(module.export("0zsTiDhM0nU")?) };
        let status =
            unsafe { create(output.address as *mut _, config.address, 0, working.address) };
        let pointer = unsafe { (output.address as *const usize).read_unaligned() };
        let id = u32::from_le_bytes(read(working.address as usize + 8, 4).try_into().unwrap());
        let registered = CLIENTS.with(|clients| {
            clients
                .borrow()
                .client(1, id)
                .is_ok_and(|client| client.implementation == pointer as u64)
        });
        event(
            json!({"event":"native-client-create-returned","status":status,"native_client_handle":id,"registry_matches_native_object":registered,"authentic_dashboard_verified":false}),
        );
        if status != 0 || pointer != working.address as usize || !registered {
            return Err("Native client creation did not match the actual handle registry".into());
        }
        if mode == "--connect" {
            let result = Block::new(4)?;
            event(
                json!({"event":"calling-native-client-connect","service":String::from_utf8_lossy(&name[..name.len()-1])}),
            );
            let call: unsafe extern "sysv64" fn(usize, usize, usize, usize) -> i32 =
                unsafe { std::mem::transmute(module.export("vogma5QshU4")?) };
            let status = unsafe { call(pointer, 0, 0, result.address as usize) };
            let callback_status =
                i32::from_le_bytes(read(result.address as usize, 4).try_into().unwrap());
            event(
                json!({"event":"native-connect-returned","status":status,"callback_status":callback_status}),
            );
            if !HANDSHAKE.get() {
                return Err(
                    "Connection testing must not silently report success without a service".into(),
                );
            }
            if status != 0 || callback_status != 0 || COMPLETION.get() != Some((id, 0)) {
                return Err("Native connection did not return verified completion".into());
            }
            if MULTIBUFFER_PROBE.get() {
                multibuffer_request_probe(pointer);
            }
            if REQUEST_PROBE.get() && !MULTIBUFFER_PROBE.get() {
                let method_result = Block::new(4)?;
                let response_bytes = Block::new(8)?;
                let response = Block::new(48)?;
                unsafe {
                    std::ptr::write_bytes(response.address, 0xa5, 48);
                }
                let input = Block::new(16)?;
                let payload = b"native-ipmi-test";
                let input_length = if RPC_ENABLED.get() { payload.len() } else { 0 };
                unsafe {
                    std::ptr::copy_nonoverlapping(payload.as_ptr(), input.address, payload.len());
                    (method_result.address as *mut i32).write_unaligned(i32::MAX);
                }
                let invoke: unsafe extern "sysv64" fn(
                    usize,
                    u32,
                    usize,
                    usize,
                    usize,
                    usize,
                    usize,
                    usize,
                ) -> i32 = unsafe { std::mem::transmute(module.export("57XmD42P9gQ")?) };
                event(
                    json!({"event":"calling-native-sync-method","method_id":0,"input_bytes":input_length,
                    "response_capacity":16,"synthetic_test_endpoint":true,"original_shell_service":false}),
                );
                let status = unsafe {
                    invoke(
                        pointer,
                        0,
                        if input_length == 0 {
                            0
                        } else {
                            input.address as usize
                        },
                        input_length,
                        method_result.address as usize,
                        response.address as usize + 16,
                        response_bytes.address as usize,
                        16,
                    )
                };
                event(json!({"event":"native-sync-method-returned","status":status}));
                if !RPC_ENABLED.get() {
                    stop("Native method probe returned without a verified service response");
                }
                let length = usize::from_le_bytes(
                    read(response_bytes.address as usize, 8).try_into().unwrap(),
                );
                let method_status =
                    i32::from_le_bytes(read(method_result.address as usize, 4).try_into().unwrap());
                let expected: Vec<u8> = if RPC_ERROR.get() {
                    vec![]
                } else {
                    payload.iter().rev().copied().collect()
                };
                let expected_status = if RPC_ERROR.get() { -123 } else { 0 };
                let guards_intact = read(response.address as usize, 16) == vec![0xa5; 16]
                    && read(response.address as usize + 32, 16) == vec![0xa5; 16];
                if status != 0
                    || method_status != expected_status
                    || length != expected.len()
                    || read(response.address as usize + 16, length) != expected
                    || !guards_intact
                {
                    stop("Native RPC did not return the exact diagnostic response");
                }
                event(
                    json!({"event":"native-sync-roundtrip-verified","method_status":method_status,"response_bytes":length,
                    "exact_payload_match":true,"guard_bytes_intact":guards_intact,"synthetic_test_endpoint":true,"original_shell_service":false,"authentic_dashboard_verified":false}),
                );
            }
            if THREADED.get() {
                channel().client_returned(id).map_err(str::to_string)?;
                channel()
                    .wait_teardown(THREAD_WAIT)
                    .map_err(str::to_string)?;
            } else {
                cleanup_session(module)?;
            }
        }
        let destroy: unsafe extern "sysv64" fn(usize) -> i32 =
            unsafe { std::mem::transmute(module.export("R9CB6TmSpeU")?) };
        let status = unsafe { destroy(pointer) };
        let invalidated =
            u32::from_le_bytes(read(working.address as usize + 8, 4).try_into().unwrap())
                == u32::MAX;
        let empty = CLIENTS.with(|clients| clients.borrow().is_empty());
        event(
            json!({"event":"native-client-destroy-returned","status":status,"native_handle_invalidated":invalidated,"registry_empty":empty}),
        );
        if status != 0 || !invalidated || (!empty && !HANDSHAKE.get()) {
            return Err("Native client destruction did not match registry cleanup".into());
        }
        event(
            json!({"event":"component-test-complete","native_ipmi_client_creation_verified":true,"native_ipmi_client_destruction_verified":true,"connected_service_verified":false,"authentic_dashboard_verified":false,"mapped_module_bytes":module.image_size}),
        );
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn native_errno_bridge_is_thread_local_and_converts_eagain() {
            unsafe {
                *error_address() = 35;
            }
            let child = std::thread::spawn(|| unsafe {
                assert_eq!(*error_address(), 0);
                *error_address() = 4;
                assert_eq!(kernel_error(*error_address()), 0x80020004u32 as i32);
            });
            child.join().unwrap();
            unsafe {
                assert_eq!(*error_address(), 35);
                assert_eq!(kernel_error(*error_address()), 0x80020023u32 as i32);
                assert_eq!(kernel_error(0), 0);
            }
        }
        #[test]
        fn default_mutex_bridge_releases_resources() {
            let pointer = Block::new(8).unwrap();
            let name = Block::new(32).unwrap();
            unsafe {
                let value = c"ComponentMutex";
                std::ptr::copy_nonoverlapping(
                    value.as_ptr().cast::<u8>(),
                    name.address,
                    value.to_bytes_with_nul().len(),
                );
            }
            let address = pointer.address as usize;
            unsafe {
                assert_eq!(mutex_init(address, 0, name.address as usize), 0);
                let handle = mutex_handle(address);
                assert_eq!(mutex_lock(address), 0);
                assert_ne!(MUTEXES.lock().unwrap()[&handle].0, 0);
                assert_eq!(mutex_unlock(address), 0);
                assert_eq!(MUTEXES.lock().unwrap()[&handle], (0, 0));
                assert_eq!(mutex_destroy(address), 0);
                assert!(!MUTEXES.lock().unwrap().contains_key(&handle));
                assert_eq!(mutex_handle(address), 0);
            }
        }
        #[test]
        fn import_memory_access_stays_in_owned_writable_ranges() {
            let block = Block::new(4096).unwrap();
            let address = block.address as usize;
            assert!(can_access(address, 4096, true));
            assert!(!can_access(address + 4096, 1, false));
            assert!(!can_access(0, 8, false));
            let mut old = 0;
            assert_ne!(
                unsafe { VirtualProtect(block.address as *const _, 4096, PAGE_READONLY, &mut old) },
                0
            );
            assert!(can_access(address, 8, false));
            assert!(!can_access(address, 8, true));
        }
    }
}

fn main() {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use std::{
            fs,
            os::windows::process::CommandExt,
            path::{Path, PathBuf},
            process::{Command, Stdio},
            time::{Duration, Instant, SystemTime, UNIX_EPOCH},
        };
        let args: Vec<_> = std::env::args().skip(1).collect();
        if (2..=3).contains(&args.len()) && args[0] == "--worker" {
            if let Err(error) = native::worker(
                Path::new(&args[1]),
                args.get(2).map(String::as_str).unwrap_or_default(),
            ) {
                println!("{}", serde_json::json!({"event":"failure","reason":error}));
                std::process::exit(1);
            }
            return;
        }
        let result = (|| -> Result<(), String> {
            if !(2..=3).contains(&args.len())
                || (args.len() == 3
                    && ![
                        "--connect",
                        "--server",
                        "--server-connect",
                        "--server-dispatch",
                        "--server-session",
                        "--handshake",
                        "--threaded-handshake",
                        "--threaded-request",
                        "--threaded-multibuffer-request",
                        "--threaded-multibuffer-dispatch",
                        "--threaded-multibuffer-reply-probe",
                        "--threaded-multibuffer-rpc",
                        "--threaded-multibuffer-rpc-error",
                        "--threaded-rpc",
                        "--threaded-rpc-error",
                    ]
                    .contains(&args[2].as_str()))
            {
                return Err(
                    "Usage: PS4ModuleTest DUMP_ROOT REPORT_PARENT [--connect|--server|--server-connect|--server-dispatch|--server-session|--handshake|--threaded-handshake|--threaded-request|--threaded-multibuffer-request|--threaded-multibuffer-dispatch|--threaded-multibuffer-reply-probe|--threaded-multibuffer-rpc|--threaded-multibuffer-rpc-error|--threaded-rpc|--threaded-rpc-error]".into(),
                );
            }
            let dump = fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
            let parent = fs::canonicalize(&args[1]).map_err(|e| e.to_string())?;
            if parent.starts_with(&dump) || dump.starts_with(&parent) {
                return Err("Report parent must be separate from original dumps".into());
            }
            let module = dump.join("system/common/lib/libSceIpmi.sprx");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let output = parent.join(format!("ipmi-component-{now}"));
            fs::create_dir(&output).map_err(|e| e.to_string())?;
            let stdout = output.join("events.jsonl");
            let stderr = output.join("stderr.txt");
            let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            command.arg("--worker").arg(&module);
            if args.len() == 3 {
                command.arg(&args[2]);
            }
            let mut child = command
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(fs::File::create(&stdout).map_err(|e| e.to_string())?)
                .stderr(fs::File::create(&stderr).map_err(|e| e.to_string())?)
                .spawn()
                .map_err(|e| e.to_string())?;
            let start = Instant::now();
            let mut timed_out = false;
            let status = loop {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    break status;
                }
                if start.elapsed() > Duration::from_secs(20) {
                    child.kill().map_err(|e| e.to_string())?;
                    timed_out = true;
                    break child.wait().map_err(|e| e.to_string())?;
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let report = serde_json::json!({"module":module,"test_mode":args.get(2).map(String::as_str).unwrap_or("client-lifecycle"),"events":stdout,"stderr":stderr,"exit_code":status.code(),"timed_out":timed_out,"authentic_dashboard_verified":false});
            let path: PathBuf = output.join("result.json");
            fs::write(&path, serde_json::to_vec_pretty(&report).unwrap())
                .map_err(|e| e.to_string())?;
            println!("{}", path.display());
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        eprintln!("PS4ModuleTest requires Windows x64");
        std::process::exit(2);
    }
}
