//! Оформление. Цвета и размеры сняты с интерфейса Яндекс Музыки (тёмная и светлая темы, токены --ym-*).

use iced::border::{self, Border};
use iced::font::{Family, Weight};
use iced::theme::Palette;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use iced::overlay::menu;
use iced::widget::{button, container, pick_list, scrollable, slider, svg, text_input, Scrollable};
use iced::{color, Background, Color, Element, Font, Length, Shadow, Theme};

/// Цвета одной темы. Сняты с токенов --ym-* оригинала: тёмная тема и светлая.
pub struct Colors {
    /// Фон окна и боковой панели.
    pub base: Color,
    pub content: Color,
    pub player: Color,
    pub tile: Color,
    /// Всплывающие окна и меню.
    pub popover: Color,
    pub menu: Color,
    pub tooltip: Color,
    /// Самый контрастный текст: названия, наведённые ссылки.
    pub strong: Color,
    pub text: Color,
    pub muted: Color,
    /// Недоступное: ещё бледнее приглушённого.
    pub faint: Color,
    pub control: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub on_accent: Color,
    pub danger: Color,
}

const fn white(a: f32) -> Color {
    Color { r: 1.0, g: 1.0, b: 1.0, a }
}

const fn black(a: f32) -> Color {
    Color { r: 0.0, g: 0.0, b: 0.0, a }
}

pub const DARK: Colors = Colors {
    base: color!(0x000000),
    content: color!(0x121212),
    player: color!(0x141414),
    tile: color!(0x292929),
    popover: color!(0x1a1a1a),
    menu: color!(0x1f1f1f),
    tooltip: color!(0x2a2a2a),
    strong: color!(0xffffff),
    text: color!(0xe6e6e6),
    muted: white(0.5),
    faint: white(0.3),
    control: white(0.08),
    control_hover: white(0.12),
    control_pressed: white(0.24),
    accent: color!(0xffff00),
    accent_hover: color!(0xdada0b),
    on_accent: color!(0x000000),
    danger: color!(0xf87171),
};

pub const LIGHT: Colors = Colors {
    base: color!(0xf5f5f5),
    content: color!(0xffffff),
    player: color!(0xffffff),
    tile: color!(0xebebeb),
    popover: color!(0xffffff),
    menu: color!(0xffffff),
    tooltip: color!(0xffffff),
    strong: color!(0x000000),
    text: color!(0x1a1a1a),
    muted: color!(0x666666),
    faint: black(0.3),
    control: black(0.06),
    control_hover: black(0.1),
    control_pressed: black(0.2),
    accent: color!(0xffee00),
    accent_hover: color!(0xf5e400),
    on_accent: color!(0x000000),
    danger: color!(0xe5484d),
};

/// Светлая ли сейчас тема. Меняется на ходу: стили читают её при каждой отрисовке.
static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

pub fn set_light(light: bool) {
    LIGHT_THEME.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT_THEME.load(Ordering::Relaxed)
}

/// Полноэкранный плеер лежит на тёмном цвете обложки и в любой теме рисуется тёмной палитрой.
static FULL_DARK: AtomicBool = AtomicBool::new(false);

pub fn set_full_dark(dark: bool) {
    FULL_DARK.store(dark, Ordering::Relaxed);
}

/// Цвета текущей темы.
pub fn c() -> &'static Colors {
    if is_light() && !FULL_DARK.load(Ordering::Relaxed) { &LIGHT } else { &DARK }
}

/// Загружены ли шрифты Яндекса; если нет, используется системный.
pub static YANDEX_FONTS: AtomicBool = AtomicBool::new(false);
/// У аккаунта есть Плюс: тогда значки «целиком с Плюсом» у треков не нужны. До ответа сервера считается, что есть.
pub static HAS_PLUS: AtomicBool = AtomicBool::new(true);

fn text_family() -> &'static str {
    if YANDEX_FONTS.load(Ordering::Relaxed) { "YS Text" } else { "Segoe UI" }
}

pub fn font() -> Font {
    Font { family: Family::Name(text_family()), ..Font::DEFAULT }
}

/// В оригинале названия написаны средним начертанием.
pub fn font_medium() -> Font {
    let weight = if YANDEX_FONTS.load(Ordering::Relaxed) { Weight::Medium } else { Weight::Semibold };
    Font { family: Family::Name(text_family()), weight, ..Font::DEFAULT }
}

