use chrono::{DateTime, Utc};
use diskburrow_services::{FileIdentity, FileObservation};
use std::{
    ffi::c_void,
    io,
    mem::{size_of, zeroed},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::*,
};

pub const DIRECTORY: u32 = 0x10;
pub const REPARSE: u32 = 0x400;
pub const READONLY: u32 = 1;
pub const SYSTEM: u32 = 4;
pub fn is_cloud(attributes: u32) -> bool {
    attributes & (0x1000 | 0x40000 | 0x400000) != 0
}
const FLAGS: u32 =
    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL;
pub struct NativeHandle(HANDLE);
// Handles have no thread affinity. Each call reads metadata or uses a uniquely leased target.
unsafe impl Send for NativeHandle {}
unsafe impl Sync for NativeHandle {}
impl NativeHandle {
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}
impl Drop for NativeHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn open(path: &str, access: u32, share: u32) -> io::Result<NativeHandle> {
    let normalized = crate::normalize_local_path(path)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unsafe local path"))?;
    let p = wide(&format!(r"\\?\{}", normalized));
    let h = unsafe {
        CreateFileW(
            p.as_ptr(),
            access,
            share,
            std::ptr::null(),
            OPEN_EXISTING,
            FLAGS,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(NativeHandle(h))
    }
}
fn information<T>(h: &NativeHandle, kind: FILE_INFO_BY_HANDLE_CLASS) -> io::Result<T> {
    let mut value: T = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            h.0,
            kind,
            (&mut value as *mut T).cast::<c_void>(),
            size_of::<T>() as u32,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}
pub trait NativeFileApi: Send + Sync {
    fn open_metadata(&self, path: &str) -> io::Result<NativeHandle> {
        open(
            path,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
    }
    // LIST_DIRECTORY ensures the no-write/no-delete sharing restrictions protect reparse mutation.
    fn open_directory(&self, path: &str) -> io::Result<NativeHandle> {
        open(
            path,
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
            FILE_SHARE_READ,
        )
    }
    fn open_target(&self, path: &str) -> io::Result<NativeHandle> {
        open(
            path,
            DELETE | FILE_READ_ATTRIBUTES | FILE_READ_DATA,
            FILE_SHARE_READ,
        )
    }
    fn inspect(&self, path: &str) -> io::Result<FileObservation> {
        let normalized = crate::normalize_local_path(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Unsafe local path"))?;
        let p = wide(&format!(r"\\?\{}", normalized));
        let attributes = unsafe { GetFileAttributesW(p.as_ptr()) };
        if attributes == INVALID_FILE_ATTRIBUTES {
            return Err(io::Error::last_os_error());
        }
        if attributes & REPARSE != 0 || is_cloud(attributes) {
            return Ok(unsafe_observation(path, attributes));
        }
        self.inspect_handle(&self.open_metadata(path)?, path)
    }
    fn inspect_handle(&self, h: &NativeHandle, path: &str) -> io::Result<FileObservation> {
        let basic: FILE_BASIC_INFO = information(h, FileBasicInfo)?;
        let attributes = basic.FileAttributes;
        if attributes & REPARSE != 0 || is_cloud(attributes) {
            return Ok(unsafe_observation(path, attributes));
        }
        let standard: FILE_STANDARD_INFO = information(h, FileStandardInfo)?;
        let identity = information::<FILE_ID_INFO>(h, FileIdInfo).ok().map(|id| {
            let low = u64::from_le_bytes(id.FileId.Identifier[..8].try_into().unwrap());
            let high = u64::from_le_bytes(id.FileId.Identifier[8..].try_into().unwrap());
            FileIdentity {
                volume: id.VolumeSerialNumber,
                file_id: format!("{low:016X}{high:016X}"),
            }
        });
        let mut allocated = (standard.AllocationSize >= 0).then_some(standard.AllocationSize);
        if attributes & (FILE_ATTRIBUTE_SPARSE_FILE | FILE_ATTRIBUTE_COMPRESSED) != 0 {
            allocated = information::<FILE_COMPRESSION_INFO>(h, FileCompressionInfo)
                .ok()
                .and_then(|c| (c.CompressedFileSize >= 0).then_some(c.CompressedFileSize));
        }
        if identity.is_none() {
            allocated = None;
        }
        let ticks = basic
            .LastWriteTime
            .checked_sub(116444736000000000)
            .ok_or_else(|| io::Error::other("Timestamp out of range"))?;
        let modified_utc = DateTime::from_timestamp(
            ticks.div_euclid(10_000_000),
            (ticks.rem_euclid(10_000_000) * 100) as u32,
        )
        .ok_or_else(|| io::Error::other("Timestamp out of range"))?;
        Ok(FileObservation {
            path: path.to_owned(),
            identity,
            logical_bytes: standard.EndOfFile,
            allocated_bytes: allocated,
            modified_utc,
            link_count: i32::try_from(standard.NumberOfLinks)
                .map_err(|_| io::Error::other("Link count overflow"))?,
            attributes,
        })
    }
    fn final_path(&self, h: &NativeHandle) -> io::Result<String> {
        let mut buffer = vec![0u16; 32768];
        let n =
            unsafe { GetFinalPathNameByHandleW(h.0, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n as usize >= buffer.len() {
            return Err(io::Error::other("Final path too long"));
        }
        let path = String::from_utf16(&buffer[..n as usize])
            .map_err(|_| io::Error::other("Invalid final path"))?;
        Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned())
    }
    fn mark_for_deletion(&self, h: &NativeHandle) -> io::Result<()> {
        let information = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                h.0,
                FileDispositionInfo,
                (&information as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
#[derive(Default)]
pub struct WindowsNativeFileApi;
impl NativeFileApi for WindowsNativeFileApi {}
pub fn inspect(path: &str) -> io::Result<FileObservation> {
    WindowsNativeFileApi.inspect(path)
}
fn unsafe_observation(path: &str, attributes: u32) -> FileObservation {
    FileObservation {
        path: path.into(),
        identity: None,
        logical_bytes: 0,
        allocated_bytes: None,
        modified_utc: DateTime::<Utc>::MIN_UTC,
        link_count: 0,
        attributes,
    }
}

pub(crate) struct AncestorLease<'a> {
    api: &'a dyn NativeFileApi,
    pinned: Vec<(String, NativeHandle, FileIdentity)>,
}
impl<'a> AncestorLease<'a> {
    pub fn open(api: &'a dyn NativeFileApi, path: &str) -> io::Result<Self> {
        Self::open_checked(api, path, &mut || Ok(()))
    }
    /// Check between native calls; a checkpoint's typed policy error is preserved.
    pub fn open_checked<E: From<io::Error>>(
        api: &'a dyn NativeFileApi,
        path: &str,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        let parent =
            crate::paths::parent(path).ok_or_else(|| io::Error::other("Unsafe ancestor"))?;
        let mut lease = Self {
            api,
            pinned: Vec::new(),
        };
        for p in crate::paths::chain(&parent) {
            checkpoint()?;
            let h = api.open_directory(&p)?;
            checkpoint()?;
            let o = api.inspect_handle(&h, &p)?;
            validate_directory_checked(api, &p, &h, &o, checkpoint)?;
            let identity = o
                .identity
                .ok_or_else(|| io::Error::other("Identity unavailable"))?;
            if lease
                .pinned
                .first()
                .is_some_and(|(_, _, id)| id.volume != identity.volume)
            {
                return Err(io::Error::other("Volume changed").into());
            }
            lease.pinned.push((p, h, identity));
        }
        Ok(lease)
    }
    pub fn identities(&self) -> impl Iterator<Item = (&str, &FileIdentity)> {
        self.pinned.iter().map(|(p, _, id)| (p.as_str(), id))
    }
    pub fn volume(&self) -> u64 {
        self.pinned[0].2.volume
    }
    pub fn verify(&self) -> io::Result<()> {
        self.verify_checked(&mut || Ok(()))
    }
    pub fn verify_checked<E: From<io::Error>>(
        &self,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        for (p, h, id) in &self.pinned {
            checkpoint()?;
            let observed = self.api.inspect_handle(h, p)?;
            validate_directory_checked(self.api, p, h, &observed, checkpoint)?;
            if observed.identity.as_ref() != Some(id) {
                return Err(io::Error::other("Ancestor changed").into());
            }
        }
        Ok(())
    }
}
pub(crate) fn validate_directory(
    api: &dyn NativeFileApi,
    path: &str,
    h: &NativeHandle,
    o: &FileObservation,
) -> io::Result<()> {
    validate_directory_checked(api, path, h, o, &mut || Ok(()))
}
fn validate_directory_checked<E: From<io::Error>>(
    api: &dyn NativeFileApi,
    path: &str,
    h: &NativeHandle,
    o: &FileObservation,
    checkpoint: &mut impl FnMut() -> Result<(), E>,
) -> Result<(), E> {
    if o.identity.is_none()
        || o.attributes & DIRECTORY == 0
        || o.attributes & REPARSE != 0
        || is_cloud(o.attributes)
    {
        return Err(io::Error::other("Unsafe ancestor").into());
    }
    checkpoint()?;
    if crate::paths::canonical(&api.final_path(h)?).is_none_or(|p| !crate::equals_path(path, &p)) {
        Err(io::Error::other("Unsafe ancestor").into())
    } else {
        Ok(())
    }
}
pub(crate) fn safe_directory(api: &dyn NativeFileApi, path: &str) -> bool {
    let Some(path) = crate::normalize_local_path(path) else {
        return false;
    };
    // The chain is pinned before probing the root and therefore never follows a changed parent.
    if path.len() == 3 {
        return api
            .open_directory(&path)
            .and_then(|h| {
                let o = api.inspect_handle(&h, &path)?;
                validate_directory(api, &path, &h, &o)
            })
            .is_ok();
    }
    let Ok(lease) = AncestorLease::open(api, &path) else {
        return false;
    };
    api.open_directory(&path)
        .and_then(|h| {
            let o = api.inspect_handle(&h, &path)?;
            validate_directory(api, &path, &h, &o)?;
            lease.verify()
        })
        .is_ok()
}
