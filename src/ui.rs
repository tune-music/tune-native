//! Оформление. Цвета и размеры сняты с интерфейса Яндекс Музыки (тёмная тема, токены --ym-*).

use iced::border::{self, Border};
use iced::font::{Family, Weight};
use iced::theme::Palette;
use iced::widget::{button, container, slider, svg, text_input};
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

pub const FONT: Font = Font::with_name("Segoe UI");
pub const FONT_MEDIUM: Font = Font { family: Family::Name("Segoe UI"), weight: Weight::Semibold, ..Font::DEFAULT };
pub const FONT_HEADLINE: Font = Font { family: Family::Name("Segoe UI"), weight: Weight::Bold, ..Font::DEFAULT };

pub fn theme() -> Theme {
    Theme::custom("Tune", Palette { background: BLACK, text: TEXT, primary: ACCENT, success: ACCENT, warning: ACCENT, danger: DANGER })
}

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
pub const DISLIKE: &str = "M2.8 4.2 1.4 5.6l3.3 3.3C4.3 9.4 4 10 4 10.6 2.8 11.6 2 13 2 14.5 2 17 4.4 19 7.5 19c1.1 0 2.3-.4 3.2-1l1.3 1.2 1.4-1.3 5.1 5.1 1.4-1.4L2.8 4.2zM16.5 3c-1.7 0-3.4.8-4.5 2.1C10.9 3.8 9.2 3 7.8 3l14 14c.2-1.2.2-2.4.2-3.5V8.5C22 5.4 19.6 3 16.5 3z";
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
        button::Status::Hovered => flat(Some(CONTROL), TEXT, 20.0),
        button::Status::Pressed => flat(Some(CONTROL_HOVER), TEXT, 20.0),
        button::Status::Active | button::Status::Disabled => flat(None, TEXT, 20.0),
    }
}

/// Пункт боковой панели: в оригинале это текст без подложки.
pub fn nav(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered | button::Status::Pressed => flat(Some(CONTROL), TEXT, 24.0),
        button::Status::Active | button::Status::Disabled => flat(None, TEXT, 24.0),
    }
}

/// Строка трека, карточка и другие большие области.
pub fn row(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => flat(Some(CONTROL), TEXT, 8.0),
        button::Status::Pressed => flat(Some(CONTROL_HOVER), TEXT, 8.0),
        button::Status::Active | button::Status::Disabled => flat(None, TEXT, 8.0),
    }
}

pub fn accent(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    flat(Some(if hovered { ACCENT_HOVER } else { ACCENT }), ON_ACCENT, 24.0)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    match status {
        button::Status::Hovered => flat(Some(CONTROL_HOVER), TEXT, 24.0),
        button::Status::Pressed => flat(Some(CONTROL_PRESSED), TEXT, 24.0),
        button::Status::Active | button::Status::Disabled => flat(Some(CONTROL), TEXT, 24.0),
    }
}

/// Круглая кнопка запуска и паузы.
pub fn round(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    flat(Some(if hovered { ACCENT_HOVER } else { ACCENT }), ON_ACCENT, 40.0)
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

pub fn avatar(_: &Theme) -> container::Style {
    container::Style { border: border::rounded(24), ..fill(TILE) }
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

/// Перемотка фоном всей панели плеера, как в оригинале: пройденная часть светлее.
pub fn seek(_: &Theme, _: slider::Status) -> slider::Style {
    slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(CONTROL), Background::Color(Color::TRANSPARENT)),
            width: 80.0,
            border: border::rounded(12),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Rectangle { width: 0, border_radius: 0.0.into() },
            background: Background::Color(Color::TRANSPARENT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub fn volume(_: &Theme, status: slider::Status) -> slider::Style {
    let shown = !matches!(status, slider::Status::Active);
    slider::Style {
        rail: slider::Rail { backgrounds: (Background::Color(TEXT), Background::Color(CONTROL_PRESSED)), width: 4.0, border: border::rounded(2) },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: if shown { 6.0 } else { 0.0 } },
            background: Background::Color(WHITE),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}