pub fn font_headline() -> Font {
    let family = if YANDEX_FONTS.load(Ordering::Relaxed) { "YS Music Headline" } else { "Segoe UI" };
    Font { family: Family::Name(family), weight: Weight::Bold, ..Font::DEFAULT }
}

pub fn theme() -> Theme {
    Theme::custom("Tune", Palette { background: c().base, text: c().text, primary: c().accent, success: c().accent, warning: c().accent, danger: c().danger })
}

pub const LYRICS: Glyph = Glyph { path: "M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-5 14H7v-2h7v2zm3-4H7v-2h10v2zm0-4H7V7h10v2z", original: "syncLyrics_xs", selected: "" };
pub const PLAY: Glyph = Glyph { path: "M8 5v14l11-7z", original: "play_xs", selected: "" };
pub const PAUSE: Glyph = Glyph { path: "M6 19h4V5H6v14zm8-14v14h4V5h-4z", original: "pause_xs", selected: "" };
pub const NEXT: Glyph = Glyph { path: "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z", original: "next_xs", selected: "" };
pub const PREVIOUS: Glyph = Glyph { path: "M6 6h2v12H6zm3.5 6l8.5 6V6z", original: "previous_xs", selected: "" };
pub const VOLUME: Glyph = Glyph { path: "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z", original: "volume_xs", selected: "" };
pub const SEARCH: Glyph = Glyph { path: "M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z", original: "navigationSearch_xs", selected: "" };
pub const WAVE: Glyph = Glyph { path: "M12 1l1.6 7.2L19.8 4l-3.9 6.3 7.1 1.7-7.1 1.7 3.9 6.3-6.2-4.2L12 23l-1.6-7.2L4.2 20l3.9-6.3L1 12l7.1-1.7L4.2 4l6.2 4.2z", original: "navigationMyVibe_xs", selected: "" };
pub const HEART: Glyph = Glyph { path: "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z", original: "liked_xs", selected: "" };
pub const HEART_OUTLINE: Glyph = Glyph { path: "M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z", original: "like_xs", selected: "" };
pub const BACK: Glyph = Glyph { path: "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z", original: "arrowLeft_xs", selected: "" };
pub const CHEVRON: Glyph = Glyph { path: "M9.29 6.71a1 1 0 0 1 1.42 0l4.58 4.59a1 1 0 0 1 0 1.41l-4.58 4.59a1 1 0 1 1-1.42-1.41L13.17 12 9.29 8.12a1 1 0 0 1 0-1.41z", original: "arrowRight_xs", selected: "" };
pub const DISLIKE: Glyph = Glyph { path: "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zM4 12c0-4.42 3.58-8 8-8 1.85 0 3.55.63 4.9 1.69L5.69 16.9C4.63 15.55 4 13.85 4 12zm8 8c-1.85 0-3.55-.63-4.9-1.69L18.31 7.1C19.37 8.45 20 10.15 20 12c0 4.42-3.58 8-8 8z", original: "dislike_xs", selected: "" };
pub const MORE: Glyph = Glyph { path: "M6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4z", original: "more_xs", selected: "" };
pub const ALBUM: Glyph = Glyph { path: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 14a4 4 0 1 1 0-8 4 4 0 0 1 0 8zm0-5a1 1 0 1 0 0 2 1 1 0 0 0 0-2z", original: "album_xs", selected: "" };
pub const ARTIST: Glyph = Glyph { path: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z", original: "artist_xxs", selected: "" };
pub const LINK: Glyph = Glyph { path: "M3.9 12a3.1 3.1 0 0 1 3.1-3.1h4V7H7a5 5 0 0 0 0 10h4v-1.9H7A3.1 3.1 0 0 1 3.9 12zM8 13h8v-2H8v2zm9-6h-4v1.9h4a3.1 3.1 0 0 1 0 6.2h-4V17h4a5 5 0 0 0 0-10z", original: "chain_xs", selected: "" };
pub const QUEUE_NEXT: Glyph = Glyph { path: "M3 6h13v2H3zm0 5h13v2H3zm0 5h9v2H3zm12-1 6 3.5-6 3.5z", original: "playNext_xxs", selected: "" };
pub const QUEUE_LAST: Glyph = Glyph { path: "M3 6h18v2H3zm0 5h18v2H3zm0 5h11v2H3zm14-1v3h-3v2h3v3h2v-3h3v-2h-3v-3z", original: "playLast_xxs", selected: "" };
pub const REPEAT: Glyph = Glyph { path: "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z", original: "repeat_xs", selected: "" };
pub const REPEAT_ONE: Glyph = Glyph { path: "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z", original: "repeat_one_xs", selected: "" };
pub const SHUFFLE: Glyph = Glyph { path: "M10.59 9.17 5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z", original: "shuffle_xs", selected: "" };
pub const VOLUME_OFF: Glyph = Glyph { path: "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3 3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4 9.91 6.09 12 8.18V4z", original: "volumeOff_xs", selected: "" };
pub const OFFLINE: Glyph = Glyph { path: "M23.64 7c-.45-.34-4.93-4-11.64-4-1.5 0-2.89.19-4.15.48L18.18 13.8 23.64 7zm-6.6 8.22L3.27 1.44 2 2.72l2.05 2.06C1.91 5.76.59 6.82.36 7l11.63 14.49.01.01.01-.01 3.9-4.86 3.32 3.32 1.27-1.27-3.46-3.46z", original: "", selected: "" };
pub const DOWNLOAD: Glyph = Glyph { path: "M5 20h14v-2H5v2zM19 9h-4V3H9v6H5l7 7 7-7z", original: "download_xxs", selected: "" };
pub const DOWNLOAD_DONE: Glyph = Glyph { path: "M5 18h14v2H5v-2zm4.6-2.7L5 10.7l2-1.9 2.6 2.6L17 4l2 2-9.4 9.3z", original: "downloaded_xxs", selected: "" };
pub const KIDS: Glyph = Glyph { path: "M9 11.75c-.69 0-1.25.56-1.25 1.25s.56 1.25 1.25 1.25 1.25-.56 1.25-1.25-.56-1.25-1.25-1.25zm6 0c-.69 0-1.25.56-1.25 1.25s.56 1.25 1.25 1.25 1.25-.56 1.25-1.25-.56-1.25-1.25-1.25zM12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8 0-.29.02-.58.05-.86 2.36-1.05 4.23-2.98 5.21-5.37C11.07 8.33 14.05 10 17.42 10c.78 0 1.53-.09 2.25-.26.21.71.33 1.47.33 2.26 0 4.41-3.59 8-8 8z", original: "navigationKids_xs", selected: "navigationKids_selected_xs" };
pub const HEADPHONES: Glyph = Glyph { path: "M12 1c-4.97 0-9 4.03-9 9v7c0 1.66 1.34 3 3 3h3v-8H5v-2c0-3.87 3.13-7 7-7s7 3.13 7 7v2h-4v8h3c1.66 0 3-1.34 3-3v-7c0-4.97-4.03-9-9-9z", original: "navigationNonMusic_xs", selected: "navigationNonMusic_selected_xs" };
pub const REPLAY: Glyph = Glyph { path: "M12 5V1L7 6l5 5V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z", original: "", selected: "" };
pub const FORWARD: Glyph = Glyph { path: "M12 5V1l5 5-5 5V7c-3.31 0-6 2.69-6 6s2.69 6 6 6 6-2.69 6-6h2c0 4.42-3.58 8-8 8s-8-3.58-8-8 3.58-8 8-8z", original: "", selected: "" };
pub const EDIT: Glyph = Glyph { path: "M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z", original: "pencil_xxs", selected: "" };
pub const NOTES: Glyph = Glyph { path: "M3 18h12v-2H3v2zM3 6v2h18V6H3zm0 7h18v-2H3v2z", original: "", selected: "" };
pub const IMAGE: Glyph = Glyph { path: "M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z", original: "picture_xs", selected: "" };
/// Открытый замок: трек играет без цензуры.
pub const UNLOCKED: Glyph = Glyph { path: "M12 17c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm6-9h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6h1.9c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm0 12H6V10h12v10z", original: "", selected: "" };
pub const LOCK: Glyph = Glyph { path: "M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z", original: "lock_xs", selected: "" };
pub const GLOBE: Glyph = Glyph { path: "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z", original: "site_xs", selected: "" };
pub const DELETE: Glyph = Glyph { path: "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z", original: "bucket_xxs", selected: "" };
pub const GRIP: Glyph = Glyph { path: "M11 18c0 1.1-.9 2-2 2s-2-.9-2-2 .9-2 2-2 2 .9 2 2zm-2-8c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0-6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm6 4c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z", original: "dragDots_xxs", selected: "" };
pub const QUEUE: Glyph = Glyph { path: "M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z", original: "playQueue_xs", selected: "" };
pub const TUNE: Glyph = Glyph { path: "M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z", original: "settings_xs", selected: "" };
pub const INFO: Glyph = Glyph { path: "M11 7h2v2h-2zm0 4h2v6h-2zm1-9a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 18a8 8 0 1 1 0-16 8 8 0 0 1 0 16z", original: "info_xxs", selected: "" };
pub const CHECK: Glyph = Glyph { path: "M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z", original: "check_xs", selected: "" };
pub const COLLECTION: Glyph = Glyph { path: HEART_OUTLINE.path, original: "navigationCollection_xs", selected: "navigationCollection_selected_xs" };
pub const HOME: Glyph = Glyph { path: "M12 3 3 10.5V21h6v-6h6v6h6V10.5z", original: "home_m", selected: "home_selected_m" };
pub const LEFT: Glyph = Glyph { path: "M15.4 6.4 14 5l-7 7 7 7 1.4-1.4L9.8 12z", original: "arrowLeft_xs", selected: "" };
pub const RIGHT: Glyph = Glyph { path: "M8.6 6.4 10 5l7 7-7 7-1.4-1.4 5.6-5.6z", original: "arrowRight_xs", selected: "" };
pub const PLUS: Glyph = Glyph { path: "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6z", original: "add_xs", selected: "" };
pub const PIN: Glyph = Glyph { path: "M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z", original: "pin_xs", selected: "pin_filled_xs" };
pub const UP: Glyph = Glyph { path: "M7.41 15.41 12 10.83l4.59 4.58L18 14l-6-6-6 6z", original: "arrowUp_xs", selected: "" };
pub const COLLAPSE: Glyph = Glyph { path: "M7.41 8.59 12 13.17l4.59-4.58L18 10l-6 6-6-6z", original: "arrowDown_xs", selected: "" };
pub const CLIP: Glyph = Glyph { path: "M18 4l2 4h-3l-2-4h-2l2 4h-3l-2-4H8l2 4H7L5 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V4h-4z", original: "clip_xxs", selected: "" };
pub const FULLSCREEN: Glyph = Glyph { path: "M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z", original: "fullscreen_xs", selected: "" };
pub const FULLSCREEN_EXIT: Glyph = Glyph { path: "M5 16h3v3h2v-5H5v2zm3-8H5v2h5V5H8v3zm6 11h2v-3h3v-2h-5v5zm2-11V5h-2v5h5V8h-3z", original: "", selected: "" };
pub const DEVICES: Glyph = Glyph { path: "M1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11zm20-7H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z", original: "cast_xs", selected: "" };
pub const CONCERTS: Glyph = Glyph { path: "M12 3a4 4 0 0 0-4 4v5a4 4 0 0 0 8 0V7a4 4 0 0 0-4-4zm-6 9H4a8 8 0 0 0 7 7.93V22h2v-2.07A8 8 0 0 0 20 12h-2a6 6 0 0 1-12 0z", original: "navigationConcerts_xs", selected: "navigationConcerts_selected_xs" };
pub const TICKET: Glyph = Glyph { path: "M22 10V6c0-1.1-.9-2-2-2H4c-1.1 0-1.99.9-1.99 2v4c1.1 0 1.99.9 1.99 2s-.89 2-2 2v4c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2v-4c-1.1 0-2-.9-2-2s.9-2 2-2zm-2-1.46c-1.19.69-2 1.99-2 3.46s.81 2.77 2 3.46V18H4v-2.54c1.19-.69 2-1.99 2-3.46 0-1.48-.8-2.77-1.99-3.46L4 6h16v2.54z", original: "ticket_m", selected: "" };
pub const PRIVATE: Glyph = Glyph { path: "M12 7c2.76 0 5 2.24 5 5 0 .65-.13 1.26-.36 1.83l2.92 2.92c1.51-1.26 2.7-2.89 3.43-4.75-1.73-4.39-6-7.5-11-7.5-1.4 0-2.74.25-3.98.7l2.16 2.16C10.74 7.13 11.35 7 12 7zM2 4.27l2.28 2.28.46.46C3.08 8.3 1.78 10.02 1 12c1.73 4.39 6 7.5 11 7.5 1.55 0 3.03-.3 4.38-.84l.42.42L19.73 22 21 20.73 3.27 3 2 4.27zM7.53 9.8l1.55 1.55c-.05.21-.08.43-.08.65 0 1.66 1.34 3 3 3 .22 0 .44-.03.65-.08l1.55 1.55c-.67.33-1.41.53-2.2.53-2.76 0-5-2.24-5-5 0-.79.2-1.53.53-2.2zm4.31-.78 3.15 3.15.02-.16c0-1.66-1.34-3-3-3l-.17.01z", original: "eye_crossed_xxs", selected: "" };
/// Кнопки своей полосы заголовка: тонкие, как у системных.
pub const WINDOW_MINIMIZE: Glyph = Glyph { path: "M6 11.5h12v1H6z", original: "", selected: "" };
pub const WINDOW_MAXIMIZE: Glyph = Glyph { path: "M6 6h12v12H6zm1 1v10h10V7z", original: "", selected: "" };
pub const WINDOW_RESTORE: Glyph = Glyph { path: "M6 9h9v9H6zm1 1v7h7v-7zM9 6h9v9h-2v-1h1V7h-7v1H9z", original: "", selected: "" };
pub const WINDOW_CLOSE: Glyph = Glyph { path: "M6.7 6 12 11.3 17.3 6l.7.7-5.3 5.3 5.3 5.3-.7.7-5.3-5.3L6.7 18l-.7-.7 5.3-5.3L6 6.7z", original: "", selected: "" };
pub const CLOSE: Glyph = Glyph { path: "M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z", original: "close_xs", selected: "" };
pub const SETTINGS: Glyph = Glyph { path: "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z", original: "", selected: "" };
pub const LOGOUT: Glyph = Glyph { path: "M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z", original: "", selected: "" };

/// Значок: свой контур и такой же значок из набора Яндекса, если клиент Tune есть на компьютере.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    path: &'static str,
    /// Имя в наборе Яндекса; пустое — такого значка там нет.
    original: &'static str,
    /// Вариант для открытого раздела боковой панели.
    selected: &'static str,
}

impl Glyph {
    /// Тот же значок в выбранном состоянии, если у Яндекса он есть.
    pub fn selected(self) -> Glyph {
        if self.selected.is_empty() { self } else { Glyph { original: self.selected, ..self } }
    }
}

static ORIGINAL: OnceLock<HashMap<String, svg::Handle>> = OnceLock::new();

/// Значки Яндекса, разобранные при запуске. Без них остаются свои.
pub fn set_icons(icons: HashMap<String, Vec<u8>>) {
    let _ = ORIGINAL.set(icons.into_iter().map(|(id, source)| (id, svg::Handle::from_memory(source))).collect());
}

pub fn icon<'a, M: 'a>(glyph: Glyph, size: f32, tint: Color) -> Element<'a, M> {
    // Отрисовка значков не учитывает прозрачность оттенка, поэтому смешиваем с чёрным заранее.
    let tint = Color::from_rgb(tint.r * tint.a, tint.g * tint.a, tint.b * tint.a);
    let handle = ORIGINAL.get().and_then(|icons| icons.get(glyph.original)).cloned().unwrap_or_else(|| {
        let source = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#fff" d="{}"/></svg>"##, glyph.path);
        svg::Handle::from_memory(source.into_bytes())
    });
    svg(handle)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_: &Theme, _| svg::Style { color: Some(tint) })
        .into()
}

