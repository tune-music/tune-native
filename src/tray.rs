//! Значок в области уведомлений и его меню, как у Tune Client: клик открывает окно, в меню —
//! окно, настройки, перезапуск, плеер и выход, у пунктов свои значки. Ещё здесь запрет сна,
//! пока играет музыка.
//!
//! Значок живёт в своём потоке со скрытым окном: ему нужен свой цикл сообщений.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, Stream, StreamExt};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::api;

#[derive(Clone, Debug)]
pub enum Event {
    /// Клик по значку или «Открыть Tune».
    Show,
    Settings,
    Restart,
    Toggle,
    Next,
    Previous,
    Quit,
}

/// Значок Tune Native: значок Tune Client с жёлтыми ручками (assets/make_icon.py).
pub const ICON: &[u8] = include_bytes!("../assets/tune.ico");

/// Значки пунктов меню — маски прозрачности из Tune Client (assets/menu/make.py), 16/20/24/32.
const MENU_ICONS: &[(&str, [&[u8]; 4])] = &[
    ("music", [include_bytes!("../assets/menu/music_16.a8"), include_bytes!("../assets/menu/music_20.a8"), include_bytes!("../assets/menu/music_24.a8"), include_bytes!("../assets/menu/music_32.a8")]),
    ("tune", [include_bytes!("../assets/menu/tune_16.a8"), include_bytes!("../assets/menu/tune_20.a8"), include_bytes!("../assets/menu/tune_24.a8"), include_bytes!("../assets/menu/tune_32.a8")]),
    ("reload", [include_bytes!("../assets/menu/reload_16.a8"), include_bytes!("../assets/menu/reload_20.a8"), include_bytes!("../assets/menu/reload_24.a8"), include_bytes!("../assets/menu/reload_32.a8")]),
    ("toggle", [include_bytes!("../assets/menu/toggle_16.a8"), include_bytes!("../assets/menu/toggle_20.a8"), include_bytes!("../assets/menu/toggle_24.a8"), include_bytes!("../assets/menu/toggle_32.a8")]),
    ("next", [include_bytes!("../assets/menu/next_16.a8"), include_bytes!("../assets/menu/next_20.a8"), include_bytes!("../assets/menu/next_24.a8"), include_bytes!("../assets/menu/next_32.a8")]),
    ("prev", [include_bytes!("../assets/menu/prev_16.a8"), include_bytes!("../assets/menu/prev_20.a8"), include_bytes!("../assets/menu/prev_24.a8"), include_bytes!("../assets/menu/prev_32.a8")]),
    ("quit", [include_bytes!("../assets/menu/quit_16.a8"), include_bytes!("../assets/menu/quit_20.a8"), include_bytes!("../assets/menu/quit_24.a8"), include_bytes!("../assets/menu/quit_32.a8")]),
];
const MENU_ICON_SIZES: [i32; 4] = [16, 20, 24, 32];

const CALLBACK: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

/// Пункты меню: номер, подпись, значок. 0 — разделитель.
const ITEMS: &[(usize, &str, &str)] = &[
    (1, "Открыть Tune", "music"),
    (2, "Настройки Tune", "tune"),
    (3, "Перезапустить Tune", "reload"),
    (0, "", ""),
    (4, "Пауза / продолжить", "toggle"),
    (5, "Следующий трек", "next"),
    (6, "Предыдущий трек", "prev"),
    (0, "", ""),
    (7, "Выход", "quit"),
];

type Channel = (UnboundedSender<Event>, Mutex<Option<UnboundedReceiver<Event>>>);
static EVENTS: OnceLock<Channel> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static PLAYING: AtomicBool = AtomicBool::new(false);
/// Тема Tune: меню и его значки рисуются в ней.
static LIGHT: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
static TRAY_ICON: AtomicIsize = AtomicIsize::new(0);

fn channel() -> &'static Channel {
    EVENTS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// События значка для подписки приложения. Читать их можно только один раз.
pub fn events() -> impl Stream<Item = Event> {
    match channel().1.lock().unwrap().take() {
        Some(rx) => rx.left_stream(),
        None => stream::pending().right_stream(),
    }
}

