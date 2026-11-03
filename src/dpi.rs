//! Перенос окна между мониторами с разным масштабом. После смены масштаба winit ставит окно
//! так, чтобы курсор остался на том же месте заголовка. Окно может встать поровну на оба
//! монитора, масштаб тут же меняется обратно, а Windows, ведя перетаскивание, растягивает
//! окно ещё раз: за пару движений мыши оно раздувается в разы. Здесь окно при смене масштаба
//! встаёт ровно туда, куда советует Windows, как велит документация WM_DPICHANGED. Мелкую
//! погрешность размера после всех смен убирает возврат к запомненному логическому размеру.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetClientRect, GetWindowLongPtrW, IsIconic, IsZoomed, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE, SIZE_RESTORED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WINDOWPOS, WM_DPICHANGED, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE, WM_SIZE, WM_WINDOWPOSCHANGING, WNDPROC,
};

/// Обработчик окна от winit.
static NEXT: AtomicIsize = AtomicIsize::new(0);
/// Сколько смен масштаба сейчас обрабатывается; внутри них размер окна не запоминается.
static DEPTH: AtomicU32 = AtomicU32::new(0);
/// Окно сейчас тащат или тянут за край.
static MOVING: AtomicBool = AtomicBool::new(false);
/// Куда Windows советует поставить окно при текущей смене масштаба.
static SUGGESTED: Mutex<Option<RECT>> = Mutex::new(None);
/// Размер клиентской части при масштабе 100%: его окно сохраняет на любом мониторе.
static LOGICAL: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

/// Ставит обработчик перед winit. Вызывается один раз, когда окно готово.
pub fn guard(hwnd: isize) {
    let hwnd = hwnd as HWND;
    // SAFETY: окно наше и живо; прежний обработчик сохраняется и вызывается для всего остального.
    unsafe {
        remember(hwnd);
        let next = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as usize as isize);
        NEXT.store(next, Ordering::Relaxed);
    }
}

unsafe fn next(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: здесь лежит обработчик winit, сохранённый при установке.
    let next: WNDPROC = unsafe { std::mem::transmute(NEXT.load(Ordering::Relaxed)) };
    unsafe { CallWindowProcW(next, hwnd, message, wparam, lparam) }
}

/// Запоминает размер окна в единицах масштаба 100%, если оно в обычном состоянии.
unsafe fn remember(hwnd: HWND) {
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: окно живо, структура наша.
    unsafe {
        if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 || GetClientRect(hwnd, &mut client) == 0 || client.right == 0 {
            return;
        }
        let scale = f64::from(GetDpiForWindow(hwnd)) / 96.0;
        *LOGICAL.lock().unwrap() = (f64::from(client.right) / scale, f64::from(client.bottom) / scale);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_ENTERSIZEMOVE => {
            MOVING.store(true, Ordering::Relaxed);
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        WM_EXITSIZEMOVE => {
            MOVING.store(false, Ordering::Relaxed);
            let result = unsafe { next(hwnd, message, wparam, lparam) };
            // SAFETY: окно живо, перетаскивание закончилось.
            unsafe { settle(hwnd) };
            result
        }
        WM_SIZE => {
            if wparam == SIZE_RESTORED as usize && DEPTH.load(Ordering::Relaxed) == 0 {
                // SAFETY: окно живо.
                unsafe { remember(hwnd) };
            }
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        WM_WINDOWPOSCHANGING => {
            // Окно, которое winit ставит при смене масштаба, встаёт в совет Windows.
            if let Some(rect) = *SUGGESTED.lock().unwrap() {
                // SAFETY: для этого сообщения lparam указывает на WINDOWPOS, который можно править.
                let pos = unsafe { &mut *(lparam as *mut WINDOWPOS) };
                (pos.x, pos.y, pos.cx, pos.cy) = (rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top);
                pos.flags &= !(SWP_NOMOVE | SWP_NOSIZE);
            }
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        WM_DPICHANGED => {
            // SAFETY: для этого сообщения lparam указывает на RECT.
            let rect = unsafe { *(lparam as *const RECT) };
            DEPTH.fetch_add(1, Ordering::Relaxed);
            // Вложенная смена приходит со своим советом; после неё возвращается совет внешней.
            let outer = SUGGESTED.lock().unwrap().replace(rect);
            let result = unsafe { next(hwnd, message, wparam, lparam) };
            *SUGGESTED.lock().unwrap() = outer;
            if DEPTH.fetch_sub(1, Ordering::Relaxed) == 1 && !MOVING.load(Ordering::Relaxed) {
                // SAFETY: окно живо, смены масштаба закончились.
                unsafe { settle(hwnd) };
            }
            result
        }
        _ => unsafe { next(hwnd, message, wparam, lparam) },
    }
}

/// Возвращает окну запомненный логический размер, если он разошёлся на пару пикселей.
unsafe fn settle(hwnd: HWND) {
    // SAFETY: окно живо, структуры наши.
    unsafe {
        if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
            return;
        }
        let dpi = GetDpiForWindow(hwnd);
        let (width, height) = *LOGICAL.lock().unwrap();
        let scale = f64::from(dpi) / 96.0;
        let (want_width, want_height) = ((width * scale).round() as i32, (height * scale).round() as i32);
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut client);
        if width == 0.0 || (client.right, client.bottom) == (want_width, want_height) {
            return;
        }
        let mut frame = RECT { left: 0, top: 0, right: want_width, bottom: want_height };
        AdjustWindowRectExForDpi(&mut frame, GetWindowLongPtrW(hwnd, GWL_STYLE) as u32, 0, GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32, dpi);
        DEPTH.fetch_add(1, Ordering::Relaxed);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, frame.right - frame.left, frame.bottom - frame.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        DEPTH.fetch_sub(1, Ordering::Relaxed);
    }
}
