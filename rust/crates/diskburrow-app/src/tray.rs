//! All HWND, menu and notification operations live on this owned message thread.
use super::{OwnedHandle, wide};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{INFINITE, SetEvent},
    },
    UI::{Shell::*, WindowsAndMessaging::*},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformEvent {
    Show,
    Scan,
    Pause,
    Exit,
    Open(String),
}
enum TrayCommand {
    Update(String, bool),
    Busy(bool),
    Notify(String, String, Option<String>),
}
pub struct TrayThread {
    stop: Arc<OwnedHandle>,
    wake: Arc<OwnedHandle>,
    window: Arc<AtomicUsize>,
    commands: Sender<TrayCommand>,
    thread: Option<JoinHandle<()>>,
}
impl TrayThread {
    pub fn start(events: Sender<PlatformEvent>, activation: Arc<OwnedHandle>) -> io::Result<Self> {
        let stop = Arc::new(OwnedHandle::event(None)?);
        let wake = Arc::new(OwnedHandle::event(None)?);
        let window = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_stop = stop.clone();
        let thread_wake = wake.clone();
        let thread_window = window.clone();
        let worker = thread::Builder::new()
            .name("DiskBurrow tray".into())
            .spawn(move || {
                run(
                    events,
                    activation,
                    thread_stop,
                    thread_wake,
                    thread_window,
                    rx,
                    ready_tx,
                )
            })?;
        let mut tray = Self {
            stop,
            wake,
            window,
            commands: tx,
            thread: Some(worker),
        };
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(tray),
            Ok(Err(e)) => {
                tray.stop_and_join();
                Err(e)
            }
            Err(_) => {
                tray.stop_and_join();
                Err(io::Error::other("Tray startup did not complete"))
            }
        }
    }
    fn command(&self, command: TrayCommand) {
        if self.commands.send(command).is_ok() {
            unsafe {
                SetEvent(self.wake.0);
            }
        }
    }
    pub fn update(&self, language: &str, paused: bool) {
        self.command(TrayCommand::Update(language.into(), paused));
    }
    pub fn set_busy(&self, busy: bool) {
        self.command(TrayCommand::Busy(busy));
    }
    pub fn notify(&self, title: &str, message: &str, path: Option<String>) {
        self.command(TrayCommand::Notify(title.into(), message.into(), path));
    }
    fn stop_and_join(&mut self) {
        unsafe {
            SetEvent(self.stop.0);
        }
        let hwnd = self.window.load(Ordering::Acquire) as HWND;
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
                PostMessageW(hwnd, WM_NULL, 0, 0);
            }
        }
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for TrayThread {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

const CALLBACK: u32 = WM_APP + 17;
const OPEN_ID: usize = 1;
const SCAN_ID: usize = 2;
const PAUSE_ID: usize = 3;
const EXIT_ID: usize = 4;
struct Context {
    events: Sender<PlatformEvent>,
    icon: NOTIFYICONDATAW,
    icon_owned: bool,
    taskbar_created: u32,
    paused: bool,
    busy: bool,
    language: String,
    pending_path: Option<String>,
}
fn run(
    events: Sender<PlatformEvent>,
    activation: Arc<OwnedHandle>,
    stop: Arc<OwnedHandle>,
    wake: Arc<OwnedHandle>,
    window: Arc<AtomicUsize>,
    commands: Receiver<TrayCommand>,
    ready: mpsc::SyncSender<io::Result<()>>,
) {
    let class = wide("DiskBurrowRustTrayWindow");
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
    wc.lpfnWndProc = Some(window_proc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    if unsafe { RegisterClassW(&wc) } == 0 {
        let _ = ready.send(Err(io::Error::last_os_error()));
        return;
    }
    let mut context = Box::new(Context {
        events,
        icon: unsafe { std::mem::zeroed() },
        icon_owned: false,
        taskbar_created: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        paused: false,
        busy: false,
        language: "en".into(),
        pending_path: None,
    });
    // An invisible top-level window receives Explorer's TaskbarCreated broadcast after restart.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            wide("DiskBurrow tray").as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            (&mut *context as *mut Context).cast(),
        )
    };
    if hwnd.is_null() {
        let _ = ready.send(Err(io::Error::last_os_error()));
        unsafe {
            UnregisterClassW(class.as_ptr(), instance);
        }
        return;
    }
    window.store(hwnd as usize, Ordering::Release);
    context.icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    context.icon.hWnd = hwnd;
    context.icon.uID = 1;
    context.icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    context.icon.uCallbackMessage = CALLBACK;
    let embedded = include_bytes!("../../../resources/DiskBurrow.ico");
    let hicon = best_icon_image(embedded)
        .map(|image| unsafe {
            CreateIconFromResourceEx(
                image.as_ptr(),
                image.len() as u32,
                1,
                0x00030000,
                32,
                32,
                LR_DEFAULTCOLOR,
            )
        })
        .unwrap_or(std::ptr::null_mut());
    context.icon_owned = !hicon.is_null();
    context.icon.hIcon = if context.icon_owned {
        hicon
    } else {
        unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) }
    };
    fill_utf16(&mut context.icon.szTip, "DiskBurrow — disk monitoring");
    if unsafe { Shell_NotifyIconW(NIM_ADD, &context.icon) } == 0 {
        let _ = ready.send(Err(io::Error::other(
            "Windows notification area is unavailable",
        )));
        window.store(0, Ordering::Release);
        unsafe {
            DestroyWindow(hwnd);
            if context.icon_owned {
                DestroyIcon(context.icon.hIcon);
            }
            UnregisterClassW(class.as_ptr(), instance);
        }
        return;
    }
    let _ = ready.send(Ok(()));
    let handles = [stop.0, activation.0, wake.0];
    'messages: loop {
        let result = unsafe {
            MsgWaitForMultipleObjects(
                handles.len() as u32,
                handles.as_ptr(),
                0,
                INFINITE,
                QS_ALLINPUT,
            )
        };
        if result == WAIT_OBJECT_0 || result == WAIT_FAILED {
            break;
        }
        if result == WAIT_OBJECT_0 + 1 {
            let _ = context.events.send(PlatformEvent::Show);
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                TrayCommand::Update(language, paused) => {
                    context.language = if language == "ru" {
                        "ru".into()
                    } else {
                        "en".into()
                    };
                    context.paused = paused;
                    fill_utf16(
                        &mut context.icon.szTip,
                        if context.language == "ru" {
                            "DiskBurrow — наблюдение за диском"
                        } else {
                            "DiskBurrow — disk monitoring"
                        },
                    );
                    context.icon.uFlags = NIF_TIP;
                    unsafe {
                        Shell_NotifyIconW(NIM_MODIFY, &context.icon);
                    }
                }
                TrayCommand::Busy(busy) => context.busy = busy,
                TrayCommand::Notify(title, message, path) => {
                    context.pending_path = path;
                    fill_utf16(&mut context.icon.szInfoTitle, &title);
                    fill_utf16(&mut context.icon.szInfo, &message);
                    context.icon.uFlags = NIF_INFO;
                    context.icon.dwInfoFlags = NIIF_WARNING;
                    unsafe {
                        Shell_NotifyIconW(NIM_MODIFY, &context.icon);
                    }
                }
            }
        }
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                break 'messages;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    // Remove callbacks while Context is still alive; then destroy the thread's window and class.
    window.store(0, Ordering::Release);
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &context.icon);
        DestroyWindow(hwnd);
        if context.icon_owned {
            DestroyIcon(context.icon.hIcon);
        }
        UnregisterClassW(class.as_ptr(), instance);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        return 1;
    }
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Context;
    if ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    if message == CALLBACK && matches!(lparam as u32, WM_RBUTTONUP | WM_CONTEXTMENU) {
        popup(hwnd, ptr);
        return 0;
    }
    let context = unsafe { &mut *ptr };
    if message == WM_QUERYENDSESSION || (message == WM_ENDSESSION && wparam != 0) {
        let _ = context.events.send(PlatformEvent::Exit);
        return if message == WM_QUERYENDSESSION { 1 } else { 0 };
    }
    if message == context.taskbar_created && context.taskbar_created != 0 {
        context.icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &context.icon);
        }
        return 0;
    }
    if message == CALLBACK {
        match lparam as u32 {
            WM_LBUTTONDBLCLK => {
                context.pending_path = None;
                let _ = context.events.send(PlatformEvent::Show);
            }
            NIN_BALLOONUSERCLICK => {
                let event = context
                    .pending_path
                    .take()
                    .map(PlatformEvent::Open)
                    .unwrap_or(PlatformEvent::Show);
                let _ = context.events.send(event);
            }
            _ => {}
        }
        return 0;
    }
    if message == WM_CLOSE {
        return 0;
    }
    if message == WM_NCDESTROY {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
fn popup(hwnd: HWND, ptr: *mut Context) {
    // Snapshot data before TrackPopupMenu runs its nested message loop. No Rust mutable
    // Context borrow may remain live while another callback can enter window_proc.
    let (russian, busy, paused, events) = unsafe {
        let context = &*ptr;
        (
            context.language == "ru",
            context.busy,
            context.paused,
            context.events.clone(),
        )
    };
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let labels = if russian {
        ["Открыть", "Сканировать", "Пауза", "Выход"]
    } else {
        ["Open", "Scan now", "Pause", "Exit"]
    };
    for (id, label) in [OPEN_ID, SCAN_ID, PAUSE_ID, EXIT_ID]
        .into_iter()
        .zip(labels)
    {
        let flags = MF_STRING
            | if id == SCAN_ID && busy { MF_GRAYED } else { 0 }
            | if id == PAUSE_ID && paused {
                MF_CHECKED
            } else {
                0
            };
        unsafe {
            AppendMenuW(menu, flags, id, wide(label).as_ptr());
        }
    }
    let mut point: POINT = unsafe { std::mem::zeroed() };
    unsafe {
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
    }
    let selected = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        )
    } as usize;
    let event = match selected {
        OPEN_ID => Some(PlatformEvent::Show),
        SCAN_ID if !busy => Some(PlatformEvent::Scan),
        PAUSE_ID => Some(PlatformEvent::Pause),
        EXIT_ID => Some(PlatformEvent::Exit),
        _ => None,
    };
    if let Some(event) = event {
        let _ = events.send(event);
    }
    unsafe {
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}
pub(crate) fn fill_utf16<const N: usize>(target: &mut [u16; N], value: &str) {
    target.fill(0);
    if N == 0 {
        return;
    }
    let mut n = 0;
    for c in value.chars().filter(|c| *c != '\0') {
        let mut units = [0; 2];
        let encoded = c.encode_utf16(&mut units);
        if n + encoded.len() >= N {
            break;
        }
        target[n..n + encoded.len()].copy_from_slice(encoded);
        n += encoded.len();
    }
}

