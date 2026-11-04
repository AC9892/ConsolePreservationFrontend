//! Keep Windows backend processes owned by the CPF process that started them.
use std::{io, process::Child};

pub const BACKEND_EXITED_MESSAGE: &str = "backend exited while the shell was running";

/// Drain child output while watching the process it depends on. The job must
/// own this child so an unavailable peer cannot leave a waiting UI behind.
#[cfg(windows)]
pub fn wait_with_peer(
    child: Child,
    job: ProcessJob,
    mut peer_running: impl FnMut() -> io::Result<bool>,
) -> io::Result<std::process::Output> {
    let waiter = std::thread::Builder::new()
        .name("cpf-shell-output".into())
        .spawn(move || child.wait_with_output())?;
    let mut peer_error = None;
    while !waiter.is_finished() {
        match peer_running() {
            Ok(true) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Ok(false) => {
                peer_error = Some(io::Error::other(BACKEND_EXITED_MESSAGE));
                break;
            }
            Err(error) => {
                peer_error = Some(error);
                break;
            }
        }
    }
    // On peer failure this terminates the shell and closes its output pipe.
    drop(job);
    let output = waiter
        .join()
        .map_err(|_| io::Error::other("shell output worker failed"))?;
    if let Some(error) = peer_error {
        return Err(error);
    }
    output
}

pub struct ProcessJob {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl ProcessJob {
    /// A failed association terminates the newly spawned child rather than
    /// leaving an unowned backend behind.
    pub fn attach(child: &mut Child) -> io::Result<Self> {
        let result = Self::assign(child);
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    #[cfg(windows)]
    fn assign(child: &Child) -> io::Result<Self> {
        use std::{
            mem::{size_of, zeroed},
            os::windows::io::AsRawHandle,
            ptr::null,
        };
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        unsafe {
            let handle = CreateJobObjectW(null(), null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self { handle };
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }

    #[cfg(not(windows))]
    fn assign(_child: &Child) -> io::Result<Self> {
        Ok(Self {})
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    #[ignore]
    fn sleeping_child_fixture() {
        if std::env::var_os("CPF_JOB_TEST_CHILD").is_none() {
            return;
        }
        if let Some(path) = std::env::var_os("CPF_JOB_TEST_READY") {
            std::fs::write(path, b"ready").unwrap();
        }
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn closing_owner_job_terminates_its_child() {
        let ready = std::env::temp_dir().join(format!(
            "cpf-job-ready-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "process_lifecycle::tests::sleeping_child_fixture",
            ])
            .env("CPF_JOB_TEST_CHILD", "1")
            .env("CPF_JOB_TEST_READY", &ready)
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let job = ProcessJob::attach(&mut child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ready.exists() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child fixture did not reach its wait loop");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_none());
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if child.try_wait().unwrap().is_some() {
                std::fs::remove_file(&ready).unwrap();
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned child survived job closure");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore]
    fn output_child_fixture() {
        if std::env::var_os("CPF_JOB_TEST_OUTPUT").is_some() {
            use std::io::Write;
            std::io::stdout()
                .write_all(&vec![b'x'; 256 * 1024])
                .unwrap();
        }
    }

    fn piped_fixture(name: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", name, "--nocapture"])
            .env("CPF_JOB_TEST_CHILD", "1")
            .env("CPF_JOB_TEST_OUTPUT", "1")
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn peer_failure_terminates_waiting_child() {
        let mut child = piped_fixture("process_lifecycle::tests::sleeping_child_fixture");
        let job = ProcessJob::attach(&mut child).unwrap();
        let started = Instant::now();
        let error = wait_with_peer(child, job, || Ok(false)).unwrap_err();
        assert!(error.to_string().contains("backend exited"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn peer_check_failure_is_reported_and_child_is_reaped() {
        let mut child = piped_fixture("process_lifecycle::tests::sleeping_child_fixture");
        let job = ProcessJob::attach(&mut child).unwrap();
        let error =
            wait_with_peer(child, job, || Err(io::Error::other("peer check failed"))).unwrap_err();
        assert_eq!(error.to_string(), "peer check failed");
    }

    #[test]
    fn child_output_is_drained_while_peer_is_running() {
        let mut child = piped_fixture("process_lifecycle::tests::output_child_fixture");
        let job = ProcessJob::attach(&mut child).unwrap();
        let started = Instant::now();
        let output = wait_with_peer(child, job, || {
            if started.elapsed() > Duration::from_secs(5) {
                return Err(io::Error::other("output pipe stalled"));
            }
            Ok(true)
        })
        .unwrap();
        assert!(output.status.success());
        assert!(output
            .stdout
            .windows(256 * 1024)
            .any(|bytes| bytes.iter().all(|b| *b == b'x')));
    }
}
