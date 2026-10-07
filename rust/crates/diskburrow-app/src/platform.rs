//! Windows environment and explicit integration. Startup inspection never writes settings/registry.
#[path = "tray.rs"]
mod tray;
use diskburrow_services::VolumeSpace;
use diskburrow_windows::{
    NativeFileApi, RuleEnvironment, WindowsNativeFileApi, WindowsRuleEnvironment, equals_path,
    is_cloud, normalize_local_path,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, mpsc::Sender},
};
pub use tray::PlatformEvent;
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{Com::CoTaskMemFree, Power::*, Registry::*, Threading::*},
    UI::Shell::*,
};

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub(crate) struct OwnedHandle(pub HANDLE);
// No mutex ownership is acquired. The handle lifetime and explicit events are thread independent.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl OwnedHandle {
    pub(crate) fn event(name: Option<&str>) -> io::Result<Self> {
        let name = name.map(wide);
        let h = unsafe {
            CreateEventW(
                std::ptr::null(),
                0,
                0,
                name.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            )
        };
        if h.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(h))
        }
    }
}

pub fn app_data_directory() -> anyhow::Result<PathBuf> {
    let mut ptr = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            0x4000,
            std::ptr::null_mut(),
            &mut ptr,
        )
    };
    if result < 0 || ptr.is_null() {
        anyhow::bail!("Local application data is unavailable");
    }
    let value = unsafe {
        let mut n = 0;
        while *ptr.add(n) != 0 {
            n += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(ptr, n))
    };
    unsafe {
        CoTaskMemFree(ptr.cast());
    }
    let value = value?;
    if normalize_local_path(&value).is_none() {
        anyhow::bail!("Local application data must be a local drive path");
    }
    Ok(PathBuf::from(value).join("DiskBurrow"))
}
pub fn system_root() -> String {
    let known = WindowsRuleEnvironment.known_directories();
    normalize_local_path(&known.windows)
        .map(|p| p[..3].into())
        .unwrap_or_default()
}
pub fn on_battery() -> bool {
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    unsafe { GetSystemPowerStatus(&mut status) == 0 || status.ACLineStatus != 1 }
}
pub fn is_local_path(path: &str) -> bool {
    local_directory_handles(path).is_ok()
}
fn local_directory_handles(
    path: &str,
) -> io::Result<(String, Vec<diskburrow_windows::NativeHandle>)> {
    let path = normalize_local_path(path).ok_or_else(|| io::Error::other("Unsafe local path"))?;
    if !WindowsRuleEnvironment.is_local_path(&path) {
        return Err(io::Error::other("Non-local volume"));
    }
    // Pin each directory before resolving its child; cloud/reparse points are never followed.
    let mut chain = Vec::new();
    let mut p = PathBuf::from(&path);
    loop {
        chain.push(p.clone());
        if !p.pop() {
            break;
        }
    }
    chain.reverse();
    let api = WindowsNativeFileApi;
    let mut handles = Vec::new();
    let mut volume = None;
    for p in chain {
        let p = p.to_string_lossy();
        let h = api.open_directory(&p)?;
        let o = api.inspect_handle(&h, &p)?;
        let final_path = api.final_path(&h)?;
        let id = o
            .identity
            .ok_or_else(|| io::Error::other("Directory identity unavailable"))?;
        if o.attributes & (FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || o.attributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || is_cloud(o.attributes)
            || !equals_path(&p, &final_path)
            || volume.is_some_and(|v| v != id.volume)
        {
            return Err(io::Error::other("Unsafe local directory"));
        }
        volume = Some(id.volume);
        handles.push(h);
    }
    Ok((path, handles))
}
pub fn volume_space(path: &str) -> anyhow::Result<VolumeSpace> {
    let (path, _pinned) = local_directory_handles(path)?;
    let p = wide(&path);
    let mut available = 0u64;
    let mut total = 0u64;
    let mut free = 0u64;
    if unsafe { GetDiskFreeSpaceExW(p.as_ptr(), &mut available, &mut total, &mut free) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(VolumeSpace {
        total_bytes: i64::try_from(total)?,
        free_bytes: i64::try_from(available.min(total))?,
    })
}
pub fn list_drives() -> Vec<(String, String)> {
    let bits = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for drive in b'A'..=b'Z' {
        if bits & (1u32 << (drive - b'A')) == 0 {
            continue;
        }
        let root = format!("{}:\\", drive as char);
        if !WindowsRuleEnvironment.is_local_path(&root) {
            continue;
        }
        let p = wide(&root);
        let mut label = vec![0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                p.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        let name = if ok == 0 {
            root.clone()
        } else {
            let n = label.iter().position(|&c| c == 0).unwrap_or(label.len());
            let name = String::from_utf16_lossy(&label[..n]);
            if name.is_empty() {
                root.clone()
            } else {
                format!("{name} ({root})")
            }
        };
        drives.push((root, name));
    }
    drives
}

pub trait RunRegistry {
    fn read(&self) -> io::Result<Option<String>>;
    fn write(&self, value: &str) -> io::Result<()>;
    fn remove(&self) -> io::Result<()>;
}
pub struct AutostartRegistration {
    payload: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartChange {
    Updated,
    Removed,
    Unchanged,
    PreservedForeign,
}
impl AutostartRegistration {
    pub fn new(executable: &Path) -> anyhow::Result<Self> {
        let p = executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Executable path is not Unicode"))?;
        let normalized = normalize_local_path(p)
            .ok_or_else(|| anyhow::anyhow!("An absolute local executable path is required"))?;
        if p.contains(['\r', '\n', '\0', '"']) {
            anyhow::bail!("Invalid executable path");
        }
        Ok(Self {
            payload: format!("\"{normalized}\" --background"),
        })
    }
    #[cfg(test)]
    pub fn payload(&self) -> &str {
        &self.payload
    }
    pub fn is_path_changed(&self, registry: &dyn RunRegistry) -> io::Result<bool> {
        Ok(registry
            .read()?
            .is_some_and(|p| !p.eq_ignore_ascii_case(&self.payload)))
    }
    /// Called only by explicit Save Settings. A foreign same-name value survives disabling.
    pub fn apply(&self, registry: &dyn RunRegistry, enabled: bool) -> io::Result<AutostartChange> {
        let existing = registry.read()?;
        if enabled {
            if existing
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&self.payload))
            {
                return Ok(AutostartChange::Unchanged);
            }
            registry.write(&self.payload)?;
            return Ok(AutostartChange::Updated);
        }
        match existing {
            None => Ok(AutostartChange::Unchanged),
            Some(p) if p.eq_ignore_ascii_case(&self.payload) => {
                registry.remove()?;
                Ok(AutostartChange::Removed)
            }
            Some(_) => Ok(AutostartChange::PreservedForeign),
        }
    }

    /// Compensate our registry change if the local settings commit fails.
    /// A concurrent third-party registry edit is never overwritten by compensation.
    pub fn commit_with(
        &self,
        registry: &dyn RunRegistry,
        enabled: bool,
        commit: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<AutostartChange> {
        let previous = registry.read()?;
        let change = self.apply(registry, enabled)?;
        if let Err(error) = commit() {
            if matches!(change, AutostartChange::Updated | AutostartChange::Removed) {
                let expected = if change == AutostartChange::Updated {
                    Some(self.payload.clone())
                } else {
                    None
                };
                if registry.read()? != expected {
                    return Err(
                        error.context("Autostart changed concurrently; rollback was refused")
                    );
                }
                let rollback = match previous {
                    Some(value) => registry.write(&value),
                    None => registry.remove(),
                };
                if let Err(rollback) = rollback {
                    return Err(error.context(format!("Autostart rollback failed: {rollback}")));
                }
            }
            return Err(error);
        }
        Ok(change)
    }
}
struct RegistryKey(HKEY);
impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
pub struct WindowsRunRegistry;
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "DiskBurrow";
impl RunRegistry for WindowsRunRegistry {
    fn read(&self) -> io::Result<Option<String>> {
        let mut h = std::ptr::null_mut();
        let key = wide(RUN_KEY);
        let code =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_QUERY_VALUE, &mut h) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let guard = RegistryKey(h);
        let name = wide(VALUE);
        let mut kind = 0;
        let mut size = 0;
        let code = unsafe {
            RegQueryValueExW(
                guard.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        if kind != REG_SZ || size > 65536 || size % 2 != 0 {
            return Err(io::Error::other("Unsupported autostart value"));
        }
        let mut data = vec![0u16; (size / 2) as usize];
        let code = unsafe {
            RegQueryValueExW(
                guard.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        if kind != REG_SZ || size % 2 != 0 {
            return Err(io::Error::other("Unsupported autostart value"));
        }
        data.truncate((size / 2) as usize);
        while data.last() == Some(&0) {
            data.pop();
        }
        let value =
            String::from_utf16(&data).map_err(|_| io::Error::other("Invalid autostart string"))?;
        Ok(Some(value))
    }
    fn write(&self, value: &str) -> io::Result<()> {
        let key = wide(RUN_KEY);
        let mut h = std::ptr::null_mut();
        let code = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut h,
                std::ptr::null_mut(),
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let guard = RegistryKey(h);
        let data = wide(value);
        let name = wide(VALUE);
        let code = unsafe {
            RegSetValueExW(
                guard.0,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }
    fn remove(&self) -> io::Result<()> {
        let key = wide(RUN_KEY);
        let mut h = std::ptr::null_mut();
        let code =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_SET_VALUE, &mut h) };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let guard = RegistryKey(h);
        let name = wide(VALUE);
        let code = unsafe { RegDeleteValueW(guard.0, name.as_ptr()) };
        if code == 0 || code == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }
}
pub struct PlatformBridge {
    instance: OwnedHandle,
    activation: Arc<OwnedHandle>,
    tray: Option<tray::TrayThread>,
    primary: bool,
}
impl PlatformBridge {
    pub fn start(sender: Sender<PlatformEvent>) -> anyhow::Result<Self> {
        let sid = user_sid()?;
        let name = format!("Global\\DiskBurrow-Rust-{sid}");
        let activation = Arc::new(OwnedHandle::event(Some(&format!("{name}.Activate")))?);
        let mutex_name = wide(&name);
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if h.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let primary = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
        let instance = OwnedHandle(h);
        let tray = if primary {
            Some(tray::TrayThread::start(sender, activation.clone())?)
        } else {
            if unsafe { SetEvent(activation.0) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            None
        };
        Ok(Self {
            instance,
            activation,
            tray,
            primary,
        })
    }
    pub fn is_primary(&self) -> bool {
        self.primary
    }
    pub fn update(&self, language: &str, paused: bool) {
        if let Some(t) = &self.tray {
            t.update(language, paused);
        }
    }
    pub fn set_busy(&self, busy: bool) {
        if let Some(t) = &self.tray {
            t.set_busy(busy);
        }
    }
    pub fn notify(&self, title: &str, message: &str, path: Option<String>) {
        if let Some(t) = &self.tray {
            t.notify(title, message, path);
        }
    }
}
impl Drop for PlatformBridge {
    fn drop(&mut self) {
        self.tray.take();
        let _ = &self.instance;
        let _ = &self.activation;
    }
}
pub(crate) fn user_sid() -> io::Result<String> {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let guard = OwnedHandle(token);
    let mut needed = 0;
    unsafe {
        GetTokenInformation(guard.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut data = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            guard.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*data.as_ptr().cast::<TOKEN_USER>() };
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let sid = unsafe {
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, n))
    };
    unsafe {
        LocalFree(text.cast());
    }
    Ok(sid)
}

/// Reveal a file or folder in Explorer; never execute the selected file itself.
pub fn open_path(path: &str) {
    if normalize_local_path(path).is_none() {
        return;
    }
    let verb = wide("open");
    let exe = wide("explorer.exe");
    let args = wide(&format!("/select,\"{}\"", path));
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            args.as_ptr(),
            std::ptr::null(),
            1,
        );
    }
}

pub fn open_maintenance(index: usize) {
    let Some(target) = maintenance_target(index) else {
        return;
    };
    let target = wide(target);
    let verb = wide("open");
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        );
    }
}

pub fn maintenance_target(index: usize) -> Option<&'static str> {
    ["ms-settings:storagesense",
     "https://support.google.com/chrome/answer/2392709?co=GENIE.Platform%3DDesktop",
     "https://support.microsoft.com/en-us/edge/view-and-delete-browser-history-in-microsoft-edge",
     "https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders",
     "https://pip.pypa.io/en/stable/topics/caching/"].get(index).copied()
}

pub fn startup_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let message = wide(message);
    let title = wide("DiskBurrow");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// The handle comes from the application's own GPUI window, never a desktop search.
pub fn restore_own_window(handle: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SW_SHOW, ShowWindow,
    };
    let hwnd = handle as HWND;
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut process);
        if process != GetCurrentProcessId() {
            return;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
    }
}