fn best_icon_image(data: &[u8]) -> Option<&[u8]> {
    if data.get(..4)? != [0, 0, 1, 0] {
        return None;
    }
    let count = u16::from_le_bytes(data.get(4..6)?.try_into().ok()?) as usize;
    let entries = data.get(6..6usize.checked_add(count.checked_mul(16)?)?)?;
    let mut best = None;
    for entry in entries.chunks_exact(16) {
        let width = if entry[0] == 0 {
            256
        } else {
            entry[0] as usize
        };
        let height = if entry[1] == 0 {
            256
        } else {
            entry[1] as usize
        };
        let size = u32::from_le_bytes(entry[8..12].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().ok()?) as usize;
        let Some(image) = data.get(offset..offset.checked_add(size)?) else {
            continue;
        };
        let area = width * height;
        if best.as_ref().is_none_or(|(previous, _)| *previous < area) {
            best = Some((area, image));
        }
    }
    best.map(|(_, image)| image)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_blue_icon_contains_a_valid_largest_image() {
        let data = include_bytes!("../../../resources/DiskBurrow.ico");
        let image = best_icon_image(data).unwrap();
        assert!(!image.is_empty());
        assert!(image.len() < data.len());
    }
    #[test]
    fn malformed_icon_directory_is_rejected() {
        for data in [
            &[][..],
            &[0, 0, 1, 0, 255, 255][..],
            &[0, 0, 2, 0, 0, 0][..],
        ] {
            assert!(best_icon_image(data).is_none());
        }
    }
}