fn send(event: Event) {
    let _ = channel().0.unbounded_send(event);
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Значок нужного размера из встроенного .ico.
fn load_icon(size: i32) -> isize {
    if let Some((offset, length)) = icon_entry(ICON, size) {
        // SAFETY: данные .ico встроены в программу, границы записи проверены.
        let icon = unsafe { CreateIconFromResourceEx(ICON.as_ptr().add(offset), length as u32, 1, 0x0003_0000, size, size, LR_DEFAULTCOLOR) };
        if !icon.is_null() {
            return icon as isize;
        }
    }
    // SAFETY: стандартный значок системы.
    unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as isize }
}

/// Смещение и длина картинки в .ico: точного размера, а нет такого — ближайшей крупнее.
/// Запись каталога — 16 байт после 6 байт заголовка: ширина (0 — 256), …, длина, смещение.
fn icon_entry(ico: &[u8], size: i32) -> Option<(usize, usize)> {
    let count = u16::from_le_bytes(ico.get(4..6)?.try_into().ok()?) as usize;
    let word = |at: usize| ico.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    let entries = (0..count).filter_map(|i| {
        let at = 6 + i * 16;
        let width = match *ico.get(at)? { 0 => 256, w => w as i32 };
        let (length, offset) = (word(at + 8)?, word(at + 12)?);
        (offset + length <= ico.len()).then_some((width, offset, length))
    });
    entries.filter(|(width, ..)| *width >= size).min_by_key(|(width, ..)| *width).map(|(_, offset, length)| (offset, length))
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: структура из чисел и массивов, нули для неё допустимы.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ICON_ID;
    data
}

fn add_icon(hwnd: HWND, icon: isize) {
    let mut data = notify_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = CALLBACK;
    data.hIcon = icon as _;
    for (slot, unit) in data.szTip.iter_mut().zip("Tune".encode_utf16()) {
        *slot = unit;
    }
    // SAFETY: данные заполнены, окно живо.
    unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
}