fn flat(background: Option<Color>, text_color: Color, radius: f32) -> button::Style {
    button::Style { background: background.map(Background::Color), text_color, border: border::rounded(radius), shadow: Shadow::default(), snap: true }
}

/// Кнопка без фона: значки плеера.
pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control), c().text, 20.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, c().text, 20.0),
    }
}

/// Пункт боковой панели: в оригинале это текст без подложки.
pub fn nav(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control), c().text, 24.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, c().text, 24.0),
    }
}

/// Строка трека, карточка и другие большие области.
pub fn row(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control), c().text, 8.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, c().text, 8.0),
    }
}

/// Строка играющего трека: подсвечена и без наведения.
pub fn row_current(_: &Theme, status: button::Status) -> button::Style {
    flat(Some(if status == button::Status::Pressed { c().control_pressed } else { c().control }), c().text, 8.0)
}

pub fn accent(_: &Theme, status: button::Status) -> button::Style {
    flat(Some(if status == button::Status::Pressed { c().accent_hover } else { c().accent }), c().on_accent, 24.0)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control_pressed), c().text, 24.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(Some(c().control), c().text, 24.0),
    }
}

/// Круглая кнопка запуска и паузы.
pub fn round(_: &Theme, status: button::Status) -> button::Style {
    flat(Some(if status == button::Status::Pressed { c().accent_hover } else { c().accent }), c().on_accent, 40.0)
}

