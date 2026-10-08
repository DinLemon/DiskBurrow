//! Local, bounded Git inspection. No operation here authorizes deletion.
use diskburrow_windows::Cancellation;
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryState {
    Repository,
    NotRepository,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitUncertainty {
    GitUnavailable,
    UnsafeConfiguration,
    CommandFailed,
    TimedOut,
    OutputLimit,
    Cancelled,
    NoUpstream,
    DetachedHead,
    UnsupportedRepository,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInspection {
    pub repository: RepositoryState,
    pub changed: Option<u64>,
    pub untracked: Option<u64>,
    pub stash: Option<u64>,
    /// Relative only to the last locally known upstream; never fetched.
    pub ahead: Option<u64>,
    pub uncertainty: Option<GitUncertainty>,
}
fn unknown(reason: GitUncertainty) -> GitInspection {
    GitInspection {
        repository: RepositoryState::Unknown,
        changed: None,
        untracked: None,
        stash: None,
        ahead: None,
        uncertainty: Some(reason),
    }
}
pub fn inspect(path: &str, cancel: &Cancellation) -> GitInspection {
    let deadline = Instant::now() + Duration::from_secs(5);
    if cancel.is_cancelled() {
        return unknown(GitUncertainty::Cancelled);
    }
    let Some(path) = diskburrow_windows::normalize_local_path(path) else {
        return unknown(GitUncertainty::UnsupportedRepository);
    };
    let Ok(directory) = dunce::canonicalize(path) else {
        return unknown(GitUncertainty::CommandFailed);
    };
    if !directory.is_dir() {
        if directory.is_file() {
            return not_repository();
        }
        return unknown(GitUncertainty::CommandFailed);
    }
    // Inspect only a directly selected checkout, never an ancestor repository.
    match direct_checkout(&directory) {
        Ok(true) => {}
        Ok(false) => {
            return not_repository();
        }
        Err(reason) => return unknown(reason),
    }
    let Some(git) = discover_git(&directory) else {
        return unknown(GitUncertainty::GitUnavailable);
    };
    inspect_with_git(&directory, &git, cancel, deadline)
}
fn direct_checkout(directory: &Path) -> Result<bool, GitUncertainty> {
    let marker = directory.join(".git");
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if directory.join("HEAD").is_file() && directory.join("objects").is_dir() {
                return Err(GitUncertainty::UnsupportedRepository);
            }
            return Ok(false);
        }
        Err(_) => return Err(GitUncertainty::CommandFailed),
    };
    if metadata.file_attributes() & 0x400 != 0 {
        return Err(GitUncertainty::UnsupportedRepository);
    }
    if metadata.is_dir() {
        return Ok(true);
    }
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err(GitUncertainty::UnsupportedRepository);
    }
    let mut pointer = String::new();
    fs::File::open(marker)
        .and_then(|file| file.take(4097).read_to_string(&mut pointer))
        .map_err(|_| GitUncertainty::CommandFailed)?;
    let target = pointer
        .strip_prefix("gitdir: ")
        .ok_or(GitUncertainty::UnsupportedRepository)?
        .trim_end_matches(['\r', '\n']);
    let target = directory.join(target);
    if diskburrow_windows::normalize_local_path(&target.to_string_lossy()).is_none() {
        return Err(GitUncertainty::UnsupportedRepository);
    }
    Ok(true)
}
fn discover_git(directory: &Path) -> Option<PathBuf> {
    // Never resolve a relative PATH entry, the current directory or a Git executable
    // supplied by the inspected tree. Only an installed, absolute git.exe is used.
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|p| {
            p.is_absolute()
                && diskburrow_windows::normalize_local_path(&p.to_string_lossy()).is_some()
        })
        .find_map(|base| {
            // Git for Windows' cmd/git.exe is a launcher. Use its adjacent built-in
            // executable so our one-process job can refuse *all* subprocesses.
            let builtin = base.parent()?.join("mingw64/bin/git.exe");
            let candidate = dunce::canonicalize(if builtin.is_file() {
                builtin
            } else {
                base.join("git.exe")
            })
            .ok()?;
            (candidate.is_file()
                && diskburrow_windows::normalize_local_path(&candidate.to_string_lossy()).is_some()
                && !diskburrow_windows::is_within(
                    &candidate.to_string_lossy(),
                    &directory.to_string_lossy(),
                ))
            .then_some(candidate)
        })
}
fn inspect_with_git(
    directory: &Path,
    git: &Path,
    cancel: &Cancellation,
    deadline: Instant,
) -> GitInspection {
    let mut remaining_output = 1024 * 1024;
    let result = (|| {
        let discovery = run_git(
            git,
            directory,
            &["rev-parse", "--show-toplevel"],
            cancel,
            deadline,
            &mut remaining_output,
        )?;
        if discovery.code != 0 {
            if discovery.output
                == b"fatal: not a git repository (or any of the parent directories): .git\n"
            {
                return Ok(not_repository());
            }
            if discovery
                .output
                .starts_with(b"fatal: this operation must be run in a work tree")
            {
                return Err(GitUncertainty::UnsupportedRepository);
            }
            return Err(GitUncertainty::CommandFailed);
        }
        let root = std::str::from_utf8(&discovery.output)
            .ok()
            .and_then(|path| {
                diskburrow_windows::normalize_local_path(path.trim_end_matches(['\r', '\n']))
            })
            .ok_or(GitUncertainty::CommandFailed)?;
        if !diskburrow_windows::equals_path(&root, &directory.to_string_lossy()) {
            return Err(GitUncertainty::UnsupportedRepository);
        }
        let config = run_git(
            git,
            directory,
            &["config", "--null", "--list", "--no-includes"],
            cancel,
            deadline,
            &mut remaining_output,
        )?;
        if config.code != 0 {
            return Err(GitUncertainty::CommandFailed);
        }
        if unsafe_configuration(&config.output) {
            return Err(GitUncertainty::UnsafeConfiguration);
        }
        let status = run_git(
            git,
            directory,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "--show-stash",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
                "--no-renames",
            ],
            cancel,
            deadline,
            &mut remaining_output,
        )?;
        if status.code != 0 {
            return Err(GitUncertainty::CommandFailed);
        }
        let parsed = parse_status(&status.output)?;
        // Refuse a result whose repository configuration changed while status ran.
        // The job additionally prevents filters added in this interval from executing.
        let after = run_git(
            git,
            directory,
            &["config", "--null", "--list", "--no-includes"],
            cancel,
            deadline,
            &mut remaining_output,
        )?;
        if unsafe_configuration(&after.output) {
            return Err(GitUncertainty::UnsafeConfiguration);
        }
        if after.code != 0 || after.output != config.output {
            return Err(GitUncertainty::CommandFailed);
        }
        if cancel.is_cancelled() {
            return Err(GitUncertainty::Cancelled);
        }
        Ok(parsed)
    })();
    result.unwrap_or_else(unknown)
}
fn not_repository() -> GitInspection {
    GitInspection {
        repository: RepositoryState::NotRepository,
        changed: None,
        untracked: None,
        stash: None,
        ahead: None,
        uncertainty: None,
    }
}
fn unsafe_configuration(output: &[u8]) -> bool {
    output
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .any(|record| {
            let key =
                String::from_utf8_lossy(record.split(|b| *b == b'\n').next().unwrap_or_default())
                    .to_ascii_lowercase();
            key == "include.path"
                || key.starts_with("includeif.")
                || (key.starts_with("filter.")
                    && [".clean", ".smudge", ".process"]
                        .iter()
                        .any(|suffix| key.ends_with(suffix)))
        })
}
fn parse_status(output: &[u8]) -> Result<GitInspection, GitUncertainty> {
    let mut result = GitInspection {
        repository: RepositoryState::Repository,
        changed: Some(0),
        untracked: Some(0),
        stash: Some(0),
        ahead: None,
        uncertainty: Some(GitUncertainty::NoUpstream),
    };
    let mut upstream = false;
    let mut detached = false;
    let mut oid_seen = false;
    let mut head_seen = false;
    let mut records = output.split(|b| *b == 0).filter(|v| !v.is_empty());
    while let Some(record) = records.next() {
        if let Some(oid) = record.strip_prefix(b"# branch.oid ") {
            if oid != b"(initial)"
                && !([40, 64].contains(&oid.len()) && oid.iter().all(u8::is_ascii_hexdigit))
            {
                return Err(GitUncertainty::CommandFailed);
            }
            oid_seen = true;
        } else if let Some(head) = record.strip_prefix(b"# branch.head ") {
            if head.is_empty() {
                return Err(GitUncertainty::CommandFailed);
            }
            head_seen = true;
            detached = head == b"(detached)";
        } else if record.starts_with(b"# branch.upstream ") {
            upstream = true;
        } else if let Some(ab) = record.strip_prefix(b"# branch.ab +") {
            let number = ab
                .split(|b| *b == b' ')
                .next()
                .ok_or(GitUncertainty::CommandFailed)?;
            result.ahead = Some(parse_number(number)?);
        } else if let Some(stash) = record.strip_prefix(b"# stash ") {
            result.stash = Some(parse_number(stash)?);
        } else if record.starts_with(b"1 ") || record.starts_with(b"u ") {
            *result.changed.as_mut().unwrap() += 1;
        } else if record.starts_with(b"2 ") {
            *result.changed.as_mut().unwrap() += 1;
            records.next().ok_or(GitUncertainty::CommandFailed)?;
        } else if record.starts_with(b"? ") {
            *result.untracked.as_mut().unwrap() += 1;
        } else if !record.starts_with(b"# ") {
            return Err(GitUncertainty::CommandFailed);
        }
    }
    if !oid_seen || !head_seen {
        return Err(GitUncertainty::CommandFailed);
    }
    if detached {
        result.uncertainty = Some(GitUncertainty::DetachedHead);
        result.ahead = None;
    } else if upstream && result.ahead.is_some() {
        result.uncertainty = None;
    } else if upstream {
        result.uncertainty = Some(GitUncertainty::CommandFailed);
    }
    Ok(result)
}
fn parse_number(bytes: &[u8]) -> Result<u64, GitUncertainty> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(GitUncertainty::CommandFailed)
}
struct ProcessOutput {
    code: u32,
    output: Vec<u8>,
}
fn run_git(
    git: &Path,
    directory: &Path,
    args: &[&str],
    cancel: &Cancellation,
    deadline: Instant,
    remaining_output: &mut usize,
) -> Result<ProcessOutput, GitUncertainty> {
    let mut arguments = vec![
        "--no-pager",
        "--no-optional-locks",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.hooksPath=NUL",
        "-c",
        "core.attributesFile=NUL",
        "-c",
        "core.untrackedCache=false",
        "-c",
        "diff.external=",
        "-c",
        "maintenance.auto=false",
        "-c",
        "gc.auto=0",
    ];
    arguments.extend_from_slice(args);
    let result = process::run(
        git,
        directory,
        &arguments,
        cancel,
        deadline,
        *remaining_output,
        1,
    )?;
    *remaining_output -= result.output.len();
    Ok(result)
}

