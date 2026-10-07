use serde::{Deserialize, Serialize};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
    Storage::FileSystem::GetDriveTypeW,
    System::{
        Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize},
        Diagnostics::ToolHelp::*,
    },
    UI::Shell::*,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDirectories {
    pub user_profile: String,
    pub local_app_data: String,
    pub windows: String,
    pub program_files: String,
    pub program_files_x86: String,
    pub program_data: String,
    pub temp_hint: String,
    pub user_library_roots: Vec<String>,
    pub user_library_roots_verified: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerProcessState {
    Closed,
    Running,
    Unavailable,
}
pub trait RuleEnvironment: Send + Sync {
    fn known_directories(&self) -> KnownDirectories;
    fn check_owner_process(&self, name: &str) -> OwnerProcessState;
    fn is_local_path(&self, path: &str) -> bool {
        let Some(p) = crate::normalize_local_path(path) else {
            return false;
        };
        let root: Vec<u16> = p[..3].encode_utf16().chain(Some(0)).collect();
        matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 6)
    }
}
/// Deterministic process state for task-owned native fixtures and embedding tests.
pub struct FixedRuleEnvironment {
    pub known: KnownDirectories,
    pub owner_state: OwnerProcessState,
}
impl RuleEnvironment for FixedRuleEnvironment {
    fn known_directories(&self) -> KnownDirectories {
        self.known.clone()
    }
    fn check_owner_process(&self, _: &str) -> OwnerProcessState {
        self.owner_state
    }
}
#[derive(Default)]
pub struct WindowsRuleEnvironment;
impl RuleEnvironment for WindowsRuleEnvironment {
    fn known_directories(&self) -> KnownDirectories {
        // DONT_VERIFY resolves redirected paths without creation or existence probes.
        let libraries = [
            FOLDERID_Desktop,
            FOLDERID_Documents,
            FOLDERID_Pictures,
            FOLDERID_Music,
            FOLDERID_Videos,
            FOLDERID_Downloads,
        ]
        .iter()
        .map(folder)
        .collect::<Vec<_>>();
        let verified = libraries
            .iter()
            .all(|p| crate::normalize_local_path(p).is_some());
        KnownDirectories {
            user_profile: folder(&FOLDERID_Profile),
            local_app_data: folder(&FOLDERID_LocalAppData),
            windows: folder(&FOLDERID_Windows),
            program_files: folder(&FOLDERID_ProgramFiles),
            program_files_x86: folder(&FOLDERID_ProgramFilesX86),
            program_data: folder(&FOLDERID_ProgramData),
            temp_hint: std::env::temp_dir().to_string_lossy().into_owned(),
            user_library_roots: libraries,
            user_library_roots_verified: verified,
        }
    }
    fn check_owner_process(&self, name: &str) -> OwnerProcessState {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return OwnerProcessState::Unavailable;
        }
        struct Snapshot(windows_sys::Win32::Foundation::HANDLE);
        impl Drop for Snapshot {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let _guard = Snapshot(snapshot);
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) };
        let expected = format!("{name}.exe");
        while ok != 0 {
            let n = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..n]).eq_ignore_ascii_case(&expected) {
                return OwnerProcessState::Running;
            }
            ok = unsafe { Process32NextW(snapshot, &mut entry) };
        }
        if unsafe { GetLastError() } == ERROR_NO_MORE_FILES {
            OwnerProcessState::Closed
        } else {
            OwnerProcessState::Unavailable
        }
    }
}
fn folder(id: &windows_sys::core::GUID) -> String {
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), 0) };
    if initialized < 0 && initialized != 0x80010106u32 as i32 {
        return String::new();
    }
    let mut buffer = std::ptr::null_mut();
    let result = unsafe { SHGetKnownFolderPath(id, 0x4000, std::ptr::null_mut(), &mut buffer) };
    let path = if result >= 0 && !buffer.is_null() {
        unsafe {
            let mut n = 0;
            while *buffer.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(buffer, n))
        }
    } else {
        String::new()
    };
    if !buffer.is_null() {
        unsafe {
            CoTaskMemFree(buffer.cast());
        }
    }
    if initialized >= 0 {
        unsafe {
            CoUninitialize();
        }
    }
    path
}
