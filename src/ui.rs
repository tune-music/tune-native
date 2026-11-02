//! Оформление. Цвета и размеры сняты с интерфейса Яндекс Музыки (тёмная тема, токены --ym-*).

use iced::border::{self, Border};
use iced::font::{Family, Weight};
use iced::theme::Palette;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iced::widget::{button, container, scrollable, svg, text_input, Scrollable};
use iced::{color, Background, Color, Element, Font, Length, Shadow, Theme};

pub const BLACK: Color = color!(0x000000);
pub const CONTENT: Color = color!(0x121212);
pub const PLAYER: Color = color!(0x141414);
pub const TILE: Color = color!(0x292929);
pub const WHITE: Color = color!(0xffffff);
pub const TEXT: Color = color!(0xe6e6e6);
pub const MUTED: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.5 };
pub const CONTROL: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.08 };
pub const CONTROL_HOVER: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.12 };
pub const CONTROL_PRESSED: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.24 };
pub const ACCENT: Color = color!(0xffff00);
pub const ACCENT_HOVER: Color = color!(0xdada0b);
pub const ON_ACCENT: Color = color!(0x000000);
pub const DANGER: Color = color!(0xf87171);

/// Загружены ли шрифты Яндекса; если нет, используется системный.
pub static YANDEX_FONTS: AtomicBool = AtomicBool::new(false);

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
    Theme::custom("Tune", Palette { background: BLACK, text: TEXT, primary: ACCENT, success: ACCENT, warning: ACCENT, danger: DANGER })
}

pub const LYRICS: &str = "M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-5 14H7v-2h7v2zm3-4H7v-2h10v2zm0-4H7V7h10v2z";
pub const PLAY: &str = "M8 5v14l11-7z";
pub const PAUSE: &str = "M6 19h4V5H6v14zm8-14v14h4V5h-4z";
pub const NEXT: &str = "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z";
pub const PREVIOUS: &str = "M6 6h2v12H6zm3.5 6l8.5 6V6z";
pub const VOLUME: &str = "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z";
pub const SEARCH: &str = "M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z";
pub const WAVE: &str = "M12 1l1.6 7.2L19.8 4l-3.9 6.3 7.1 1.7-7.1 1.7 3.9 6.3-6.2-4.2L12 23l-1.6-7.2L4.2 20l3.9-6.3L1 12l7.1-1.7L4.2 4l6.2 4.2z";
pub const HEART: &str = "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";
pub const HEART_OUTLINE: &str = "M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z";
pub const BACK: &str = "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z";
pub const CHEVRON: &str = "M9.29 6.71a1 1 0 0 1 1.42 0l4.58 4.59a1 1 0 0 1 0 1.41l-4.58 4.59a1 1 0 1 1-1.42-1.41L13.17 12 9.29 8.12a1 1 0 0 1 0-1.41z";
pub const DISLIKE: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zM4 12c0-4.42 3.58-8 8-8 1.85 0 3.55.63 4.9 1.69L5.69 16.9C4.63 15.55 4 13.85 4 12zm8 8c-1.85 0-3.55-.63-4.9-1.69L18.31 7.1C19.37 8.45 20 10.15 20 12c0 4.42-3.58 8-8 8z";
pub const MORE: &str = "M6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4z";
pub const ALBUM: &str = "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm0 14a4 4 0 1 1 0-8 4 4 0 0 1 0 8zm0-5a1 1 0 1 0 0 2 1 1 0 0 0 0-2z";
pub const ARTIST: &str = "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z";
pub const LINK: &str = "M3.9 12a3.1 3.1 0 0 1 3.1-3.1h4V7H7a5 5 0 0 0 0 10h4v-1.9H7A3.1 3.1 0 0 1 3.9 12zM8 13h8v-2H8v2zm9-6h-4v1.9h4a3.1 3.1 0 0 1 0 6.2h-4V17h4a5 5 0 0 0 0-10z";
pub const QUEUE_NEXT: &str = "M3 6h13v2H3zm0 5h13v2H3zm0 5h9v2H3zm12-1 6 3.5-6 3.5z";
pub const QUEUE_LAST: &str = "M3 6h18v2H3zm0 5h18v2H3zm0 5h11v2H3zm14-1v3h-3v2h3v3h2v-3h3v-2h-3v-3z";
pub const REPEAT: &str = "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z";
pub const REPEAT_ONE: &str = "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z";
pub const SHUFFLE: &str = "M10.59 9.17 5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z";
pub const VOLUME_OFF: &str = "M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3 3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4 9.91 6.09 12 8.18V4z";
pub const QUEUE: &str = "M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z";
pub const TUNE: &str = "M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z";
pub const CHECK: &str = "M9 16.17 4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z";
pub const HOME: &str = "M12 3 3 10.5V21h6v-6h6v6h6V10.5z";
pub const LEFT: &str = "M15.4 6.4 14 5l-7 7 7 7 1.4-1.4L9.8 12z";
pub const RIGHT: &str = "M8.6 6.4 10 5l7 7-7 7-1.4-1.4 5.6-5.6z";
pub const PLUS: &str = "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6z";
pub const PIN: &str = "M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z";
pub const CLOSE: &str = "M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z";
pub const LOGOUT: &str = "M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z";

