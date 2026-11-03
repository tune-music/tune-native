//! Надписи, которые не влезают: многоточие вместо обрыва на полуслове, полный текст в подсказке
//! при наведении и бегущая строка для играющего трека. В iced 0.14 многоточия нет, поэтому
//! строка измеряется здесь: где кончается место, там и обрезается.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, paragraph::Plain, Paragraph as _, Renderer as _};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{overlay, Clipboard, Renderer as _, Shell};
use iced::alignment;
use iced::widget::{container, text as label, tooltip};
use iced::{mouse, window, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::Message;

type Para = <Renderer as text::Renderer>::Paragraph;

const ELLIPSIS: &str = "…";
/// Сколько держать курсор над обрезанной надписью, чтобы показался полный текст.
const TIP_DELAY: Duration = Duration::from_millis(600);
const TIP_WIDTH: f32 = 420.0;
/// Бегущая строка: пауза в начале и в конце, скорость в точках за секунду.
const MARQUEE_PAUSE: f32 = 1.0;
const MARQUEE_SPEED: f32 = 40.0;

/// Надпись в одну или несколько строк: что не влезло, заменяется многоточием.
pub struct Fit {
    content: String,
    size: Option<Pixels>,
    font: Option<Font>,
    color: Option<Color>,
    lines: usize,
    width: Length,
    /// Место справа под значок после текста, например «E».
    reserve: f32,
    tip: bool,
    marquee: bool,
    cut: Option<Rc<Cell<bool>>>,
}

pub fn fit(content: impl Into<String>) -> Fit {
    Fit { content: content.into(), size: None, font: None, color: None, lines: 1, width: Length::Shrink, reserve: 0.0, tip: true, marquee: false, cut: None }
}

impl Fit {
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Сколько строк можно занять до многоточия.
    pub fn lines(mut self, lines: usize) -> Self {
        self.lines = lines.max(1);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn reserve(mut self, reserve: f32) -> Self {
        self.reserve = reserve;
        self
    }

    /// Вместо подсказки обрезанная надпись прокручивается, пока над ней курсор.
    pub fn marquee(mut self) -> Self {
        self.marquee = true;
        self.tip = false;
        self
    }

    pub fn no_tip(mut self) -> Self {
        self.tip = false;
        self
    }

    fn line_height(&self, renderer: &Renderer) -> f32 {
        text::LineHeight::default().to_absolute(self.size.unwrap_or_else(|| renderer.default_size())).0
    }
}

#[derive(Default)]
struct FitState {
    /// Вся строка: по ней меряется место и бежит бегущая строка.
    full: Plain<Para>,
    /// То, что видно: вся строка или её начало с многоточием.
    shown: Plain<Para>,
    key: Option<(String, u32, u32, Font, usize)>,
    cut: bool,
    hover: Option<Instant>,
}

fn spec(content: &str, width: f32, size: Pixels, font: Font, wrapping: text::Wrapping) -> text::Text<&str, Font> {
    text::Text {
        content,
        bounds: Size::new(width, f32::INFINITY),
        size,
        line_height: text::LineHeight::default(),
        font,
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping,
    }
}

/// Начало строки, которое вместе с многоточием влезает в `width` и `lines` строк.
fn shorten(content: &str, width: f32, lines: usize, line: f32, make: &dyn Fn(&str) -> Para, full: &Para) -> Option<String> {
    let ellipsis = make(ELLIPSIS).min_bounds().width;
    let fits = |paragraph: &Para| paragraph.min_bounds().width <= width + 0.5 && paragraph.min_bounds().height <= line * lines as f32 + 0.5;
    let point = Point::new((width - ellipsis).max(0.0), line * (lines as f32 - 0.5));
    let mut at = full.hit_test(point).map_or(0, text::Hit::cursor).min(content.len());
    loop {
        while !content.is_char_boundary(at) {
            at -= 1;
        }
        let head = content[..at].trim_end_matches([' ', ',', '.', '-', '—', '–', '·', ':', ';', '(']);
        let candidate = format!("{head}{ELLIPSIS}");
        if at == 0 || fits(&make(&candidate)) {
            return Some(candidate);
        }
        at -= content[..at].chars().next_back().map_or(1, char::len_utf8);
    }
}

impl Widget<Message, Theme, Renderer> for Fit {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<FitState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(FitState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let state = tree.state.downcast_mut::<FitState>();
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let font = self.font.unwrap_or_else(|| renderer.default_font());
        let max = limits.max();
        let width = (max.width - self.reserve).max(0.0);
        let line = self.line_height(renderer);
        let key = (self.content.clone(), width.to_bits(), size.0.to_bits(), font, self.lines);
        if state.key.as_ref() != Some(&key) {
            let wrapping = if self.lines == 1 { text::Wrapping::None } else { text::Wrapping::WordOrGlyph };
            let _ = state.full.update(spec(&self.content, width, size, font, wrapping));
            let bounds = state.full.min_bounds();
            let fits = bounds.width <= width + 0.5 && bounds.height <= line * self.lines as f32 + 0.5;
            let shown = if fits { None } else { shorten(&self.content, width, self.lines, line, &|content| Para::with_text(spec(content, width, size, font, wrapping)), state.full.raw()) };
            state.cut = shown.is_some();
            let _ = state.shown.update(spec(shown.as_deref().unwrap_or(&self.content), width, size, font, wrapping));
            state.key = Some(key);
        }
        if let Some(cut) = &self.cut {
            cut.set(state.cut);
        }
        let bounds = state.shown.min_bounds();
        layout::Node::new(limits.resolve(self.width, Length::Shrink, Size::new(bounds.width.min(width), bounds.height.max(line))))
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, _: &Renderer, _: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, _: &Rectangle) {
        if !self.marquee {
            return;
        }
        let state = tree.state.downcast_mut::<FitState>();
        let over = cursor.is_over(layout.bounds());
        match (over, state.hover) {
            (true, None) => {
                state.hover = Some(Instant::now());
                shell.request_redraw();
            }
            (false, Some(_)) => {
                state.hover = None;
                shell.request_redraw();
            }
            _ => {}
        }
        if state.cut && state.hover.is_some() && matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            shell.request_redraw();
        }
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, _: &Theme, style: &renderer::Style, layout: Layout<'_>, _: mouse::Cursor, viewport: &Rectangle) {
        let state = tree.state.downcast_ref::<FitState>();
        let bounds = layout.bounds();
        let color = self.color.unwrap_or(style.text_color);
        if let Some(at) = state.hover.filter(|_| self.marquee && state.cut) {
            let overflow = (state.full.min_bounds().width - bounds.width).max(0.0);
            let offset = marquee_offset(at.elapsed().as_secs_f32(), overflow);
            let clip = Rectangle { height: bounds.height + 4.0, y: bounds.y - 2.0, ..bounds };
            renderer.with_layer(clip, |renderer| renderer.fill_paragraph(state.full.raw(), Point::new(bounds.x - offset, bounds.y), color, clip));
        } else {
            renderer.fill_paragraph(state.shown.raw(), bounds.position(), color, *viewport);
        }
    }
}

/// Сдвиг бегущей строки: пауза, прокрутка до конца, пауза, снова с начала.
fn marquee_offset(elapsed: f32, overflow: f32) -> f32 {
    let run = overflow / MARQUEE_SPEED;
    let cycle = MARQUEE_PAUSE * 2.0 + run;
    let t = elapsed % cycle;
    ((t - MARQUEE_PAUSE) * MARQUEE_SPEED).clamp(0.0, overflow)
}

impl From<Fit> for Element<'_, Message> {
    fn from(mut fit: Fit) -> Self {
        if !fit.tip {
            return Element::new(fit);
        }
        let cut = Rc::new(Cell::new(false));
        fit.cut = Some(cut.clone());
        let full = container(label(fit.content.clone()).size(13).color(crate::ui::c().text)).max_width(TIP_WIDTH).padding([6, 10]).style(crate::ui::tip);
        tooltip(Element::new(fit), Element::new(Tip { cut, content: full.into() }), tooltip::Position::Bottom).gap(6).padding(0).delay(TIP_DELAY).into()
    }
}

