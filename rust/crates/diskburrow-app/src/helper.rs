//! A separate, same-user elevated process that reads metadata and accepts no mutation commands.
#![allow(
    unsafe_code,
    reason = "Owned Win32 handles and pinned overlapped buffers at the native IPC boundary"
)]
use crate::{
    helper_args::HelperArgs,
    platform::{OwnedHandle, wide},
};
use anyhow::{Result, bail, ensure};
use diskburrow_engine::{LiveIndex, protocol};
use diskburrow_windows::Cancellation;
use disktree_core::scan::{ScanHandle, ScanOptions, ScanProgress};
use std::{
    io::{self, BufReader, BufWriter, Read, Write},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::Authorization::*,
    Security::*,
    Storage::FileSystem::*,
    System::{IO::*, Pipes::*, Threading::*},
    UI::{Shell::*, WindowsAndMessaging::SW_HIDE},
};

pub fn parse(args: &[String]) -> Result<Option<HelperArgs>> {
    crate::helper_args::parse(args).map_err(anyhow::Error::msg)
}
fn process_sid(process: HANDLE) -> Result<String> {
    let mut raw = std::ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } != 0,
        "Read process token: {}",
        io::Error::last_os_error()
    );
    let token = OwnedHandle(raw);
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    ensure!(size > 0 && size < 65536, "Invalid token size");
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } != 0,
        "Read token user: {}",
        io::Error::last_os_error()
    );
    let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = std::ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid) } != 0,
        "Read user SID"
    );
    let value = unsafe {
        let mut n = 0;
        while *sid.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(sid, n))
    };
    unsafe {
        LocalFree(sid.cast());
    }
    Ok(value)
}
pub fn is_elevated() -> Result<bool> {
    let mut raw = std::ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } != 0,
        "Read process token"
    );
    let token = OwnedHandle(raw);
    let mut value: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut size = 0;
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&mut value as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            )
        } != 0,
        "Read elevation state"
    );
    Ok(value.TokenIsElevated != 0)
}
fn image(process: HANDLE) -> Result<String> {
    let mut value = vec![0u16; 32768];
    let mut size = value.len() as u32;
    ensure!(
        unsafe { QueryFullProcessImageNameW(process, 0, value.as_mut_ptr(), &mut size) } != 0,
        "Read process image: {}",
        io::Error::last_os_error()
    );
    Ok(String::from_utf16(&value[..size as usize])?)
}
fn expected_image(process: HANDLE) -> Result<()> {
    ensure!(
        image(process)?.eq_ignore_ascii_case(
            std::env::current_exe()?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Executable path is not Unicode"))?
        ),
        "Unexpected helper process image"
    );
    Ok(())
}
struct Child(OwnedHandle);
impl Drop for Child {
    fn drop(&mut self) {
        // This is the retained handle returned by our own launch, never a PID lookup for termination.
        if unsafe { WaitForSingleObject(self.0.0, 3000) } == WAIT_TIMEOUT {
            unsafe {
                TerminateProcess(self.0.0, 1);
                WaitForSingleObject(self.0.0, 3000);
            }
        }
    }
}
struct Descriptor(*mut std::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Job {
    overlapped: OVERLAPPED,
    event: OwnedHandle,
    bytes: Vec<u8>,
    owner: Arc<OwnedHandle>,
}
impl Job {
    fn new(owner: Arc<OwnedHandle>, bytes: Vec<u8>) -> io::Result<Box<Self>> {
        let event = OwnedHandle::event(None)?;
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = event.0;
        Ok(Box::new(Self {
            overlapped,
            event,
            bytes,
            owner,
        }))
    }
    fn wait(
        job: Box<Self>,
        cancel: &Cancellation,
        deadline: Instant,
        child: Option<HANDLE>,
    ) -> io::Result<(Box<Self>, u32)> {
        loop {
            let mut transferred = 0;
            if unsafe { GetOverlappedResult(job.owner.0, &job.overlapped, &mut transferred, 0) }
                != 0
            {
                return Ok((job, transferred));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_INCOMPLETE as i32) {
                return Err(error);
            }
            let stopped = cancel.is_cancelled()
                || Instant::now() >= deadline
                || child.is_some_and(|h| unsafe { WaitForSingleObject(h, 0) } == WAIT_OBJECT_0);
            if stopped {
                unsafe {
                    CancelIoEx(job.owner.0, &job.overlapped);
                }
                let completed =
                    unsafe { WaitForSingleObject(job.event.0, 30_000) } == WAIT_OBJECT_0;
                if !completed {
                    // A pathological kernel completion must never write into freed stack/heap storage.
                    // The bounded 64 KiB job keeps its handle and memory alive until process shutdown.
                    Box::leak(job);
                }
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Helper connection or transfer cancelled/timed out",
                ));
            }
            match unsafe { WaitForSingleObject(job.event.0, 100) } {
                WAIT_OBJECT_0 | WAIT_TIMEOUT => {}
                _ => {
                    unsafe {
                        CancelIoEx(job.owner.0, &job.overlapped);
                        WaitForSingleObject(job.event.0, INFINITE);
                    }
                    return Err(io::Error::last_os_error());
                }
            }
        }
    }
}
#[derive(Clone)]
struct Pipe {
    handle: Arc<OwnedHandle>,
    cancel: Cancellation,
    deadline: Instant,
}
impl Pipe {
    fn transfer(&self, write: bool, data: &mut [u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.cancel.is_cancelled() || Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Helper transfer cancelled/timed out",
            ));
        }
        let size = data.len().min(65536);
        let bytes = if write {
            data[..size].to_vec()
        } else {
            vec![0; size]
        };
        let mut job = Job::new(self.handle.clone(), bytes)?;
        let ok = if write {
            unsafe {
                WriteFile(
                    self.handle.0,
                    job.bytes.as_ptr(),
                    size as u32,
                    std::ptr::null_mut(),
                    &mut job.overlapped,
                )
            }
        } else {
            unsafe {
                ReadFile(
                    self.handle.0,
                    job.bytes.as_mut_ptr(),
                    size as u32,
                    std::ptr::null_mut(),
                    &mut job.overlapped,
                )
            }
        };
        if ok == 0 && io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(io::Error::last_os_error());
        }
        let (job, count) = Job::wait(job, &self.cancel, self.deadline, None)?;
        let count = count as usize;
        if count > size {
            return Err(io::Error::other("Invalid pipe transfer count"));
        }
        if !write {
            data[..count].copy_from_slice(&job.bytes[..count]);
        }
        Ok(count)
    }
}
impl Read for Pipe {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.transfer(false, bytes)
    }
}
impl Write for Pipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(true, &mut bytes.to_vec())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn create_pipe(name: &str, sid: &str, cancel: Cancellation) -> Result<Pipe> {
    let dacl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
    let mut raw = std::ptr::null_mut();
    ensure!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                dacl.as_ptr(),
                1,
                &mut raw,
                std::ptr::null_mut(),
            )
        } != 0,
        "Create pipe security descriptor"
    );
    let descriptor = Descriptor(raw);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let name = wide(&format!(r"\\.\pipe\{name}"));
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            &attributes,
        )
    };
    ensure!(
        handle != INVALID_HANDLE_VALUE,
        "Create private pipe: {}",
        io::Error::last_os_error()
    );
    Ok(Pipe {
        handle: Arc::new(OwnedHandle(handle)),
        cancel,
        deadline: Instant::now() + Duration::from_secs(600),
    })
}
fn launch(args: &HelperArgs) -> Result<Child> {
    let executable = wide(
        std::env::current_exe()?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Executable path is not Unicode"))?,
    );
    let verb = wide("runas");
    let parameters = wide(&format!(
        "--mft-helper --pipe {} --nonce {} --root {} --parent {}",
        args.pipe, args.nonce, args.root, args.parent
    ));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpFile = executable.as_ptr();
    info.lpVerb = verb.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            bail!("FastScan.UacCancelled");
        }
        return Err(error.into());
    }
    ensure!(
        !info.hProcess.is_null(),
        "UAC launch returned no process handle"
    );
    Ok(Child(OwnedHandle(info.hProcess)))
}
pub fn scan(root: &str, cancel: Cancellation, progress: Arc<ScanProgress>) -> Result<LiveIndex> {
    ensure!(
        !is_elevated()?,
        "The application host must remain unelevated"
    );
    ensure!(
        root.len() == 3 && crate::platform::is_local_path(root),
        "A whole confirmed local NTFS volume is required"
    );
    let parent = unsafe { GetCurrentProcessId() };
    let nonce = format!("{}{}", uuid_part(), uuid_part());
    let args = HelperArgs {
        pipe: format!("DiskBurrowRustMft-{parent}-{}", uuid_part()),
        nonce,
        root: root.into(),
        parent,
    };
    crate::helper_args::parse(&[
        "--mft-helper".into(),
        "--pipe".into(),
        args.pipe.clone(),
        "--nonce".into(),
        args.nonce.clone(),
        "--root".into(),
        args.root.clone(),
        "--parent".into(),
        parent.to_string(),
    ])
    .map_err(anyhow::Error::msg)?;
    let sid = process_sid(unsafe { GetCurrentProcess() })?;
    let pipe = create_pipe(&args.pipe, &sid, cancel.clone())?;
    let child = launch(&args)?;
    ensure!(!cancel.is_cancelled(), "Cancelled");
    let mut job = Job::new(pipe.handle.clone(), vec![])?;
    let connected = unsafe { ConnectNamedPipe(pipe.handle.0, &mut job.overlapped) };
    if connected == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_IO_PENDING as i32) {
            Job::wait(
                job,
                &cancel,
                Instant::now() + Duration::from_secs(90),
                Some(child.0.0),
            )?;
        } else if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
            return Err(error.into());
        }
    }
    let mut client = 0;
    ensure!(
        unsafe { GetNamedPipeClientProcessId(pipe.handle.0, &mut client) } != 0
            && client == unsafe { GetProcessId(child.0.0) },
        "Unexpected pipe client"
    );
    protocol::validate_peer(&sid, &process_sid(child.0.0)?, &args.nonce, &args.nonce)?;
    expected_image(child.0.0)?;
    let mut reader = BufReader::with_capacity(65536, pipe);
    protocol::read_hello(&mut reader, &args.nonce, root)?;
    let mut frames = 0;
    loop {
        let mut frame = [0; 1];
        reader.read_exact(&mut frame)?;
        match frame[0] {
            1 => {
                frames += 1;
                ensure!(frames <= 10000, "Too many progress frames");
                let mut bytes = [0; 32];
                reader.read_exact(&mut bytes)?;
                let value = |i| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
                let (files, dirs, size, errors) = (value(0), value(8), value(16), value(24));
                ensure!(
                    files <= 20_000_000
                        && dirs <= 20_000_000
                        && size <= i64::MAX as u64
                        && errors <= 20_000_000,
                    "Invalid helper progress"
                );
                progress.update_remote(files, dirs, size, errors);
            }
            2 => {
                let index = protocol::read_index(&mut reader, root)?;
                ensure!(!cancel.is_cancelled(), "Cancelled");
                return Ok(index);
            }
            3 => {
                let _detail = protocol::read_error(&mut reader)?;
                bail!("FastScan.ReadFailed");
            }
            _ => bail!("Unknown helper frame"),
        }
    }
}
fn uuid_part() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub fn run(args: HelperArgs) -> Result<()> {
    ensure!(
        is_elevated()?,
        "The MFT reader needs administrator approval"
    );
    let parent = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            args.parent,
        )
    };
    ensure!(
        !parent.is_null(),
        "The original parent process is unavailable"
    );
    let parent = Arc::new(OwnedHandle(parent));
    let sid = process_sid(unsafe { GetCurrentProcess() })?;
    protocol::validate_peer(&sid, &process_sid(parent.0)?, &args.nonce, &args.nonce)?;
    expected_image(parent.0)?;
    let name = wide(&format!(r"\\.\pipe\{}", args.pipe));
    let deadline = Instant::now() + Duration::from_secs(90);
    let raw = loop {
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            break handle;
        }
        ensure!(
            Instant::now() < deadline
                && unsafe { WaitForSingleObject(parent.0, 0) } == WAIT_TIMEOUT,
            "The original pipe server is unavailable"
        );
        thread::sleep(Duration::from_millis(100));
    };
    let cancel = Cancellation::default();
    let pipe = Pipe {
        handle: Arc::new(OwnedHandle(raw)),
        cancel: cancel.clone(),
        deadline: Instant::now() + Duration::from_secs(600),
    };
    let mut server = 0;
    ensure!(
        unsafe { GetNamedPipeServerProcessId(pipe.handle.0, &mut server) } != 0
            && server == args.parent,
        "Unexpected pipe server"
    );
    let handle = ScanHandle::spawn(
        args.root.clone().into(),
        ScanOptions {
            mft_only: true,
            ..Default::default()
        },
    );
    let child_progress = handle.progress.clone();
    let watcher_cancel = cancel.clone();
    let mut watch_pipe = pipe.clone();
    let watched_parent = parent.clone();
    let watcher = thread::spawn(move || {
        let mut control = [0; 1];
        let _ = watch_pipe.read(&mut control);
        watcher_cancel.cancel();
        child_progress.cancel();
        let _ = watched_parent;
    });
    let mut writer = BufWriter::with_capacity(65536, pipe);
    let mut sent_index = false;
    let result = (|| -> Result<()> {
        protocol::write_hello(&mut writer, &args.nonce, &args.root)?;
        let mut last = Instant::now();
        let tree = loop {
            if cancel.is_cancelled() || unsafe { WaitForSingleObject(parent.0, 0) } != WAIT_TIMEOUT
            {
                handle.cancel();
                bail!("Parent disappeared or cancelled the read");
            }
            if let Some(result) = handle.poll() {
                break result?;
            }
            if last.elapsed() >= Duration::from_millis(500) {
                let progress = handle.progress.snapshot();
                writer.write_all(&[1])?;
                for number in [
                    progress.files,
                    progress.dirs,
                    progress.bytes,
                    progress.errors,
                ] {
                    writer.write_all(&number.to_le_bytes())?;
                }
                writer.flush()?;
                last = Instant::now();
            }
            thread::sleep(Duration::from_millis(60));
        };
        let index = LiveIndex::from_tree(args.root, tree)?;
        ensure!(!cancel.is_cancelled(), "Cancelled");
        sent_index = true;
        writer.write_all(&[2])?;
        protocol::write_index(&mut writer, &index)?;
        writer.flush()?;
        Ok(())
    })();
    if result.is_err() && !sent_index && !cancel.is_cancelled() {
        let _ = writer.write_all(&[3]).and_then(|_| {
            protocol::write_error(&mut writer, "FastScan.ReadFailed").map_err(io::Error::other)
        });
    }
    cancel.cancel();
    handle.cancel();
    let _ = watcher.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Pipe, Pipe) {
        let name = format!("DiskBurrowRust-test-{}", uuid_part());
        let sid = process_sid(unsafe { GetCurrentProcess() }).unwrap();
        let server = create_pipe(&name, &sid, Cancellation::default()).unwrap();
        let name = wide(&format!(r"\\.\pipe\{name}"));
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(raw, INVALID_HANDLE_VALUE);
        let mut job = Job::new(server.handle.clone(), vec![]).unwrap();
        let ok = unsafe { ConnectNamedPipe(server.handle.0, &mut job.overlapped) };
        if ok == 0 {
            let code = io::Error::last_os_error().raw_os_error();
            if code == Some(ERROR_IO_PENDING as i32) {
                Job::wait(job, &server.cancel, server.deadline, None).unwrap();
            } else {
                assert_eq!(code, Some(ERROR_PIPE_CONNECTED as i32));
            }
        }
        let client = Pipe {
            handle: Arc::new(OwnedHandle(raw)),
            cancel: Cancellation::default(),
            deadline: Instant::now() + Duration::from_secs(15),
        };
        (server, client)
    }
    #[test]
    fn private_overlapped_pipe_transfers_multiple_chunks_and_reports_peer_pid() {
        let (mut server, mut client) = pair();
        let mut pid = 0;
        assert_ne!(
            unsafe { GetNamedPipeClientProcessId(server.handle.0, &mut pid) },
            0
        );
        assert_eq!(pid, unsafe { GetCurrentProcessId() });
        let bytes: Vec<u8> = (0..400000).map(|n| (n % 251) as u8).collect();
        let expected = bytes.clone();
        let writer = thread::spawn(move || client.write_all(&bytes).unwrap());
        let mut output = vec![0; expected.len()];
        server.read_exact(&mut output).unwrap();
        writer.join().unwrap();
        assert_eq!(output, expected);
    }
    #[test]
    fn cancellation_interrupts_pending_native_read_without_freeing_its_buffer_early() {
        let (mut server, _client) = pair();
        let cancel = server.cancel.clone();
        let read = thread::spawn(move || {
            let mut byte = [0; 1];
            server.read(&mut byte)
        });
        thread::sleep(Duration::from_millis(30));
        let started = Instant::now();
        cancel.cancel();
        assert_eq!(
            read.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