pub fn icon<'a, M: 'a>(path: &str, size: f32, tint: Color) -> Element<'a, M> {
    // Отрисовка значков не учитывает прозрачность оттенка, поэтому смешиваем с чёрным заранее.
    let tint = Color::from_rgb(tint.r * tint.a, tint.g * tint.a, tint.b * tint.a);
    let source = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#fff" d="{path}"/></svg>"##);
    svg(svg::Handle::from_memory(source.into_bytes()))
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
        button::Status::Pressed => flat(Some(CONTROL), TEXT, 20.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, TEXT, 20.0),
    }
}

/// Пункт боковой панели: в оригинале это текст без подложки.
pub fn nav(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(CONTROL), TEXT, 24.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, TEXT, 24.0),
    }
}

/// Строка трека, карточка и другие большие области.
pub fn row(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(CONTROL), TEXT, 8.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, TEXT, 8.0),
    }
}

pub fn accent(_: &Theme, status: button::Status) -> button::Style {
    flat(Some(if status == button::Status::Pressed { ACCENT_HOVER } else { ACCENT }), ON_ACCENT, 24.0)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(CONTROL_PRESSED), TEXT, 24.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(Some(CONTROL), TEXT, 24.0),
    }
}

/// Круглая кнопка запуска и паузы.
pub fn round(_: &Theme, status: button::Status) -> button::Style {
    flat(Some(if status == button::Status::Pressed { ACCENT_HOVER } else { ACCENT }), ON_ACCENT, 40.0)
}

fn fill(color: Color) -> container::Style {
    container::Style { background: Some(Background::Color(color)), text_color: Some(TEXT), ..container::Style::default() }
}

pub fn root(_: &Theme) -> container::Style {
    fill(BLACK)
}

pub fn content(_: &Theme) -> container::Style {
    container::Style { border: Border { color: CONTROL, width: 1.0, radius: 12.0.into() }, ..fill(CONTENT) }
}

pub fn player(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(12), ..fill(PLAYER) }
}

pub fn tile(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(6), ..fill(TILE) }
}

/// Всплывающее меню и уведомление: --ym-background-color-primary-enabled-popover.
pub fn popover(_: &Theme) -> container::Style {
    container::Style { border: Border { color: CONTROL, width: 1.0, radius: 12.0.into() }, ..fill(color!(0x1a1a1a)) }
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

pub fn avatar(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(24), ..fill(TILE) }
}

/// Пункт всплывающего меню.
pub fn menu_item(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Pressed => flat(Some(CONTROL), TEXT, 6.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, TEXT, 6.0),
    }
}

/// Фильтр поиска.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| match (active, status) {
        (true, _) => flat(Some(TEXT), BLACK, 18.0),
        (false, button::Status::Pressed) => flat(Some(CONTROL_HOVER), TEXT, 18.0),
        (false, _) => flat(Some(CONTROL), TEXT, 18.0),
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
        button::Status::Pressed => flat(Some(CONTROL), TEXT, 12.0),
        button::Status::Hovered | button::Status::Active | button::Status::Disabled => flat(None, TEXT, 12.0),
    }
}

/// Значок explicit.
pub fn explicit(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(3), ..fill(CONTROL_PRESSED) }
}

/// Значок подписки.
pub fn plus(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(color!(0xff5c4d))), text_color: Some(WHITE), border: border::rounded(10), ..container::Style::default() }
}

/// Значок поверх обложки играющего трека.
pub fn overlay(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(Color { a: 0.5, ..BLACK })), border: border::rounded(6), ..container::Style::default() }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => MUTED,
        text_input::Status::Hovered => CONTROL_PRESSED,
        text_input::Status::Active | text_input::Status::Disabled => CONTROL_HOVER,
    };
    text_input::Style {
        background: Background::Color(CONTROL),
        border: Border { color: edge, width: 1.0, radius: 26.0.into() },
        icon: MUTED,
        placeholder: MUTED,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

/// Источник музыки над треком на главной.
pub fn source_button(_: &Theme, _: button::Status) -> button::Style {
    flat(None, ACCENT, 20.0)
}

/// Полоса прокрутки: тонкий скруглённый бегунок без дорожки.
const SCROLL_WIDTH: f32 = 6.0;
const SCROLL_MARGIN: f32 = 4.0;
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
        scroller: scrollable::Scroller { background: Background::Color(Color { a: alpha, ..WHITE }), border: border::rounded(SCROLL_WIDTH / 2.0) },
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

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn glow_reaches_target_and_stops() {
        assert_eq!(scroll_target(scrollable::Status::Active { is_horizontal_scrollbar_disabled: true, is_vertical_scrollbar_disabled: false }), SCROLL_REST);
        let halfway = scroll_step(SCROLL_REST, SCROLL_DRAGGED, SCROLL_FADE / 2.0);
        assert!(halfway > SCROLL_REST && halfway < SCROLL_DRAGGED);
        assert_eq!(scroll_step(SCROLL_REST, SCROLL_DRAGGED, SCROLL_FADE), SCROLL_DRAGGED);
        assert_eq!(scroll_step(SCROLL_DRAGGED, SCROLL_REST, SCROLL_FADE), SCROLL_REST);
    }
}