/// Подсказка, которая есть только у обрезанной надписи: иначе она пустая и не видна.
struct Tip<'a> {
    cut: Rc<Cell<bool>>,
    content: Element<'a, Message>,
}

impl Widget<Message, Theme, Renderer> for Tip<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        if !self.cut.get() {
            return layout::Node::new(Size::ZERO);
        }
        let node = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(node.size(), vec![node])
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        if let Some(inner) = layout.children().next().filter(|_| self.cut.get()) {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, inner, cursor, viewport);
        }
    }
}

/// Ряд ссылок, например исполнителей: кто не влез, не показывается, в конце многоточие.
pub struct FitRow<'a> {
    items: Vec<Element<'a, Message>>,
    size: f32,
    font: Font,
    color: Color,
}

pub fn fit_row<'a>(items: Vec<Element<'a, Message>>, size: f32, font: Font, color: Color) -> FitRow<'a> {
    FitRow { items, size, font, color }
}

#[derive(Default)]
struct RowState {
    /// Сколько ссылок видно; `usize::MAX` — все.
    shown: usize,
    ellipsis: f32,
}

impl Widget<Message, Theme, Renderer> for FitRow<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RowState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(RowState { shown: usize::MAX, ellipsis: 0.0 })
    }

    fn children(&self) -> Vec<Tree> {
        self.items.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.items);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let max = limits.max();
        let free = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, max.height));
        let mut nodes: Vec<layout::Node> = self.items.iter_mut().zip(&mut tree.children).map(|(item, tree)| item.as_widget_mut().layout(tree, renderer, &free)).collect();
        let ellipsis = Para::with_text(text::Text {
            content: ELLIPSIS,
            bounds: Size::INFINITE,
            size: Pixels(self.size),
            line_height: text::LineHeight::default(),
            font: self.font,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        })
        .min_bounds()
        .width;
        let total: f32 = nodes.iter().map(|node| node.size().width).sum();
        let mut shown = nodes.len();
        if total > max.width {
            let mut used = 0.0;
            shown = 0;
            for node in &nodes {
                if used + node.size().width + ellipsis > max.width {
                    break;
                }
                used += node.size().width;
                shown += 1;
            }
        }
        let mut x = 0.0;
        let mut height: f32 = 0.0;
        for (index, node) in nodes.iter_mut().enumerate() {
            height = height.max(node.size().height);
            if index < shown {
                *node = node.clone().move_to(Point::new(x, 0.0));
                x += node.size().width;
            } else {
                *node = node.clone().move_to(Point::new(max.width + 10_000.0, 0.0));
            }
        }
        let state = tree.state.downcast_mut::<RowState>();
        state.shown = shown;
        state.ellipsis = ellipsis;
        let width = if shown < nodes.len() { x + ellipsis } else { x };
        layout::Node::with_children(Size::new(width.min(max.width), height), nodes)
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        let shown = tree.state.downcast_ref::<RowState>().shown;
        for ((item, tree), layout) in self.items.iter_mut().zip(&mut tree.children).zip(layout.children()).take(shown) {
            item.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        let shown = tree.state.downcast_ref::<RowState>().shown;
        self.items
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .take(shown)
            .map(|((item, tree), layout)| item.as_widget().mouse_interaction(tree, layout, cursor, viewport, renderer))
            .max()
            .unwrap_or_default()
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let state = tree.state.downcast_ref::<RowState>();
        let mut end = layout.bounds().x;
        for ((item, tree), layout) in self.items.iter().zip(&tree.children).zip(layout.children()).take(state.shown) {
            item.as_widget().draw(tree, renderer, theme, style, layout, cursor, viewport);
            end = layout.bounds().x + layout.bounds().width;
        }
        if state.shown < self.items.len() {
            let spec = text::Text {
                content: ELLIPSIS.to_string(),
                bounds: Size::new(state.ellipsis + 2.0, layout.bounds().height),
                size: Pixels(self.size),
                line_height: text::LineHeight::default(),
                font: self.font,
                align_x: text::Alignment::Default,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::None,
            };
            renderer.fill_text(spec, Point::new(end, layout.bounds().y), self.color, *viewport);
        }
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(&mut self.items, tree, layout, renderer, viewport, translation)
    }
}

impl<'a> From<FitRow<'a>> for Element<'a, Message> {
    fn from(row: FitRow<'a>) -> Self {
        Element::new(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_waits_runs_and_waits() {
        assert_eq!(marquee_offset(0.5, 80.0), 0.0);
        assert_eq!(marquee_offset(MARQUEE_PAUSE + 1.0, 80.0), MARQUEE_SPEED);
        assert_eq!(marquee_offset(MARQUEE_PAUSE + 2.0 + 0.5, 80.0), 80.0);
        assert_eq!(marquee_offset(MARQUEE_PAUSE * 2.0 + 2.0 + 0.1, 80.0), 0.0);
    }
}
