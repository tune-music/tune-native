//! Оформление: палитра, шрифты, значки и стили виджетов.

use iced::border::{self, Border};
use iced::font::{Family, Weight};
use iced::theme::Palette;
use iced::widget::{button, container, slider, svg, text_input};
use iced::{color, Background, Color, Element, Font, Length, Shadow, Theme};

pub const PANEL: Color = color!(0x09090b);
pub const BACKGROUND: Color = color!(0x18181b);
pub const LINE: Color = color!(0x27272a);
pub const HOVER: Color = color!(0x27272a);
pub const PRESSED: Color = color!(0x3f3f46);
pub const TEXT: Color = color!(0xfafafa);
pub const MUTED: Color = color!(0xa1a1aa);
pub const ACCENT: Color = color!(0xffdb4d);
pub const ACCENT_HOVER: Color = color!(0xffe47a);
pub const ON_ACCENT: Color = color!(0x18181b);
pub const DANGER: Color = color!(0xf87171);

pub const FONT: Font = Font::with_name("Segoe UI");
pub const FONT_BOLD: Font = Font { family: Family::Name("Segoe UI"), weight: Weight::Semibold, ..Font::DEFAULT };

const RADIUS: f32 = 6.0;

pub fn theme() -> Theme {
    Theme::custom(
        "Tune",
        Palette { background: BACKGROUND, text: TEXT, primary: ACCENT, success: ACCENT, warning: ACCENT, danger: DANGER },
    )
}

pub const PLAY: &str = "M8 5v14l11-7z";
pub const PAUSE: &str = "M6 19h4V5H6v14zm8-14v14h4V5h-4z";
pub const NEXT: &str = "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z";
pub const PREVIOUS: &str = "M6 6h2v12H6zm3.5 6l8.5 6V6z";
pub const VOLUME: &str = "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z";
pub const SEARCH: &str = "M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z";
pub const LOGOUT: &str = "M17 7l-1.41 1.41L18.17 11H8v2h10.17l-2.58 2.58L17 17l5-5zM4 5h8V3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h8v-2H4V5z";

pub fn icon<'a, M: 'a>(path: &str, size: f32, tint: Color) -> Element<'a, M> {
    let source = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#fff" d="{path}"/></svg>"##);
    svg(svg::Handle::from_memory(source.into_bytes()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_: &Theme, _| svg::Style { color: Some(tint) })
        .into()
}

fn flat(background: Option<Color>, text_color: Color) -> button::Style {
    button::Style {
        background: background.map(Background::Color),
        text_color,
        border: border::rounded(RADIUS),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Кнопка с прозрачным фоном: значки плеера, пункты боковой панели, строки списка.
pub fn ghost(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => flat(Some(HOVER), TEXT),
        button::Status::Pressed => flat(Some(PRESSED), TEXT),
        button::Status::Active | button::Status::Disabled => flat(None, TEXT),
    }
}

/// Открытый раздел в боковой панели.
pub fn selected(_: &Theme, _: button::Status) -> button::Style {
    flat(Some(HOVER), TEXT)
}

pub fn accent(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => flat(Some(ACCENT_HOVER), ON_ACCENT),
        button::Status::Active | button::Status::Disabled => flat(Some(ACCENT), ON_ACCENT),
    }
}

/// Круглая кнопка запуска и паузы.
pub fn round(_: &Theme, status: button::Status) -> button::Style {
    let background = if matches!(status, button::Status::Hovered | button::Status::Pressed) { ACCENT_HOVER } else { ACCENT };
    button::Style { border: border::rounded(20), ..flat(Some(background), ON_ACCENT) }
}

pub fn panel(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(PANEL)), text_color: Some(TEXT), ..container::Style::default() }
}

pub fn page(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(BACKGROUND)), text_color: Some(TEXT), ..container::Style::default() }
}

pub fn line(_: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(LINE)), ..container::Style::default() }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => MUTED,
        text_input::Status::Hovered => PRESSED,
        text_input::Status::Active | text_input::Status::Disabled => LINE,
    };
    text_input::Style {
        background: Background::Color(PANEL),
        border: Border { color: edge, width: 1.0, radius: RADIUS.into() },
        icon: MUTED,
        placeholder: MUTED,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

fn track(active: Color, status: slider::Status) -> slider::Style {
    let shown = !matches!(status, slider::Status::Active);
    slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(active), Background::Color(PRESSED)),
            width: 4.0,
            border: border::rounded(2),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: if shown { 6.0 } else { 0.0 } },
            background: Background::Color(TEXT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// Перемотка: пройденная часть акцентная, ручка появляется при наведении.
pub fn seek(_: &Theme, status: slider::Status) -> slider::Style {
    track(ACCENT, status)
}

pub fn volume(_: &Theme, status: slider::Status) -> slider::Style {
    track(TEXT, status)
}