fn fill(color: Color) -> container::Style {
    container::Style { background: Some(Background::Color(color)), text_color: Some(c().text), ..container::Style::default() }
}

pub fn root(_: &Theme) -> container::Style {
    fill(c().base)
}

pub fn content(_: &Theme) -> container::Style {
    container::Style { border: Border { color: c().control, width: 1.0, radius: 12.0.into() }, ..fill(c().content) }
}

pub fn player(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(12), ..fill(c().player) }
}

pub fn tile(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(6), ..fill(c().tile) }
}

/// Всплывающее меню и уведомление: --ym-background-color-primary-enabled-popover.
pub fn popover(_: &Theme) -> container::Style {
    container::Style { border: Border { color: c().control, width: 1.0, radius: 12.0.into() }, ..fill(c().popover) }
}

/// Всплывающее окно посреди появления.
pub fn popover_faded(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let style = popover(theme);
        container::Style {
            background: style.background.map(|back| back.scale_alpha(alpha)),
            border: Border { color: style.border.color.scale_alpha(alpha), ..style.border },
            ..style
        }
    }
}

/// Кнопка всплывающего окна посреди появления.
pub fn button_faded(style: fn(&Theme, button::Status) -> button::Style, alpha: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let base = style(theme, status);
        button::Style { background: base.background.map(|back| back.scale_alpha(alpha)), text_color: base.text_color.scale_alpha(alpha), ..base }
    }
}