mod process {
    use super::*;
    use std::{
        mem::{size_of, zeroed},
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GetLastError, HANDLE,
            HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
            ReadFile,
        },
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectExtendedLimitInformation, SetInformationJobObject,
            },
            Pipes::{CreatePipe, PeekNamedPipe},
            SystemInformation::GetWindowsDirectoryW,
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
                DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
                InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
                TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn checked(handle: HANDLE) -> Result<Handle, GitUncertainty> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(GitUncertainty::CommandFailed)
        } else {
            Ok(Handle(handle))
        }
    }
    // Windows argv quoting, with an explicit application path. No shell is involved.
    fn quote(value: &OsStr) -> Vec<u16> {
        let mut result = vec![b'"' as u16];
        let mut slashes = 0;
        for ch in value.encode_wide() {
            if ch == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            result.extend(std::iter::repeat_n(
                b'\\' as u16,
                if ch == b'"' as u16 {
                    slashes * 2 + 1
                } else {
                    slashes
                },
            ));
            slashes = 0;
            result.push(ch);
        }
        result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        result.push(b'"' as u16);
        result
    }
    fn environment(git: &Path, directory: &Path) -> Result<Vec<u16>, GitUncertainty> {
        // Obtain the Windows directory from the OS, rather than inheriting a
        // potentially poisoned SystemRoot or DLL-search PATH from the caller.
        let mut windows_directory = vec![0u16; 32768];
        let length = unsafe {
            GetWindowsDirectoryW(
                windows_directory.as_mut_ptr(),
                windows_directory.len() as u32,
            )
        } as usize;
        if length == 0 || length >= windows_directory.len() {
            return Err(GitUncertainty::CommandFailed);
        }
        let system_root = std::ffi::OsString::from_wide(&windows_directory[..length]);
        let system32 = PathBuf::from(&system_root).join("System32");
        let path = std::env::join_paths([git.parent().unwrap_or(&system32), system32.as_path()])
            .unwrap_or_default();
        let mut entries = vec![
            ("SystemRoot", system_root.clone()),
            ("WINDIR", system_root),
            ("PATH", path),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_CONFIG_SYSTEM", "NUL".into()),
            ("GIT_CONFIG_GLOBAL", "NUL".into()),
            ("GIT_TERMINAL_PROMPT", "0".into()),
            ("GIT_OPTIONAL_LOCKS", "0".into()),
            ("GIT_ATTR_NOSYSTEM", "1".into()),
            ("GIT_NO_REPLACE_OBJECTS", "1".into()),
            ("GIT_NO_LAZY_FETCH", "1".into()),
            ("LC_ALL", "C".into()),
        ];
        if let Some(parent) = directory.parent() {
            entries.push(("GIT_CEILING_DIRECTORIES", parent.as_os_str().to_owned()));
        }
        entries.sort_by_key(|(name, _)| name.to_ascii_uppercase());
        let mut block = Vec::new();
        for (name, value) in entries {
            block.extend(OsStr::new(name).encode_wide());
            block.push(b'=' as u16);
            block.extend(value.encode_wide());
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }
    pub(super) fn run(
        executable: &Path,
        directory: &Path,
        args: &[&str],
        cancel: &Cancellation,
        deadline: Instant,
        output_limit: usize,
        active_processes: u32,
    ) -> Result<ProcessOutput, GitUncertainty> {
        if cancel.is_cancelled() {
            return Err(GitUncertainty::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(GitUncertainty::TimedOut);
        }
        // All native pointers reference live local storage. The process cannot execute
        // until assigned to the non-inherited job. Only the explicitly listed pipe/NUL
        // handles are inherited; the job handle never escapes this scope.
        unsafe {
            let job = checked(CreateJobObjectW(null(), null()))?;
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = active_processes;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(GitUncertainty::CommandFailed);
            }
            let security = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: null_mut(),
                bInheritHandle: 1,
            };
            let (mut read, mut write) = (null_mut(), null_mut());
            if CreatePipe(&mut read, &mut write, &security, 0) == 0 {
                return Err(GitUncertainty::CommandFailed);
            }
            let read = Handle(read);
            let write = Handle(write);
            if SetHandleInformation(read.0, HANDLE_FLAG_INHERIT, 0) == 0 {
                return Err(GitUncertainty::CommandFailed);
            }
            let stdin = checked(CreateFileW(
                wide(OsStr::new("NUL")).as_ptr(),
                0x80000000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &security,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            ))?;
            let mut attr_size = 0;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attr_size);
            let mut storage = vec![0usize; attr_size.div_ceil(size_of::<usize>())];
            let attributes = storage.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(attributes, 1, 0, &mut attr_size) == 0 {
                return Err(GitUncertainty::CommandFailed);
            }
            struct Attributes(windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST);
            impl Drop for Attributes {
                fn drop(&mut self) {
                    unsafe {
                        DeleteProcThreadAttributeList(self.0);
                    }
                }
            }
            let attributes = Attributes(attributes);
            let inherited = [write.0, stdin.0];
            if UpdateProcThreadAttribute(
                attributes.0,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_ptr().cast(),
                size_of_val(&inherited),
                null_mut(),
                null(),
            ) == 0
            {
                return Err(GitUncertainty::CommandFailed);
            }
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = stdin.0;
            startup.StartupInfo.hStdOutput = write.0;
            startup.StartupInfo.hStdError = write.0;
            startup.lpAttributeList = attributes.0;
            let executable_wide = wide(executable.as_os_str());
            let directory_wide = wide(directory.as_os_str());
            let mut command = quote(executable.as_os_str());
            for arg in args {
                command.push(b' ' as u16);
                command.extend(quote(OsStr::new(arg)));
            }
            command.push(0);
            let environment = environment(executable, directory)?;
            let mut information: PROCESS_INFORMATION = zeroed();
            if CreateProcessW(
                executable_wide.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                directory_wide.as_ptr(),
                &startup.StartupInfo,
                &mut information,
            ) == 0
            {
                return Err(
                    if matches!(GetLastError(), ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
                        GitUncertainty::GitUnavailable
                    } else {
                        GitUncertainty::CommandFailed
                    },
                );
            }
            let child = Handle(information.hProcess);
            let thread = Handle(information.hThread);
            if AssignProcessToJobObject(job.0, child.0) == 0 {
                TerminateProcess(child.0, 1);
                WaitForSingleObject(child.0, 1000);
                return Err(GitUncertainty::CommandFailed);
            }
            drop(write);
            drop(stdin);
            if ResumeThread(thread.0) == u32::MAX {
                return Err(GitUncertainty::CommandFailed);
            }
            drop(thread);
            let mut output = Vec::new();
            loop {
                if cancel.is_cancelled() {
                    return Err(GitUncertainty::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(GitUncertainty::TimedOut);
                }
                let mut available = 0;
                let pipe_open = PeekNamedPipe(
                    read.0,
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                ) != 0;
                if pipe_open && available > 0 {
                    let mut buffer = [0u8; 8192];
                    let mut count = 0;
                    if ReadFile(
                        read.0,
                        buffer.as_mut_ptr(),
                        available.min(buffer.len() as u32),
                        &mut count,
                        null_mut(),
                    ) == 0
                    {
                        return Err(GitUncertainty::CommandFailed);
                    }
                    if output.len() + count as usize > output_limit {
                        return Err(GitUncertainty::OutputLimit);
                    }
                    output.extend_from_slice(&buffer[..count as usize]);
                    continue;
                }
                match WaitForSingleObject(child.0, 0) {
                    WAIT_OBJECT_0 => {
                        let mut code = 0;
                        if GetExitCodeProcess(child.0, &mut code) == 0 {
                            return Err(GitUncertainty::CommandFailed);
                        }
                        return Ok(ProcessOutput { code, output });
                    }
                    WAIT_TIMEOUT => std::thread::sleep(Duration::from_millis(5)),
                    _ => return Err(GitUncertainty::CommandFailed),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, process::Command};
    fn fixture() -> tempfile::TempDir {
        let temp = std::env::var_os("TEMP").expect("E: TEMP required");
        assert!(temp.to_string_lossy().starts_with("E:"));
        tempfile::Builder::new()
            .prefix("diskburrow-git-")
            .tempdir_in(temp)
            .unwrap()
    }
    fn git(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git.exe")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "core.hooksPath=NUL",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn repo() -> tempfile::TempDir {
        let temp = fixture();
        git(temp.path(), &["init", "-b", "main"]);
        fs::write(temp.path().join("tracked.txt"), "original\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-m", "initial"]);
        temp
    }
    fn check(path: &Path) -> GitInspection {
        inspect(path.to_str().unwrap(), &Cancellation::default())
    }
    #[test]
    fn clean_repository_reports_no_upstream_without_claiming_unpushed_clean() {
        let temp = repo();
        let actual = check(temp.path());
        assert_eq!(actual.repository, RepositoryState::Repository);
        assert_eq!(
            (actual.changed, actual.untracked, actual.stash, actual.ahead),
            (Some(0), Some(0), Some(0), None)
        );
        assert_eq!(actual.uncertainty, Some(GitUncertainty::NoUpstream));
    }
    #[test]
    fn tracked_untracked_and_stash_are_observed_locally() {
        let temp = repo();
        fs::write(temp.path().join("tracked.txt"), "modified\n").unwrap();
        fs::write(temp.path().join("untracked.txt"), "new\n").unwrap();
        let actual = check(temp.path());
        assert_eq!((actual.changed, actual.untracked), (Some(1), Some(1)));
        git(temp.path(), &["stash", "push", "-u", "-m", "fixture"]);
        assert_eq!(check(temp.path()).stash, Some(1));
    }
    #[test]
    fn ahead_is_measured_against_existing_local_tracking_reference() {
        let temp = repo();
        git(
            temp.path(),
            &["update-ref", "refs/remotes/origin/main", "HEAD"],
        );
        git(
            temp.path(),
            &[
                "config",
                "remote.origin.url",
                "https://example.invalid/must-not-fetch",
            ],
        );
        git(
            temp.path(),
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        );
        git(temp.path(), &["config", "branch.main.remote", "origin"]);
        git(
            temp.path(),
            &["config", "branch.main.merge", "refs/heads/main"],
        );
        git(temp.path(), &["commit", "--allow-empty", "-m", "ahead"]);
        let actual = check(temp.path());
        assert_eq!(actual.ahead, Some(1));
        assert_eq!(actual.uncertainty, None);
    }
    #[test]
    fn unsafe_clean_filter_is_unknown_and_never_executes_marker() {
        let temp = repo();
        let marker = temp.path().join("FILTER_EXECUTED");
        let command = format!("cmd.exe /c echo executed > {}", marker.display());
        git(temp.path(), &["config", "filter.hostile.clean", &command]);
        fs::write(temp.path().join(".gitattributes"), "*.txt filter=hostile\n").unwrap();
        fs::write(temp.path().join("tracked.txt"), "trigger\n").unwrap();
        let actual = check(temp.path());
        assert_eq!(
            actual.uncertainty,
            Some(GitUncertainty::UnsafeConfiguration)
        );
        assert_eq!(actual.changed, None);
        assert!(!marker.exists());
    }
    #[test]
    fn filter_added_after_config_preflight_cannot_execute_in_native_job() {
        let temp = repo();
        let marker = temp.path().join("FILTER_ESCAPED");
        let command = format!("cmd.exe /c echo executed > {}", marker.display());
        git(temp.path(), &["config", "filter.hostile.clean", &command]);
        git(temp.path(), &["config", "filter.hostile.required", "true"]);
        fs::write(temp.path().join(".gitattributes"), "*.txt filter=hostile\n").unwrap();
        // Equal length forces Git to hash the file through the configured clean
        // filter instead of deciding it is modified solely from the stat size.
        fs::write(temp.path().join("tracked.txt"), "changed!\n").unwrap();
        let executable = discover_git(temp.path()).unwrap();
        // Exercise the status command directly, as if the hostile filter appeared
        // after the preliminary config read. The job remains the execution boundary.
        let output = run_git(
            &executable,
            temp.path(),
            &["status", "--porcelain=v2", "--branch", "--show-stash", "-z"],
            &Cancellation::default(),
            Instant::now() + Duration::from_secs(5),
            &mut (1024 * 1024),
        )
        .unwrap();
        assert!(!marker.exists());
        assert!(
            output.code != 0 || parse_status(&output.output).is_err(),
            "blocked filter must not publish verified status: {}",
            String::from_utf8_lossy(&output.output)
        );
    }
    #[test]
    fn ordinary_directory_is_not_a_repository() {
        let temp = fixture();
        assert_eq!(
            check(temp.path()).repository,
            RepositoryState::NotRepository
        );
    }
    #[test]
    fn selection_child_directory_does_not_inspect_ancestor_repository() {
        let temp = repo();
        let child = temp.path().join("ordinary-child");
        fs::create_dir(&child).unwrap();
        fs::write(temp.path().join("tracked.txt"), "ancestor changed\n").unwrap();
        let actual = check(&child);
        assert_eq!(actual.repository, RepositoryState::NotRepository);
        assert_eq!(actual.changed, None);
    }
    #[test]
    fn selected_ordinary_file_is_not_a_repository() {
        let temp = repo();
        let actual = check(&temp.path().join("tracked.txt"));
        assert_eq!(actual.repository, RepositoryState::NotRepository);
        assert_eq!(actual.changed, None);
        assert_eq!(actual.uncertainty, None);
    }
    #[test]
    fn empty_or_incomplete_porcelain_is_unknown_instead_of_clean() {
        assert!(parse_status(b"").is_err());
        assert!(parse_status(b"# branch.head main\0").is_err());
    }
    #[test]
    fn missing_git_is_unknown_and_never_not_repository() {
        let temp = repo();
        let actual = inspect_with_git(
            temp.path(),
            &temp.path().join("missing-git.exe"),
            &Cancellation::default(),
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(actual.repository, RepositoryState::Unknown);
        assert_eq!(actual.uncertainty, Some(GitUncertainty::GitUnavailable));
    }
    #[test]
    fn cancellation_never_publishes_clean() {
        let temp = repo();
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            inspect(temp.path().to_str().unwrap(), &cancel).uncertainty,
            Some(GitUncertainty::Cancelled)
        );
    }
    #[test]
    fn excessive_configuration_is_unknown_with_bounded_output() {
        let temp = repo();
        let config = temp.path().join(".git/config");
        let original = fs::read_to_string(&config).unwrap();
        fs::write(
            config,
            format!(
                "{original}\n[inspection]\npadding = {}\n",
                "A".repeat(1024 * 1024)
            ),
        )
        .unwrap();
        let actual = check(temp.path());
        assert_eq!(actual.uncertainty, Some(GitUncertainty::OutputLimit));
        assert_eq!(actual.changed, None);
    }
    #[test]
    fn malformed_config_is_unknown_and_never_not_repository() {
        let temp = repo();
        fs::write(temp.path().join(".git/config"), "[malformed\n").unwrap();
        let actual = check(temp.path());
        assert_eq!(actual.repository, RepositoryState::Unknown);
        assert_eq!(actual.uncertainty, Some(GitUncertainty::CommandFailed));
    }
    #[test]
    fn include_config_is_rejected_without_following_it() {
        let temp = repo();
        git(
            temp.path(),
            &["config", "include.path", "must-not-follow.cfg"],
        );
        assert_eq!(
            check(temp.path()).uncertainty,
            Some(GitUncertainty::UnsafeConfiguration)
        );
    }
    #[test]
    fn detached_head_reports_local_changes_but_unpushed_unknown() {
        let temp = repo();
        git(temp.path(), &["checkout", "--detach"]);
        let actual = check(temp.path());
        assert_eq!(actual.changed, Some(0));
        assert_eq!(actual.ahead, None);
        assert_eq!(actual.uncertainty, Some(GitUncertainty::DetachedHead));
    }
    #[test]
    fn inherited_git_environment_and_global_filter_cannot_redirect_inspection() {
        let selected = repo();
        let other = repo();
        fs::write(selected.path().join("tracked.txt"), "modified\n").unwrap();
        fs::write(
            selected.path().join(".gitattributes"),
            "*.txt filter=hostile\n",
        )
        .unwrap();
        let global = selected.path().join(".git/hostile-global.cfg");
        let marker = selected.path().join("ENVIRONMENT_ESCAPED");
        let command = format!("cmd.exe /c echo executed > {}", marker.display());
        fs::write(
            &global,
            format!("[filter \"hostile\"]\nclean = {command}\n[core]\nfsmonitor = {command}\n"),
        )
        .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(fixture_args("fixture_poisoned_environment"))
            .current_dir(selected.path())
            .env("GIT_DIR", other.path().join(".git"))
            .env("GIT_WORK_TREE", other.path())
            .env("GIT_INDEX_FILE", other.path().join(".git/index"))
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_SYSTEM", &global)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
            .env("GIT_CONFIG_VALUE_0", &command)
            .env(
                "GIT_CONFIG_PARAMETERS",
                "'core.fsmonitor'='untrusted-command'",
            )
            .env("GIT_EXEC_PATH", other.path())
            .env("GIT_EXTERNAL_DIFF", &command)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!marker.exists());
    }
    fn fixture_args(name: &str) -> Vec<String> {
        vec![
            "--ignored".into(),
            "--exact".into(),
            format!("git_inspection::tests::{name}"),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ]
    }
    fn fixture_guard() {
        let directory = std::env::current_dir().unwrap();
        assert!(directory.to_string_lossy().starts_with("E:"));
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("diskburrow-git-")
        );
    }
    #[test]
    #[ignore = "child-only environment fixture"]
    fn fixture_poisoned_environment() {
        fixture_guard();
        let actual = check(&std::env::current_dir().unwrap());
        assert_eq!(actual.repository, RepositoryState::Repository);
        assert_eq!((actual.changed, actual.untracked), (Some(1), Some(1)));
        assert_eq!(actual.uncertainty, Some(GitUncertainty::NoUpstream));
    }
    #[test]
    #[ignore = "child-only native process fixture"]
    fn fixture_sleep() {
        fixture_guard();
        fs::write("child-pid", std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(30));
    }
    #[test]
    #[ignore = "child-only native process fixture"]
    fn fixture_tree() {
        fixture_guard();
        fs::write("parent-pid", std::process::id().to_string()).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(fixture_args("fixture_sleep"))
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_secs(30));
        let _ = child.kill();
        let _ = child.wait();
    }
    #[test]
    #[ignore = "child-only native process fixture"]
    fn fixture_spawn() {
        fixture_guard();
        match Command::new(std::env::current_exe().unwrap())
            .args(fixture_args("fixture_sleep"))
            .spawn()
        {
            Err(_) => println!("CHILD_BLOCKED"),
            Ok(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("CHILD_ESCAPED");
            }
        }
    }
    #[test]
    #[ignore = "child-only native process fixture"]
    fn fixture_output() {
        fixture_guard();
        use std::io::Write;
        let data = [b'x'; 8192];
        for _ in 0..200 {
            std::io::stdout().write_all(&data).unwrap();
            std::io::stderr().write_all(&data).unwrap();
        }
    }
    #[test]
    fn job_refuses_descendants_before_any_marker_can_be_written() {
        let temp = fixture();
        let arguments = fixture_args("fixture_spawn");
        let refs: Vec<_> = arguments.iter().map(String::as_str).collect();
        let output = process::run(
            &std::env::current_exe().unwrap(),
            temp.path(),
            &refs,
            &Cancellation::default(),
            Instant::now() + Duration::from_secs(5),
            1024 * 1024,
            1,
        )
        .unwrap();
        assert_eq!(
            output.code,
            0,
            "{}",
            String::from_utf8_lossy(&output.output)
        );
        assert!(String::from_utf8_lossy(&output.output).contains("CHILD_BLOCKED"));
        assert!(!temp.path().join("child-pid").exists());
    }
    #[test]
    fn timeout_terminates_the_native_job() {
        let temp = fixture();
        let arguments = fixture_args("fixture_sleep");
        let refs: Vec<_> = arguments.iter().map(String::as_str).collect();
        let started = Instant::now();
        let output = process::run(
            &std::env::current_exe().unwrap(),
            temp.path(),
            &refs,
            &Cancellation::default(),
            started + Duration::from_millis(500),
            1024 * 1024,
            1,
        );
        assert!(matches!(output, Err(GitUncertainty::TimedOut)));
        assert!(started.elapsed() < Duration::from_secs(2));
        if let Ok(pid) = fs::read_to_string(temp.path().join("child-pid")) {
            assert_process_gone(pid.trim().parse().unwrap());
        }
    }
    fn assert_process_gone(pid: u32) {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject},
        };
        unsafe {
            let handle = OpenProcess(SYNCHRONIZATION_SYNCHRONIZE, 0, pid);
            if !handle.is_null() {
                let state = WaitForSingleObject(handle, 2000);
                CloseHandle(handle);
                assert_eq!(state, WAIT_OBJECT_0, "process {pid} survived job close");
            }
        }
    }
    #[test]
    fn cancellation_closes_job_and_kills_the_entire_child_tree() {
        let temp = fixture();
        let cancel = Cancellation::default();
        let other = cancel.clone();
        let marker = temp.path().join("child-pid");
        let trigger = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !marker.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            other.cancel();
        });
        let arguments = fixture_args("fixture_tree");
        let refs: Vec<_> = arguments.iter().map(String::as_str).collect();
        // Same native runner with two processes allowed solely to prove descendant
        // lifetime. Every production Git call uses an active-process limit of one.
        let output = process::run(
            &std::env::current_exe().unwrap(),
            temp.path(),
            &refs,
            &cancel,
            Instant::now() + Duration::from_secs(8),
            1024 * 1024,
            2,
        );
        trigger.join().unwrap();
        assert!(matches!(output, Err(GitUncertainty::Cancelled)));
        for filename in ["parent-pid", "child-pid"] {
            let pid = fs::read_to_string(temp.path().join(filename)).unwrap();
            assert_process_gone(pid.trim().parse().unwrap());
        }
    }
    #[test]
    fn combined_stdout_and_stderr_overflow_is_bounded_and_terminated() {
        let temp = fixture();
        let arguments = fixture_args("fixture_output");
        let refs: Vec<_> = arguments.iter().map(String::as_str).collect();
        let output = process::run(
            &std::env::current_exe().unwrap(),
            temp.path(),
            &refs,
            &Cancellation::default(),
            Instant::now() + Duration::from_secs(5),
            64 * 1024,
            1,
        );
        assert!(matches!(output, Err(GitUncertainty::OutputLimit)));
    }
}
