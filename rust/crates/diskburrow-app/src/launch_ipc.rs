//! Bounded, same-user and same-executable launch-only transport.
#![allow(
    unsafe_code,
    reason = "Owned Win32 handles and nonblocking launch-only pipe boundary"
)]
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{Pipes::*, Threading::*},
};

pub const MAX_PAYLOAD_BYTES: usize = 65536;
const TIMEOUT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(5);
const QUEUE_SIZE: usize = 16;
struct Handle(HANDLE);
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn pipe_name(namespace: &str) -> io::Result<Vec<u16>> {
    if namespace.is_empty()
        || namespace.len() > 128
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(invalid("Invalid launch namespace"));
    }
    Ok(wide(&format!(r"\\.\pipe\{namespace}.Launch")))
}
fn validate(bytes: &[u8]) -> io::Result<&str> {
    if bytes.is_empty() || bytes.len() > MAX_PAYLOAD_BYTES || bytes.contains(&0) {
        return Err(invalid("Invalid launch payload size or NUL"));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid("Launch payload is not UTF-8"))
}
#[derive(Clone)]
struct Identity {
    sid: String,
    image: String,
}
impl Identity {
    fn current() -> io::Result<Self> {
        Self::of(unsafe { GetCurrentProcess() })
    }
    fn of(process: HANDLE) -> io::Result<Self> {
        let mut raw = std::ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle(raw);
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        if needed == 0 || needed > 65536 {
            return Err(io::Error::other("Invalid launch peer token size"));
        }
        let mut bytes = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() };
        let mut text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = unsafe {
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        unsafe {
            LocalFree(text.cast());
        }
        let mut image = vec![0u16; 32768];
        let mut len = image.len() as u32;
        if unsafe { QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut len) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let image = String::from_utf16(&image[..len as usize])
            .map_err(|_| io::Error::other("Invalid peer executable path"))?;
        Ok(Self { sid, image })
    }
    // Keep the process handle alive through the exchange; never trust payload-supplied PID/SID/path.
    fn pin_peer(&self, pipe: &Handle, server: bool) -> io::Result<Handle> {
        let mut pid = 0;
        let ok = unsafe {
            if server {
                GetNamedPipeServerProcessId(pipe.0, &mut pid)
            } else {
                GetNamedPipeClientProcessId(pipe.0, &mut pid)
            }
        };
        if ok == 0 || pid == 0 {
            return Err(io::Error::last_os_error());
        }
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(raw);
        let peer = Self::of(process.0)?;
        if peer.sid != self.sid || !peer.image.eq_ignore_ascii_case(&self.image) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Launch peer identity mismatch",
            ));
        }
        Ok(process)
    }
}
fn create_pipe(name: &[u16], identity: &Identity) -> io::Result<Handle> {
    let sddl = wide(&format!("D:P(A;;GA;;;{})", identity.sid));
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // Every transfer is deliberately nonblocking. No overlapped buffer survives a call.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_PAYLOAD_BYTES as u32,
            MAX_PAYLOAD_BYTES as u32,
            0,
            &attributes,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if pipe == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(pipe))
    }
}
fn pause(stop: &AtomicBool, deadline: Instant) -> io::Result<()> {
    if stop.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Launch IPC stopped",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Launch IPC timed out",
        ));
    }
    thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    Ok(())
}
fn read_message(
    pipe: &Handle,
    stop: &AtomicBool,
    deadline: Instant,
    maximum: usize,
) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; maximum];
    loop {
        pause(stop, deadline)?;
        let mut count = 0;
        if unsafe {
            ReadFile(
                pipe.0,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                &mut count,
                std::ptr::null_mut(),
            )
        } != 0
        {
            bytes.truncate(count as usize);
            return Ok(bytes);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_NO_DATA as i32) {
            return Err(error);
        }
    }
}
fn write_message(
    pipe: &Handle,
    stop: &AtomicBool,
    deadline: Instant,
    bytes: &[u8],
) -> io::Result<()> {
    loop {
        pause(stop, deadline)?;
        let mut count = 0;
        if unsafe {
            WriteFile(
                pipe.0,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut count,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if count as usize == bytes.len() {
            return Ok(());
        }
        if count != 0 {
            return Err(io::Error::other("Partial launch message"));
        }
    }
}
pub struct Server {
    messages: Receiver<String>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    /// Bind a private same-user namespace. The caller supplies its SID-derived app namespace.
    pub fn start(namespace: &str) -> io::Result<Self> {
        let name = pipe_name(namespace)?;
        let identity = Identity::current()?;
        let pipe = create_pipe(&name, &identity)?;
        let (sender, messages) = mpsc::sync_channel(QUEUE_SIZE);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let worker = thread::Builder::new()
            .name("DiskBurrow launch IPC".into())
            .spawn(move || serve(pipe, identity, sender, thread_stop))?;
        Ok(Self {
            messages,
            stop,
            worker: Some(worker),
        })
    }
    /// Pop a bounded launch payload. Strictly decode the read-only LaunchOverrides whitelist
    /// before acting; this transport never interprets or executes payload contents.
    pub fn try_recv(&self) -> Option<String> {
        self.messages.try_recv().ok()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // All native calls use NOWAIT; the only worker wait is the <=5ms polling pause.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn serve(pipe: Handle, identity: Identity, sender: SyncSender<String>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let connected = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) };
        // NOWAIT success only transitions a disconnected pipe to listening. A connection
        // exists only on ERROR_PIPE_CONNECTED; never inspect stale LastError on success.
        let error = if connected == 0 {
            unsafe { GetLastError() }
        } else {
            ERROR_PIPE_LISTENING
        };
        if error == ERROR_PIPE_CONNECTED {
            let deadline = Instant::now() + TIMEOUT;
            let _ = receive(&pipe, &identity, &sender, &stop, deadline);
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
        } else if error == ERROR_NO_DATA {
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
        }
        thread::sleep(POLL);
    }
}
fn receive(
    pipe: &Handle,
    identity: &Identity,
    sender: &SyncSender<String>,
    stop: &AtomicBool,
    deadline: Instant,
) -> io::Result<()> {
    let _peer = identity.pin_peer(pipe, false)?;
    let bytes = read_message(pipe, stop, deadline, MAX_PAYLOAD_BYTES)?;
    let payload = validate(&bytes)?;
    let accepted = sender.try_send(payload.to_owned()).is_ok();
    write_message(pipe, stop, deadline, &[u8::from(accepted)])?;
    // Wait for the client's receipt before disconnecting: DisconnectNamedPipe discards unread ACKs.
    let _ = read_message(pipe, stop, deadline, 1)?;
    Ok(())
}
fn open_client(name: &[u16], stop: &AtomicBool, deadline: Instant) -> io::Result<Handle> {
    loop {
        pause(stop, deadline)?;
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if raw != INVALID_HANDLE_VALUE {
            let pipe = Handle(raw);
            let mode = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
            if unsafe { SetNamedPipeHandleState(pipe.0, &mode, std::ptr::null(), std::ptr::null()) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            return Ok(pipe);
        }
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(n) if n == ERROR_PIPE_BUSY as i32 || n == ERROR_FILE_NOT_FOUND as i32)
        {
            return Err(error);
        }
    }
}
/// Forward prevalidated, read-only launch JSON with a single three-second deadline.
/// Success means queued by the peer transport; the UI still validates and safely schedules it.
pub fn forward(namespace: &str, payload: &str) -> io::Result<()> {
    validate(payload.as_bytes())?;
    forward_raw(namespace, payload.as_bytes())
}
fn forward_raw(namespace: &str, payload: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    let stop = AtomicBool::new(false);
    let name = pipe_name(namespace)?;
    let identity = Identity::current()?;
    let pipe = open_client(&name, &stop, deadline)?;
    let _peer = identity.pin_peer(&pipe, true)?;
    write_message(&pipe, &stop, deadline, payload)?;
    let accepted = read_message(&pipe, &stop, deadline, 1)?;
    let _ = write_message(&pipe, &stop, deadline, &[1]);
    if accepted != [1] {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Launch IPC queue is full",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    fn namespace() -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!(
            "DiskBurrow-launch-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    }
    #[test]
    fn owned_native_server_delivers_unicode_launch_and_stays_available() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let payload = r#"{"root":"E:\\owned IPC\\папка","depth":4}"#;
        forward(&name, payload).unwrap();
        assert_eq!(server.try_recv().as_deref(), Some(payload));
        forward(&name, "{}").unwrap();
        assert_eq!(server.try_recv().as_deref(), Some("{}"));
    }
    #[test]
    fn invalid_payloads_fail_before_connection() {
        let name = namespace();
        for payload in [
            String::new(),
            "x\0y".into(),
            "x".repeat(MAX_PAYLOAD_BYTES + 1),
        ] {
            assert_eq!(
                forward(&name, &payload).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
    #[test]
    fn unavailable_instance_times_out_within_three_seconds_plus_poll_margin() {
        let start = Instant::now();
        assert_eq!(
            forward(&namespace(), "{}").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_millis(3300));
    }
    #[test]
    fn idle_server_drop_is_bounded_and_releases_namespace() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_millis(250));
        drop(Server::start(&name).unwrap());
    }
    #[test]
    fn native_server_rejects_invalid_utf8_nul_and_oversize_wire_messages() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        for bytes in [
            vec![0xff],
            vec![b'{', 0, b'}'],
            vec![b'x'; MAX_PAYLOAD_BYTES + 1],
        ] {
            assert!(forward_raw(&name, &bytes).is_err());
            assert!(server.try_recv().is_none());
        }
        forward(&name, "{}").unwrap();
        assert_eq!(server.try_recv().as_deref(), Some("{}"));
    }
    #[test]
    fn native_queue_is_bounded_and_acknowledges_full_without_delivery() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let largest = "x".repeat(MAX_PAYLOAD_BYTES);
        forward(&name, &largest).unwrap();
        assert_eq!(server.try_recv().unwrap().len(), MAX_PAYLOAD_BYTES);
        for _ in 0..QUEUE_SIZE {
            forward(&name, "{}").unwrap();
        }
        assert_eq!(
            forward(&name, "{}").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        for _ in 0..QUEUE_SIZE {
            assert_eq!(server.try_recv().as_deref(), Some("{}"));
        }
        assert!(server.try_recv().is_none());
    }
    #[test]
    fn stalled_native_client_cannot_block_server_drop() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let stop = AtomicBool::new(false);
        let _pipe =
            open_client(&pipe_name(&name).unwrap(), &stop, Instant::now() + TIMEOUT).unwrap();
        thread::sleep(Duration::from_millis(25));
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_millis(250));
    }
    #[test]
    fn stalled_native_client_expires_and_listener_recovers() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let stop = AtomicBool::new(false);
        let _stalled =
            open_client(&pipe_name(&name).unwrap(), &stop, Instant::now() + TIMEOUT).unwrap();
        thread::sleep(TIMEOUT + Duration::from_millis(100));
        forward(&name, "{}").unwrap();
        assert_eq!(server.try_recv().as_deref(), Some("{}"));
    }
    #[test]
    fn real_peer_sid_pin_is_required_even_with_matching_executable() {
        let name = namespace();
        let _server = Server::start(&name).unwrap();
        let stop = AtomicBool::new(false);
        let pipe =
            open_client(&pipe_name(&name).unwrap(), &stop, Instant::now() + TIMEOUT).unwrap();
        let mut identity = Identity::current().unwrap();
        identity.sid = "S-1-5-18".into();
        assert_eq!(
            identity.pin_peer(&pipe, true).err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    fn child_powershell(script: &str) -> std::process::Child {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("powershell.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
    fn bounded_child_exit(mut child: std::process::Child) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                if !status.success() {
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "Owned foreign-peer fixture failed: {status}; stdout: {}; stderr: {}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Owned foreign-peer fixture timed out");
            }
            thread::sleep(POLL);
        }
    }
    #[test]
    fn server_rejects_foreign_executable_same_user_native_client() {
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let script = format!(
            "$p=[System.IO.Pipes.NamedPipeClientStream]::new('.','{name}.Launch',[System.IO.Pipes.PipeDirection]::InOut);$p.Connect(1000);$b=[System.Text.Encoding]::UTF8.GetBytes('{{}}');$p.Write($b,0,$b.Length);$r=$p.ReadByte();$p.Dispose();if($r -ne -1){{exit 2}}"
        );
        bounded_child_exit(child_powershell(&script));
        assert!(server.try_recv().is_none());
        forward(&name, "{}").unwrap();
        assert_eq!(server.try_recv().as_deref(), Some("{}"));
    }
    #[test]
    fn client_rejects_foreign_executable_same_user_native_server_before_write() {
        let name = namespace();
        let script = format!(
            "$p=[System.IO.Pipes.NamedPipeServerStream]::new('{name}.Launch',[System.IO.Pipes.PipeDirection]::InOut,1,[System.IO.Pipes.PipeTransmissionMode]::Message);$r=-1;try{{$p.WaitForConnection();$r=$p.ReadByte()}}catch{{if($_.Exception.GetBaseException() -isnot [System.IO.IOException]){{throw}}}}finally{{$p.Dispose()}};if($r -ne -1){{exit 2}}"
        );
        let child = child_powershell(&script);
        assert_eq!(
            forward(&name, "{}").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        bounded_child_exit(child);
    }
    #[test]
    fn rejects_namespace_injection_before_native_access() {
        for name in ["", r"\\remote\evil", "foo/bar", "name\0"] {
            assert_eq!(
                forward(name, "{}").unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                Server::start(name).err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
    #[test]
    fn same_executable_child_fixture() {
        if let Ok(namespace) = std::env::var("DISKBURROW_TEST_LAUNCH_NAMESPACE") {
            forward(&namespace, r#"{"root":"E:\\owned IPC\\second launch"}"#).unwrap();
        }
    }
    #[test]
    fn second_process_same_executable_delivers_only_to_owned_native_server() {
        use std::os::windows::process::CommandExt;
        let name = namespace();
        let server = Server::start(&name).unwrap();
        let test_module = module_path!().split_once("::").unwrap().1;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .creation_flags(CREATE_NO_WINDOW)
            .args([
                "--exact",
                &format!("{test_module}::same_executable_child_fixture"),
            ])
            .env("DISKBURROW_TEST_LAUNCH_NAMESPACE", &name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        bounded_child_exit(child);
        assert_eq!(
            server.try_recv().as_deref(),
            Some(r#"{"root":"E:\\owned IPC\\second launch"}"#)
        );
        assert!(server.try_recv().is_none());
    }
}
