//! Значок в области уведомлений и его меню, как у официального клиента: клик сворачивает и
//! разворачивает окно, в меню — окно, плеер, «О приложении» и выход. Ещё здесь запрет сна,
//! пока играет музыка (у оригинала `powerSaveBlocker` «prevent-app-suspension»).
//!
//! Значок живёт в своём потоке со скрытым окном: ему нужен свой цикл сообщений.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, Stream, StreamExt};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::{api, fonts};

#[derive(Clone, Debug)]
pub enum Event {
    /// Клик по значку или «Свернуть» / «Развернуть».
    Window,
    Toggle,
    Next,
    Previous,
    Open(&'static str),
    Quit,
}

const TERMS: &str = "https://yandex.ru/legal/music_mobile_agreement/";
const RECOMMENDATIONS: &str = "https://music.yandex.ru/legal/recommendations/ru/#music";
const HELP: &str = "https://yandex.ru/support/music/";

const CALLBACK: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

const CMD_WINDOW: usize = 1;
const CMD_TOGGLE: usize = 2;
const CMD_PREVIOUS: usize = 3;
const CMD_NEXT: usize = 4;
const CMD_TERMS: usize = 5;
const CMD_RECOMMENDATIONS: usize = 6;
const CMD_HELP: usize = 7;
const CMD_QUIT: usize = 8;

type Channel = (UnboundedSender<Event>, Mutex<Option<UnboundedReceiver<Event>>>);
static EVENTS: OnceLock<Channel> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static PLAYING: AtomicBool = AtomicBool::new(false);
/// Окно Tune свёрнуто или спрятано: в меню тогда «Развернуть».
static AWAY: AtomicBool = AtomicBool::new(false);
static HAS_TRACK: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

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

/// Значок оригинала (`favicon.ico` из его архива), сохранённый рядом с настройками.
fn load_icon() -> isize {
    let path = api::data_dir().join("icons").join("tune.ico");
    if !path.exists() {
        if let Some(bytes) = fonts::asar().and_then(|asar| fonts::read_from_asar(&asar, &["app"], "favicon.ico")) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
            let _ = std::fs::write(&path, bytes);
        }
    }
    let name = wide(&path.to_string_lossy());
    // SAFETY: строка с нулём в конце живёт до конца вызова.
    let icon = unsafe { LoadImageW(std::ptr::null_mut(), name.as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE | LR_DEFAULTSIZE) };
    if !icon.is_null() {
        return icon as isize;
    }
    // SAFETY: стандартный значок системы.
    unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) as isize }
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

/// Что показать в меню: играет ли музыка, есть ли трек и свёрнуто ли окно.
/// Заодно запрещает или разрешает сон: поток приложения живёт всё время работы.
pub fn sync(playing: bool, has_track: bool, away: bool) {
    let was = PLAYING.swap(playing, Ordering::Relaxed);
    HAS_TRACK.store(has_track, Ordering::Relaxed);
    AWAY.store(away, Ordering::Relaxed);
    if was != playing {
        let flags = if playing { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
        // SAFETY: только флаги для этого потока.
        unsafe { SetThreadExecutionState(flags) };
    }
}

fn show_menu(hwnd: HWND) {
    // SAFETY: меню создаётся, показывается и удаляется здесь же, в потоке значка.
    unsafe {
        let menu = CreatePopupMenu();
        let about = CreatePopupMenu();
        let item = |menu, id: usize, label: &str, enabled: bool| {
            let text = wide(label);
            AppendMenuW(menu, MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED }, id, text.as_ptr());
        };
        let has_track = HAS_TRACK.load(Ordering::Relaxed);
        item(menu, CMD_WINDOW, if AWAY.load(Ordering::Relaxed) { "Развернуть" } else { "Свернуть" }, true);
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        item(menu, CMD_TOGGLE, if PLAYING.load(Ordering::Relaxed) { "Пауза" } else { "Воспроизведение" }, has_track);
        item(menu, CMD_PREVIOUS, "Предыдущая песня", has_track);
        item(menu, CMD_NEXT, "Следующая песня", has_track);
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        item(about, CMD_TERMS, "Пользовательское соглашение", true);
        item(about, CMD_RECOMMENDATIONS, "Правила рекомендаций", true);
        item(about, CMD_HELP, "Помощь", true);
        let label = wide("О приложении");
        AppendMenuW(menu, MF_POPUP, about as usize, label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        item(menu, CMD_QUIT, "Закрыть приложение", true);
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        // Без этого меню не закрывается кликом мимо него.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, point.x, point.y, 0, hwnd, std::ptr::null()) as usize;
        DestroyMenu(menu);
        match chosen {
            CMD_WINDOW => send(Event::Window),
            CMD_TOGGLE => send(Event::Toggle),
            CMD_PREVIOUS => send(Event::Previous),
            CMD_NEXT => send(Event::Next),
            CMD_TERMS => send(Event::Open(TERMS)),
            CMD_RECOMMENDATIONS => send(Event::Open(RECOMMENDATIONS)),
            CMD_HELP => send(Event::Open(HELP)),
            CMD_QUIT => send(Event::Quit),
            _ => {}
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == CALLBACK {
        match (lparam & 0xffff) as u32 {
            WM_LBUTTONUP => send(Event::Window),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            _ => {}
        }
        return 0;
    }
    // Проводник перезапустился: значок нужно добавить заново.
    if TASKBAR_CREATED.get() == Some(&message) {
        add_icon(hwnd, ICON.load(Ordering::Relaxed));
        return 0;
    }
    // SAFETY: остальное — как у обычного окна.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

static ICON: AtomicIsize = AtomicIsize::new(0);

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
            let icon = load_icon();
            ICON.store(icon, Ordering::Relaxed);
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