pub fn hide_own_window(handle: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, SW_HIDE, ShowWindow,
    };
    let hwnd = handle as HWND;
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut process);
        if process == GetCurrentProcessId() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct MemoryRegistry {
        value: RefCell<Option<String>>,
        writes: RefCell<usize>,
    }
    impl RunRegistry for MemoryRegistry {
        fn read(&self) -> io::Result<Option<String>> {
            Ok(self.value.borrow().clone())
        }
        fn write(&self, value: &str) -> io::Result<()> {
            *self.value.borrow_mut() = Some(value.into());
            *self.writes.borrow_mut() += 1;
            Ok(())
        }
        fn remove(&self) -> io::Result<()> {
            self.value.borrow_mut().take();
            *self.writes.borrow_mut() += 1;
            Ok(())
        }
    }
    #[test]
    fn autostart_construct_and_inspect_do_not_write_registry() {
        let registry = MemoryRegistry::default();
        let policy =
            AutostartRegistration::new(Path::new(r"E:\Program Files\DiskBurrow\DiskBurrow.exe"))
                .unwrap();
        assert_eq!(
            policy.payload(),
            r#""E:\Program Files\DiskBurrow\DiskBurrow.exe" --background"#
        );
        assert!(!policy.is_path_changed(&registry).unwrap());
        assert_eq!(0, *registry.writes.borrow());
    }

    #[test]
    fn failed_settings_commit_restores_original_autostart_value() {
        let registry = MemoryRegistry::default();
        let original = r#""E:\Other\DiskBurrow.exe""#.to_owned();
        *registry.value.borrow_mut() = Some(original.clone());
        let policy =
            AutostartRegistration::new(Path::new(r"E:\DiskBurrow\DiskBurrow.exe")).unwrap();
        assert!(
            policy
                .commit_with(&registry, true, || anyhow::bail!("Fixture disk full"))
                .is_err()
        );
        assert_eq!(*registry.value.borrow(), Some(original));
        assert_eq!(*registry.writes.borrow(), 2);
    }
    #[test]
    fn settings_compensation_never_overwrites_a_concurrent_registry_edit() {
        let registry = MemoryRegistry::default();
        let policy =
            AutostartRegistration::new(Path::new(r"E:\DiskBurrow\DiskBurrow.exe")).unwrap();
        assert!(
            policy
                .commit_with(&registry, true, || {
                    registry.write("third-party concurrent value")?;
                    anyhow::bail!("Fixture disk full")
                })
                .is_err()
        );
        assert_eq!(
            registry.value.borrow().as_deref(),
            Some("third-party concurrent value")
        );
    }

    #[test]
    fn restore_reopens_only_an_owned_native_fixture_window() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, IsWindowVisible, WS_POPUP,
        };
        let class = wide("STATIC");
        let title = wide("DiskBurrow native lifecycle fixture");
        let handle = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                -32000,
                -32000,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!handle.is_null());
        assert_eq!(unsafe { IsWindowVisible(handle) }, 0);
        restore_own_window(handle as isize);
        assert_ne!(unsafe { IsWindowVisible(handle) }, 0);
        hide_own_window(handle as isize);
        assert_eq!(unsafe { IsWindowVisible(handle) }, 0);
        restore_own_window(handle as isize);
        assert_ne!(unsafe { IsWindowVisible(handle) }, 0);
        unsafe {
            DestroyWindow(handle);
        }
        restore_own_window(0);
    }
    #[test]
    fn explicit_enable_disable_and_foreign_entry_preservation() {
        let registry = MemoryRegistry::default();
        let policy =
            AutostartRegistration::new(Path::new(r"E:\DiskBurrow\DiskBurrow.exe")).unwrap();
        assert_eq!(
            AutostartChange::Updated,
            policy.apply(&registry, true).unwrap()
        );
        assert_eq!(
            AutostartChange::Unchanged,
            policy.apply(&registry, true).unwrap()
        );
        assert_eq!(
            AutostartChange::Removed,
            policy.apply(&registry, false).unwrap()
        );
        *registry.value.borrow_mut() = Some(r#""E:\Other\DiskBurrow.exe""#.into());
        assert!(policy.is_path_changed(&registry).unwrap());
        assert_eq!(
            AutostartChange::PreservedForeign,
            policy.apply(&registry, false).unwrap()
        );
        assert_eq!(2, *registry.writes.borrow());
        assert!(registry.value.borrow().as_ref().unwrap().contains("Other"));
    }
    #[test]
    fn autostart_rejects_command_injection_relative_unc_and_stream_paths() {
        for path in [
            "DiskBurrow.exe",
            r"\\server\share\DiskBurrow.exe",
            r#"E:\bad" --evil.exe"#,
            r"E:\DiskBurrow.exe:stream",
            "E:\\line\n.exe",
        ] {
            assert!(
                AutostartRegistration::new(Path::new(path)).is_err(),
                "{path}"
            );
        }
    }
    #[test]
    fn tray_fixed_buffer_truncates_at_unicode_scalar_and_zero_terminates() {
        let mut a = [5u16; 5];
        tray::fill_utf16(&mut a, "A\0😀XYZ");
        assert_eq!(String::from_utf16(&a[..4]).unwrap(), "A😀X");
        assert_eq!(0, a[4]);
        let mut tiny = [7u16; 2];
        tray::fill_utf16(&mut tiny, "😀");
        assert_eq!([0, 0], tiny);
    }
    #[test]
    fn native_local_volume_and_battery_are_readonly() {
        assert!(!is_local_path(r"\\server\share"));
        assert!(!is_local_path(r"E:\NUL"));
        let space = volume_space("E:\\").unwrap();
        assert!(space.total_bytes > 0);
        assert!(space.free_bytes >= 0 && space.free_bytes <= space.total_bytes);
        let _ = on_battery();
    }
}