/// Поле ввода всплывающего окна посреди появления.
pub fn input_faded(alpha: f32) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |theme, status| {
        let base = input(theme, status);
        text_input::Style {
            background: base.background.scale_alpha(alpha),
            border: Border { color: base.border.color.scale_alpha(alpha), ..base.border },
            icon: base.icon.scale_alpha(alpha),
            placeholder: base.placeholder.scale_alpha(alpha),
            value: base.value.scale_alpha(alpha),
            selection: base.selection.scale_alpha(alpha),
        }
    }
}

/// Всплывающая подсказка: у кнопок и у обрезанного текста одна и та же.
pub fn tip(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(6).color(c().control).width(1), ..fill(c().tooltip) }
}

pub fn avatar(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(24), ..fill(c().tile) }
}

/// Пункт всплывающего меню.
pub fn menu_item(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control), c().text, 6.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, c().text, 6.0),
    }
}

/// Карточка «Слов в волне»: тёплый отсвет акцента из угла переходит в обычную плитку.
/// `alpha` — карточка проявляется, когда пришла.
pub fn word_card(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let glow = Color { a: if is_light() { 0.28 } else { 0.12 } * alpha, ..c().accent };
        let tile = c().tile.scale_alpha(alpha);
        let gradient = iced::gradient::Linear::new(iced::Radians(2.2)).add_stop(0.0, glow).add_stop(0.55, tile).add_stop(1.0, tile);
        container::Style {
            background: Some(Background::Gradient(gradient.into())),
            text_color: Some(c().text.scale_alpha(alpha)),
            border: Border { color: c().control.scale_alpha(alpha), width: 1.0, radius: 18.0.into() },
            ..container::Style::default()
        }
    }
}

