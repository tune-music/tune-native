//! Размер, место и развёрнутость окна между запусками. Хранятся в `window.json` как
//! `WINDOWPLACEMENT` Windows: обычное место окна и развёрнуто ли оно. Пишутся, когда окно
//! перестали тащить или тянуть за край и когда его развернули или вернули обратно.
//!
//! Окно создаётся скрытым и показывается уже на своём месте, без прыжка. Место за пределами
//! нынешних мониторов (монитор отключили) не восстанавливается: окно встаёт как обычно.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, SetWindowPlacement, ShowWindow, SW_HIDE, SW_SHOW, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT};

/// Запоминать ли место: у предпросмотров оно своё и не должно затирать настоящее.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Окно было развёрнуто: «вернуть» после «развернуть» тоже надо записать.
static ZOOMED: AtomicBool = AtomicBool::new(false);

fn path() -> std::path::PathBuf {
    crate::api::data_dir().join("window.json")
}

/// Сохранённое место: обычный прямоугольник окна и развёрнуто ли оно.
pub fn load() -> Option<(RECT, bool)> {
    let v: Value = serde_json::from_slice(&std::fs::read(path()).ok()?).ok()?;
    let side = |key: &str| v[key].as_i64().and_then(|x| i32::try_from(x).ok());
    let rect = RECT { left: side("left")?, top: side("top")?, right: side("right")?, bottom: side("bottom")? };
    (rect.right - rect.left >= 200 && rect.bottom - rect.top >= 200).then_some((rect, v["maximized"].as_bool() == Some(true)))
}

/// Записывает место окна. Свёрнутое не пишется: его место — обычное, оно уже записано.
pub fn save(hwnd: isize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: структура наша, окно живо.
    let mut place: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    place.length = size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(hwnd as HWND, &mut place) } == 0 || place.showCmd == windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMINIMIZED as u32 {
        return;
    }
    let maximized = place.showCmd == SW_SHOWMAXIMIZED as u32;
    ZOOMED.store(maximized, Ordering::Relaxed);
    let r = place.rcNormalPosition;
    let value = json!({ "left": r.left, "top": r.top, "right": r.right, "bottom": r.bottom, "maximized": maximized });
    let _ = std::fs::write(path(), value.to_string());
}

/// Окно развернули или вернули: записать, если состояние поменялось.
pub fn zoom_changed(hwnd: isize, zoomed: bool) {
    if ZOOMED.load(Ordering::Relaxed) != zoomed {
        save(hwnd);
    }
}

/// Ставит окно на сохранённое место и показывает, если оно должно быть видно. Вернёт,
/// показано ли окно здесь; `false` — места нет, окно показывает вызывающий.
pub fn restore(hwnd: isize, visible: bool) -> bool {
    ENABLED.store(true, Ordering::Relaxed);
    let Some((rect, maximized)) = load() else { return false };
    // SAFETY: только проверка прямоугольника.
    if unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) }.is_null() {
        return false;
    }
    ZOOMED.store(maximized, Ordering::Relaxed);
    let show = match (visible, maximized) {
        (false, _) => SW_HIDE,
        (true, true) => SW_SHOWMAXIMIZED,
        (true, false) => SW_SHOWNORMAL,
    };
    let place = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, flags: 0, showCmd: show as u32, ptMinPosition: POINT { x: -1, y: -1 }, ptMaxPosition: POINT { x: -1, y: -1 }, rcNormalPosition: rect };
    // SAFETY: своё окно, структура наша.
    unsafe { SetWindowPlacement(hwnd as HWND, &place) };
    true
}

/// Показывает окно из трея: развёрнутое — развёрнутым.
pub fn show(hwnd: isize) {
    let show = if ZOOMED.load(Ordering::Relaxed) { SW_SHOWMAXIMIZED } else { SW_SHOW };
    // SAFETY: своё окно.
    unsafe { ShowWindow(hwnd as HWND, show) };
}
