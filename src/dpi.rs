//! Перенос окна между мониторами с разным масштабом. Когда winit сдвигает окно под курсор
//! после смены масштаба, оно может снова оказаться в основном на прежнем мониторе, и Windows
//! присылает новую смену прямо изнутри первой. Вложенный вызов winit считает от ещё не
//! обновлённого размера, и окно с каждым разом раздувается. Здесь вложенная смена
//! откладывается до конца внешней, а после всех смен окну возвращается его размер.

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetClientRect, GetWindowLongPtrW, GetWindowRect, IsIconic, IsZoomed, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC, GWL_EXSTYLE, GWL_STYLE, SIZE_RESTORED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WM_DPICHANGED, WM_SIZE, WNDPROC,
};

/// Обработчик окна от winit.
static NEXT: AtomicIsize = AtomicIsize::new(0);
/// Сколько смен масштаба сейчас обрабатывается; внутри них размер окна не запоминается.
static DEPTH: AtomicU32 = AtomicU32::new(0);
/// Масштаб, о котором знает winit.
static TOLD: AtomicU32 = AtomicU32::new(0);
/// Размер клиентской части при масштабе 100%: его окно сохраняет на любом мониторе.
static LOGICAL: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

/// Ставит обработчик перед winit. Вызывается один раз, когда окно готово.
pub fn guard(hwnd: isize) {
    let hwnd = hwnd as HWND;
    // SAFETY: окно наше и живо; прежний обработчик сохраняется и вызывается для всего остального.
    unsafe {
        TOLD.store(GetDpiForWindow(hwnd), Ordering::Relaxed);
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
        WM_SIZE => {
            if wparam == SIZE_RESTORED as usize && DEPTH.load(Ordering::Relaxed) == 0 {
                // SAFETY: окно живо.
                unsafe { remember(hwnd) };
            }
            unsafe { next(hwnd, message, wparam, lparam) }
        }
        // Вложенную смену winit не видит: её доставит `settle` с честным размером окна.
        WM_DPICHANGED if DEPTH.load(Ordering::Relaxed) > 0 => 0,
        WM_DPICHANGED => {
            DEPTH.fetch_add(1, Ordering::Relaxed);
            let result = unsafe { next(hwnd, message, wparam, lparam) };
            TOLD.store(u32::from(wparam as u16), Ordering::Relaxed);
            // SAFETY: окно живо, смена масштаба только что обработана.
            unsafe { settle(hwnd) };
            DEPTH.fetch_sub(1, Ordering::Relaxed);
            result
        }
        _ => unsafe { next(hwnd, message, wparam, lparam) },
    }
}

/// Доводит окно до порядка: winit знает настоящий масштаб, а размер — прежний логический.
unsafe fn settle(hwnd: HWND) {
    for _ in 0..4 {
        // SAFETY: окно живо, структуры наши.
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            let mut outer = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut outer);
            if dpi != TOLD.load(Ordering::Relaxed) {
                // Отложенная смена: winit берёт из прямоугольника только место окна.
                next(hwnd, WM_DPICHANGED, (dpi | dpi << 16) as usize, std::ptr::from_ref(&outer) as LPARAM);
                TOLD.store(dpi, Ordering::Relaxed);
                continue;
            }
            if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
                return;
            }
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
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, frame.right - frame.left, frame.bottom - frame.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}