/// Кнопка без какой-либо подложки: обложки колеса волны.
pub fn bare(_: &Theme, _: button::Status) -> button::Style {
    flat(None, c().text, 0.0)
}

/// Значок ИИ у карточки «Слов в волне».
pub fn word_badge(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(Color { a: 0.16 * alpha, ..c().accent })), border: border::rounded(8), ..container::Style::default() }
}

/// Оценка карточки «Слов в волне»: выбранная — акцентом.
pub fn word_rate(active: bool, alpha: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (back, text) = match (active, status) {
            (true, button::Status::Pressed) => (c().accent_hover, c().on_accent),
            (true, _) => (c().accent, c().on_accent),
            (false, button::Status::Pressed) => (c().control_pressed, c().text),
            (false, button::Status::Hovered) => (c().control_hover, c().text),
            (false, _) => (c().control, c().text),
        };
        flat(Some(back.scale_alpha(alpha)), text.scale_alpha(alpha), 18.0)
    }
}

/// Фильтр поиска.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (active, status) {
        (true, _) => flat(Some(c().text), c().base, 18.0),
        (false, button::Status::Pressed) => flat(Some(c().control_hover), c().text, 18.0),
        (false, _) => flat(Some(c().control), c().text, 18.0),
    }
}

/// Фильтр во всплывающем окне посреди появления.
pub fn chip_faded(active: bool, alpha: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    let style = chip(active);
    move |theme, status| {
        let base = style(theme, status);
        button::Style { background: base.background.map(|back| back.scale_alpha(alpha)), text_color: base.text_color.scale_alpha(alpha), ..base }
    }
}

