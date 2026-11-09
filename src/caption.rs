//! Своя полоса заголовка, как у Windows Terminal. Системная рамка остаётся: тень, растягивание
//! за края, прилипание к краям экрана и меню окна работают как обычно. Убирается только
//! системный заголовок: клиентская часть начинается от верха окна, а полосу рисует Tune.
//!
//! Так текст в полосе не зависит от текста окна: Windows берёт подпись для панели задач и
//! Alt+Tab из текста окна, а в полосе может быть просто «Tune».
//!
//! Кнопки «свернуть» и «закрыть» — обычные кнопки iced. Над «развернуть» Windows узнаёт
//! кнопку развёртывания (HTMAXBUTTON): на Windows 11 при наведении всплывают раскладки окон.
//! Мышь над ней iced не видит, поэтому наведение и нажатие отслеживаются здесь.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, Stream, StreamExt};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetClientRect, IsZoomed, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_WNDPROC, HTCAPTION, HTCLIENT, HTMAXBUTTON, HTTOP, HTTOPLEFT, HTTOPRIGHT, NCCALCSIZE_PARAMS,
    SM_CXPADDEDBORDER, SM_CYFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_ACTIVATE, WM_MOUSEMOVE,
    WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WNDPROC, IDC_HAND, LoadCursorW, SetCursor, WM_SETCURSOR,
};

/// Высота полосы в точках при масштабе 100%.
pub const HEIGHT: f32 = 36.0;
/// Ширина кнопки окна, как у системных.
pub const BUTTON: f32 = 52.0;

/// Обработчик, стоявший до нас (dpi и winit).
static NEXT: AtomicIsize = AtomicIsize::new(0);
static MAX_HOVER: AtomicBool = AtomicBool::new(false);
static MAX_PRESSED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(true);
/// Полосы нет: клип во весь экран.
static HIDDEN: AtomicBool = AtomicBool::new(false);

type Channel = (UnboundedSender<()>, Mutex<Option<UnboundedReceiver<()>>>);
static EVENTS: OnceLock<Channel> = OnceLock::new();

fn channel() -> &'static Channel {
    EVENTS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// Полосу надо перерисовать: мышь над «развернуть» или окно стало (не)активным.
pub fn events() -> impl Stream<Item = ()> {
    match channel().1.lock().unwrap().take() {
        Some(rx) => rx.left_stream(),
        None => stream::pending().right_stream(),
    }
}

fn set(flag: &AtomicBool, value: bool) {
    if flag.swap(value, Ordering::Relaxed) != value {
        let _ = channel().0.unbounded_send(());
    }
}

/// Мышь над кнопкой «развернуть» и нажата ли она.
pub fn max_state() -> (bool, bool) {
    (MAX_HOVER.load(Ordering::Relaxed), MAX_PRESSED.load(Ordering::Relaxed))
}

/// Прячет полосу: верх окна снова обычная клиентская часть.
pub fn set_hidden(hidden: bool) {
    HIDDEN.store(hidden, Ordering::Relaxed);
}

pub fn hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// Окно в фокусе: в неактивном подпись и значки тусклее, как у системного заголовка.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn zoomed(hwnd: isize) -> bool {
    // SAFETY: только чтение состояния окна.
    unsafe { IsZoomed(hwnd as HWND) != 0 }
}

pub fn toggle_maximize(hwnd: isize) {
    // SAFETY: своё окно.
    unsafe { ShowWindow(hwnd as HWND, if zoomed(hwnd) { SW_RESTORE } else { SW_MAXIMIZE }) };
}

pub fn minimize(hwnd: isize) {
    // SAFETY: своё окно.
    unsafe { ShowWindow(hwnd as HWND, SW_MINIMIZE) };
}