/// Убирает значок: зовётся при выходе, иначе он висит до наведения мыши.
pub fn remove() {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        let data = notify_data(hwnd as HWND);
        // SAFETY: тот же значок, что добавлен.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}

/// Играет ли музыка и какая тема у Tune. Пока играет — сон запрещён: поток приложения
/// живёт всё время работы.
pub fn sync(playing: bool, light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
    let was = PLAYING.swap(playing, Ordering::Relaxed);
    if was != playing {
        let flags = if playing { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
        // SAFETY: только флаги для этого потока.
        unsafe { SetThreadExecutionState(flags) };
    }
}

/// Тёмное или светлое меню. Windows умеет это только недокументированно, через uxtheme
/// (SetPreferredAppMode — №135, FlushMenuThemes — №136), так же делает и Tune Client.
fn menu_theme(light: bool) {
    static UXTHEME: OnceLock<isize> = OnceLock::new();
    let name = wide("uxtheme.dll");
    // SAFETY: системная библиотека, функции берутся по номерам, известным с Windows 10 1903.
    unsafe {
        let module = *UXTHEME.get_or_init(|| LoadLibraryW(name.as_ptr()) as isize);
        if module == 0 {
            return;
        }
        let module = module as windows_sys::Win32::Foundation::HMODULE;
        if let (Some(set_mode), Some(flush)) = (GetProcAddress(module, 135 as *const u8), GetProcAddress(module, 136 as *const u8)) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            // 2 — всегда тёмная, 3 — всегда светлая.
            set_mode(if light { 3 } else { 2 });
            flush();
        }
    }
}

/// Значок пункта: маска нужного размера, залитая цветом текста меню.
fn menu_bitmap(name: &str, light: bool) -> HBITMAP {
    let Some((_, masks)) = MENU_ICONS.iter().find(|(n, _)| *n == name) else { return std::ptr::null_mut() };
    // SAFETY: только чтение метрики.
    let want = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let index = MENU_ICON_SIZES.iter().position(|&s| s >= want).unwrap_or(MENU_ICON_SIZES.len() - 1);
    let size = MENU_ICON_SIZES[index];
    let (r, g, b) = if light { (0x2b_u32, 0x2b_u32, 0x2b_u32) } else { (0xe6, 0xe6, 0xe6) };
    // SAFETY: структура из чисел; растр верхними строками вперёд, 32 бита на точку.
    unsafe {
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
        // Меню ждёт цвет, уже умноженный на прозрачность.
        for (pixel, &alpha) in pixels.iter_mut().zip(masks[index].iter()) {
            let a = alpha as u32;
            *pixel = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
        }
        bitmap
    }
}

fn show_menu(hwnd: HWND) {
    let light = LIGHT.load(Ordering::Relaxed);
    menu_theme(light);
    // SAFETY: меню и значки создаются, показываются и удаляются здесь же, в потоке значка.
    unsafe {
        let menu = CreatePopupMenu();
        let mut bitmaps = Vec::new();
        for &(id, label, icon) in ITEMS {
            if id == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                continue;
            }
            let text = wide(label);
            AppendMenuW(menu, MF_STRING, id, text.as_ptr());
            let bitmap = menu_bitmap(icon, light);
            if !bitmap.is_null() {
                let mut info: MENUITEMINFOW = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
                info.fMask = MIIM_BITMAP;
                info.hbmpItem = bitmap;
                SetMenuItemInfoW(menu, id as u32, 0, &info);
                bitmaps.push(bitmap);
            }
        }
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        // Без этого меню не закрывается кликом мимо него.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, point.x, point.y, 0, hwnd, std::ptr::null()) as usize;
        DestroyMenu(menu);
        for bitmap in bitmaps {
            DeleteObject(bitmap);
        }
        match chosen {
            1 => send(Event::Show),
            2 => send(Event::Settings),
            3 => send(Event::Restart),
            4 => send(Event::Toggle),
            5 => send(Event::Next),
            6 => send(Event::Previous),
            7 => send(Event::Quit),
            _ => {}
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == CALLBACK {
        match (lparam & 0xffff) as u32 {
            WM_LBUTTONUP => send(Event::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        }
        return 0;
    }
    // Проводник перезапустился: значок нужно добавить заново.
    if TASKBAR_CREATED.get() == Some(&message) {
        add_icon(hwnd, TRAY_ICON.load(Ordering::Relaxed));
        return 0;
    }
    // SAFETY: остальное — как у обычного окна.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Значок появляется один раз за запуск.
pub fn start() {
    if WINDOW.load(Ordering::Relaxed) != 0 {
        return;
    }
    std::thread::spawn(|| {
        // SAFETY: окно, класс и цикл сообщений принадлежат этому потоку.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("TuneTray");
            let mut info: WNDCLASSW = std::mem::zeroed();
            info.lpfnWndProc = Some(procedure);
            info.hInstance = instance;
            info.lpszClassName = class.as_ptr();
            RegisterClassW(&info);
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
            if hwnd.is_null() {
                api::log("трей: окно значка не создалось");
                return;
            }
            let created = wide("TaskbarCreated");
            let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(created.as_ptr()));
            let icon = load_icon(GetSystemMetrics(SM_CXSMICON));
            TRAY_ICON.store(icon, Ordering::Relaxed);
            WINDOW.store(hwnd as isize, Ordering::Relaxed);
            add_icon(hwnd, icon);
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_every_tray_size() {
        for size in [16, 20, 24, 32, 48] {
            let (offset, _) = icon_entry(ICON, size).unwrap();
            // Pillow кладёт картинки в .ico как PNG.
            assert_eq!(&ICON[offset..offset + 4], b"\x89PNG", "{size}");
        }
        assert!(icon_entry(ICON, 300).is_none());
    }

    #[test]
    fn menu_masks_match_sizes() {
        for (name, masks) in MENU_ICONS {
            for (mask, size) in masks.iter().zip(MENU_ICON_SIZES) {
                assert_eq!(mask.len(), (size * size) as usize, "{name} {size}");
            }
        }
    }
}