/// Карточка альбома, исполнителя, плейлиста: подсвечивается при наведении.
pub fn card(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(c().control), c().text, 12.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, c().text, 12.0),
    }
}

/// Значок explicit.
pub fn explicit(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(3), ..fill(c().control_pressed) }
}

/// Значок подписки.
pub fn plus(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(color!(0xff5c4d))), text_color: Some(Color::WHITE), border: border::rounded(10), ..container::Style::default() }
}

/// Значок поверх обложки играющего трека.
pub fn overlay(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(Color { a: 0.5, ..Color::BLACK })), border: border::rounded(6), ..container::Style::default() }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => c().muted,
        text_input::Status::Hovered => c().control_pressed,
        text_input::Status::Active | text_input::Status::Disabled => c().control_hover,
    };
    text_input::Style {
        background: Background::Color(c().control),
        border: Border { color: edge, width: 1.0, radius: 26.0.into() },
        icon: c().muted,
        placeholder: c().muted,
        value: c().text,
        selection: Color { a: 0.35, ..c().accent },
    }
}

/// Ползунок эквалайзера: тонкая дорожка, жёлтая часть от низа до значения. Выключенный — серый.
pub fn eq_slider(enabled: bool, alpha: f32) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, status| {
        let style = eq_slider_style(enabled, status);
        let fade = |back: Background| match back {
            Background::Color(color) => Background::Color(color.scale_alpha(alpha)),
            other => other,
        };
        slider::Style {
            rail: slider::Rail { backgrounds: (fade(style.rail.backgrounds.0), fade(style.rail.backgrounds.1)), ..style.rail },
            handle: slider::Handle { background: fade(style.handle.background), ..style.handle },
        }
    }
}

/// Выпадающий список во всплывающем окне посреди появления.
pub fn select_faded(alpha: f32) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style {
    move |_, status| {
        let back = if matches!(status, pick_list::Status::Active) { c().control } else { c().control_hover };
        pick_list::Style {
            text_color: c().text.scale_alpha(alpha),
            placeholder_color: c().muted.scale_alpha(alpha),
            handle_color: c().muted.scale_alpha(alpha),
            background: Background::Color(back.scale_alpha(alpha)),
            border: border::rounded(8),
        }
    }
}

/// Раскрытый выпадающий список.
pub fn select_menu(_: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(c().menu),
        border: Border { color: c().control, width: 1.0, radius: 8.0.into() },
        text_color: c().text,
        selected_text_color: c().text,
        selected_background: Background::Color(c().control_hover),
        shadow: Shadow::default(),
    }
}

fn eq_slider_style(enabled: bool, status: slider::Status) -> slider::Style {
    let fill = if enabled { c().accent } else { c().muted };
    let handle = if matches!(status, slider::Status::Dragged) && enabled { c().accent_hover } else { fill };
    slider::Style {
        rail: slider::Rail { backgrounds: (Background::Color(fill), Background::Color(c().control_pressed)), width: 4.0, border: Border { radius: 2.0.into(), ..Border::default() } },
        handle: slider::Handle { shape: slider::HandleShape::Circle { radius: 7.0 }, background: Background::Color(handle), border_width: 0.0, border_color: Color::TRANSPARENT },
    }
}

/// Цвет из ответа Яндекса, вида #2AA75B.
pub fn hex_color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let part = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    Some(Color::from_rgb8(part(0)?, part(2)?, part(4)?))
}

/// Полоса прокрутки: тонкий скруглённый бегунок без дорожки.
const SCROLL_WIDTH: f32 = 6.0;
const SCROLL_MARGIN: f32 = 4.0;
const SCROLL_INNER: f32 = 4.0;
/// Яркость бегунка: в покое, с курсором над списком, над бегунком и когда его тянут.
const SCROLL_REST: f32 = 0.14;
const SCROLL_OVER_LIST: f32 = 0.22;
const SCROLL_OVER_BAR: f32 = 0.42;
const SCROLL_DRAGGED: f32 = 0.6;
/// За сколько секунд яркость проходит весь путь от покоя до максимума.
const SCROLL_FADE: f32 = 0.16;
/// Если полоса столько не рисовалась, переход брошен.
const SCROLL_STALE: Duration = Duration::from_millis(300);

struct ScrollGlow {
    level: f32,
    target: f32,
    last: Option<Instant>,
}

/// Списки на странице видны по одному, поэтому яркость одна на всех.
static SCROLL_GLOW: Mutex<ScrollGlow> = Mutex::new(ScrollGlow { level: SCROLL_REST, target: SCROLL_REST, last: None });