/// Ставит обработчик и просит Windows пересчитать рамку без заголовка. Вызывается один раз.
pub fn install(hwnd: isize) {
    let hwnd = hwnd as HWND;
    // SAFETY: окно наше и живо; прежний обработчик сохраняется и вызывается для всего остального.
    unsafe {
        let next = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as usize as isize);
        NEXT.store(next, Ordering::Relaxed);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

unsafe fn next(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: здесь лежит прежний обработчик, сохранённый при установке.
    let next: WNDPROC = unsafe { std::mem::transmute(NEXT.load(Ordering::Relaxed)) };
    unsafe { CallWindowProcW(next, hwnd, message, wparam, lparam) }
}

/// Толщина рамки, за которую тянут окно.
unsafe fn frame(hwnd: HWND) -> i32 {
    // SAFETY: окно живо.
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

/// Что под точкой экрана в полосе заголовка; `None` — ниже полосы.
unsafe fn hit(hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    let mut point = POINT { x: i32::from(lparam as u16 as i16), y: i32::from((lparam >> 16) as u16 as i16) };
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: окно живо, структуры наши.
    unsafe {
        ScreenToClient(hwnd, &mut point);
        GetClientRect(hwnd, &mut client);
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        // Верхний край растягивает окно, как у системного заголовка; развёрнутое — нет.
        let edge = frame(hwnd);
        if IsZoomed(hwnd) == 0 && point.y < edge {
            return Some(if point.x < edge * 2 {
                HTTOPLEFT
            } else if point.x >= client.right - edge * 2 {
                HTTOPRIGHT
            } else {
                HTTOP
            });
        }
        if point.y >= (HEIGHT * scale).round() as i32 {
            return None;
        }
        let button = (BUTTON * scale).round() as i32;
        Some(match (client.right - point.x) / button.max(1) {
            0 | 2 => HTCLIENT,
            1 => HTMAXBUTTON,
            _ => HTCAPTION,
        })
    }
}

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Заголовок убирается: верх клиентской части — верх окна. Развёрнутое окно выходит за
        // экран на толщину рамки, её и отступаем, иначе полоса уедет за край.
        WM_NCCALCSIZE if wparam != 0 => {
            // SAFETY: при wparam != 0 lparam указывает на NCCALCSIZE_PARAMS.
            let params = unsafe { &mut *(lparam as *mut NCCALCSIZE_PARAMS) };
            let top = params.rgrc[0].top;
            let result = unsafe { next(hwnd, message, wparam, lparam) };
            if result == 0 {
                params.rgrc[0].top = top + if unsafe { IsZoomed(hwnd) } != 0 { unsafe { frame(hwnd) } } else { 0 };
            }
            result
        }
        WM_NCHITTEST => {
            let result = unsafe { next(hwnd, message, wparam, lparam) };
            if result != HTCLIENT as LRESULT || hidden() {
                return result;
            }
            unsafe { hit(hwnd, lparam) }.map_or(result, |hit| hit as LRESULT)
        }
        WM_NCMOUSEMOVE => {
            let over = wparam == HTMAXBUTTON as usize;
            if over {
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                // SAFETY: структура наша, окно живо.
                unsafe { TrackMouseEvent(&mut track) };
            } else {
                set(&MAX_PRESSED, false);
            }
            set(&MAX_HOVER, over);
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
            set(&MAX_HOVER, false);
            set(&MAX_PRESSED, false);
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        // Нажатие на «развернуть» обрабатывается здесь: иначе Windows нарисовала бы поверх
        // свою старую кнопку. Срабатывает отпускание над ней, как у обычной кнопки.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam == HTMAXBUTTON as usize => {
            set(&MAX_PRESSED, true);
            0
        }
        WM_NCLBUTTONUP if wparam == HTMAXBUTTON as usize => {
            if MAX_PRESSED.load(Ordering::Relaxed) {
                set(&MAX_PRESSED, false);
                toggle_maximize(hwnd as isize);
            }
            0
        }
        // Над «развернуть» тот же палец, что над «свернуть» и «закрыть» (их рисует iced).
        WM_SETCURSOR if lparam & 0xFFFF == HTMAXBUTTON as LPARAM => {
            // SAFETY: системный курсор, освобождать не нужно.
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND)) };
            1
        }
        WM_ACTIVATE => {
            set(&ACTIVE, wparam & 0xFFFF != 0);
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { next(hwnd, message, wparam, lparam) },
    }
}