/// Идёт ли ещё переход яркости: тогда нужны новые кадры.
pub fn scroll_moving() -> bool {
    let glow = SCROLL_GLOW.lock().unwrap();
    // Если на странице больше нет полосы, стиль не вызывается и переход никогда не кончится.
    glow.level != glow.target && glow.last.is_none_or(|last| last.elapsed() < SCROLL_STALE)
}

fn scroll_target(status: scrollable::Status) -> f32 {
    match status {
        scrollable::Status::Dragged { is_vertical_scrollbar_dragged: true, .. } => SCROLL_DRAGGED,
        scrollable::Status::Hovered { is_vertical_scrollbar_hovered: true, .. } => SCROLL_OVER_BAR,
        scrollable::Status::Hovered { .. } => SCROLL_OVER_LIST,
        _ => SCROLL_REST,
    }
}

fn scroll_step(level: f32, target: f32, elapsed: f32) -> f32 {
    let delta = elapsed / SCROLL_FADE * (SCROLL_DRAGGED - SCROLL_REST);
    if level < target {
        (level + delta).min(target)
    } else {
        (level - delta).max(target)
    }
}

fn scroll_look(theme: &Theme, status: scrollable::Status, alpha: f32) -> scrollable::Style {
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller { background: Background::Color(Color { a: alpha, ..c().strong }), border: border::rounded(SCROLL_WIDTH / 2.0) },
    };
    scrollable::Style { vertical_rail: rail, horizontal_rail: rail, ..scrollable::default(theme, status) }
}

/// Стиль списка: яркость плавно идёт к нужной при каждой отрисовке.
fn scroll_style(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let now = Instant::now();
    let mut glow = SCROLL_GLOW.lock().unwrap();
    let elapsed = glow.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
    glow.target = scroll_target(status);
    glow.level = scroll_step(glow.level, glow.target, elapsed);
    glow.last = (glow.level != glow.target).then_some(now);
    scroll_look(theme, status, glow.level)
}

/// Для меню поверх страницы: у него своя постоянная яркость, чтобы не спорить со страницей.
pub fn scroll_still(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    scroll_look(theme, status, SCROLL_OVER_LIST)
}

/// Прокручиваемая область с общей полосой.
pub fn scroll<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Scrollable<'a, M> {
    let bar = scrollable::Scrollbar::new().width(SCROLL_WIDTH).scroller_width(SCROLL_WIDTH).margin(SCROLL_MARGIN);
    Scrollable::new(content).direction(scrollable::Direction::Vertical(bar)).style(scroll_style)
}

/// Узкая колонка, которая листается вниз: тонкая полоса не наезжает на содержимое.
pub fn scroll_y_thin<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Scrollable<'a, M> {
    let bar = scrollable::Scrollbar::new().width(SCROLL_INNER).scroller_width(SCROLL_INNER).margin(0).spacing(2);
    Scrollable::new(content).direction(scrollable::Direction::Vertical(bar)).style(scroll_still)
}

/// Лента, которая листается вбок: тонкая полоса снизу.
pub fn scroll_x<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Scrollable<'a, M> {
    let bar = scrollable::Scrollbar::new().width(SCROLL_INNER).scroller_width(SCROLL_INNER).margin(0);
    Scrollable::new(content).direction(scrollable::Direction::Horizontal(bar)).style(scroll_still)
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn reads_hex_colors() {
        assert_eq!(hex_color("#000000"), Some(Color::BLACK));
        assert_eq!(hex_color("#FFFFFF"), Some(Color::WHITE));
        assert_eq!(hex_color("2AA75B"), None);
        assert_eq!(hex_color("#12"), None);
        assert_eq!(hex_color("#GGGGGG"), None);
    }

    #[test]
    fn glow_reaches_target_and_stops() {
        assert_eq!(scroll_target(scrollable::Status::Active { is_horizontal_scrollbar_disabled: true, is_vertical_scrollbar_disabled: false }), SCROLL_REST);
        let halfway = scroll_step(SCROLL_REST, SCROLL_DRAGGED, SCROLL_FADE / 2.0);
        assert!(halfway > SCROLL_REST && halfway < SCROLL_DRAGGED);
        assert_eq!(scroll_step(SCROLL_REST, SCROLL_DRAGGED, SCROLL_FADE), SCROLL_DRAGGED);
        assert_eq!(scroll_step(SCROLL_DRAGGED, SCROLL_REST, SCROLL_FADE), SCROLL_REST);
    }
}
