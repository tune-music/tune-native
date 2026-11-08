//! Появление и исчезновение. У iced нет общей прозрачности для целого дерева, поэтому обёртка
//! сдвигает и масштабирует содержимое при отрисовке и может накрыть его цветом фона.
//! Обёртка есть всегда, даже в покое: иначе смена дерева сбрасывала бы прокрутку и фокус.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use std::time::Instant;
use iced::advanced::widget::tree;
use iced::advanced::{overlay, Clipboard, Renderer as _, Shell};
use iced::{mouse, window, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector};

use crate::Message;

/// Быстрое начало и мягкий конец.
pub fn ease_out(level: f32) -> f32 {
    1.0 - (1.0 - level.clamp(0.0, 1.0)).powi(3)
}

/// Масштаб вокруг точки: `x` и `y` — доли размера области.
pub fn scale_around(bounds: Rectangle, x: f32, y: f32, scale: f32) -> Transformation {
    let (x, y) = (bounds.x + bounds.width * x, bounds.y + bounds.height * y);
    Transformation::translate(x, y) * Transformation::scale(scale) * Transformation::translate(-x, -y)
}

/// Элемент, который появляется и исчезает: значение открыто или закрывается.
pub struct Fade<T> {
    pub item: Option<T>,
    pub live: bool,
    level: f32,
    open: f32,
    close: f32,
}

impl<T: PartialEq> Fade<T> {
    pub fn new(open: f32, close: f32) -> Self {
        Self { item: None, live: false, level: 0.0, open, close }
    }

    /// Запоминает текущее значение: после закрытия оно нужно, чтобы дорисовать исчезновение.
    pub fn sync(&mut self, now: Option<T>) {
        self.live = now.is_some();
        if let Some(value) = now {
            if self.item.as_ref() != Some(&value) {
                if self.item.is_some() {
                    self.level = self.level.min(0.35);
                }
                self.item = Some(value);
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.item.is_some() && self.level != if self.live { 1.0 } else { 0.0 }
    }

    pub fn step(&mut self, elapsed: f32) {
        if self.live {
            self.level = (self.level + elapsed / self.open).min(1.0);
        } else {
            self.level = (self.level - elapsed / self.close).max(0.0);
            if self.level == 0.0 {
                self.item = None;
            }
        }
    }

    pub fn shown(&self) -> f32 {
        ease_out(self.level)
    }
}

pub struct Reveal<'a> {
    content: Element<'a, Message>,
    shift: Vector,
    scale: f32,
    origin: (f32, f32),
    veil: Color,
    inert: bool,
    clip: bool,
}

pub fn reveal<'a>(content: impl Into<Element<'a, Message>>) -> Reveal<'a> {
    Reveal { content: content.into(), shift: Vector::ZERO, scale: 1.0, origin: (0.5, 0.5), veil: Color::TRANSPARENT, inert: false, clip: false }
}

impl<'a> Reveal<'a> {
    pub fn shift(mut self, x: f32, y: f32) -> Self {
        self.shift = Vector::new(x, y);
        self
    }

    pub fn scale(mut self, scale: f32, x: f32, y: f32) -> Self {
        self.scale = scale;
        self.origin = (x, y);
        self
    }

    pub fn veil(mut self, veil: Color) -> Self {
        self.veil = veil;
        self
    }

    /// Всё, что содержимое рисует, обрезается по его месту, даже сдвинутое и увеличенное.
    pub fn clip(mut self) -> Self {
        self.clip = true;
        self
    }

    /// Закрывающийся элемент уже не отвечает на мышь и клавиатуру.
    pub fn inert(mut self, inert: bool) -> Self {
        self.inert = inert;
        self
    }
}

impl Widget<Message, Theme, Renderer> for Reveal<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        if !self.inert {
            self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        if self.inert {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let bounds = layout.bounds();
        if self.clip {
            renderer.with_layer(bounds, |renderer| self.draw_moved(tree, renderer, theme, style, layout, cursor, viewport));
        } else {
            self.draw_moved(tree, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl Reveal<'_> {
    /// Область экрана в координатах содержимого: обратно сдвигу и масштабу.
    fn unmove(&self, bounds: Rectangle, seen: Rectangle) -> Rectangle {
        let scale = self.scale.max(0.01);
        let (ox, oy) = (bounds.x + bounds.width * self.origin.0, bounds.y + bounds.height * self.origin.1);
        Rectangle {
            x: (seen.x - self.shift.x - ox) / scale + ox,
            y: (seen.y - self.shift.y - oy) / scale + oy,
            width: seen.width / scale,
            height: seen.height / scale,
        }
    }

    fn draw_moved(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let bounds = layout.bounds();
        // Свои слои (прокрутка, полоса прокрутки) содержимое обрезает по видимой области,
        // а обрезку снаружи не наследует. Поэтому видимая область — наше место, если мы
        // обрезаем, — передаётся внутрь всегда, а под сдвигом пересчитывается обратно:
        // после сдвига и масштаба она ляжет ровно туда, где её видно, и слои не вылезут.
        let seen = if self.clip { bounds.intersection(viewport).unwrap_or(Rectangle::new(bounds.position(), Size::ZERO)) } else { *viewport };
        if self.shift == Vector::ZERO && self.scale == 1.0 {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, &seen);
        } else {
            let move_to = Transformation::translate(self.shift.x, self.shift.y) * scale_around(bounds, self.origin.0, self.origin.1, self.scale);
            let inner = self.unmove(bounds, seen);
            renderer.with_transformation(move_to, |renderer| {
                self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, &inner);
            });
        }
        if self.veil.a > 0.0 {
            renderer.with_layer(bounds, |renderer| {
                renderer.fill_quad(renderer::Quad { bounds, ..renderer::Quad::default() }, self.veil);
            });
        }
    }
}

impl<'a> From<Reveal<'a>> for Element<'a, Message> {
    fn from(reveal: Reveal<'a>) -> Self {
        Element::new(reveal)
    }
}

/// Заслонка: всё, что под ней в стопке слоёв, не видит мыши. Нажатия, колесо и наведение
/// забираются, даже если под курсором пустое место; кнопки внутри работают как обычно.
pub struct Block<'a> {
    content: Element<'a, Message>,
}

pub fn block<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(Block { content: content.into() })
}

impl Widget<Message, Theme, Renderer> for Block<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        let mouse = matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::ButtonReleased(_) | mouse::Event::WheelScrolled { .. }));
        if mouse && cursor.is_over(layout.bounds()) {
            shell.capture_event();
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        // Не «ничего» над пустым местом: стопка тогда не пускает курсор к слоям ниже.
        match self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer) {
            mouse::Interaction::None if cursor.is_over(layout.bounds()) => mouse::Interaction::Idle,
            interaction => interaction,
        }
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// Слой, закрытый сверху непрозрачным окном (клипом): живёт как обычно, но не рисуется —
/// видеокарте не нужно каждый кадр рисовать то, чего не видно.
pub struct Covered<'a> {
    content: Element<'a, Message>,
    hidden: bool,
}

pub fn covered<'a>(content: impl Into<Element<'a, Message>>, hidden: bool) -> Element<'a, Message> {
    Element::new(Covered { content: content.into(), hidden })
}

impl Widget<Message, Theme, Renderer> for Covered<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        // Под закрытым слоем мышь не нужна: и наведение, и анимации кнопок ждут, пока он откроется.
        if !self.hidden {
            self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        if self.hidden {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        if !self.hidden {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        if self.hidden {
            return None;
        }
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// Сколько точек прокрутки даёт один щелчок колеса.
pub const WHEEL_STEP: f32 = 90.0;
/// Как быстро догоняется прокрутка: чем больше, тем короче хвост.
const WHEEL_SPEED: f32 = 14.0;
/// Меньше этого остаток не стоит кадра.
const WHEEL_REST: f32 = 0.5;

/// Какую часть остатка прокрутить за `elapsed` секунд.
fn wheel_part(elapsed: f32) -> f32 {
    1.0 - (-elapsed * WHEEL_SPEED).exp()
}

#[derive(Default)]
struct SmoothState {
    /// Сколько ещё прокрутить.
    left: Vector,
    /// Что-то внутри попросило кадр: стиль полосы прокрутки узнает о новом состоянии только
    /// при отрисовке, поэтому после неё нужен ещё один кадр, чтобы начать переход.
    kick: bool,
    last: Option<Instant>,
}

/// Плавная прокрутка всего, что внутри: колесо не двигает список сразу, а копится здесь
/// и уходит к прокручиваемым элементам маленькими порциями по кадрам.
pub struct Smooth<'a> {
    content: Element<'a, Message>,
}

pub fn smooth<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(Smooth { content: content.into() })
}

impl Widget<Message, Theme, Renderer> for Smooth<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SmoothState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(SmoothState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event {
            let state = tree.state.downcast_mut::<SmoothState>();
            let (x, y) = match *delta {
                mouse::ScrollDelta::Lines { x, y } => (x * WHEEL_STEP, y * WHEEL_STEP),
                mouse::ScrollDelta::Pixels { x, y } => (x, y),
            };
            state.left = state.left + Vector::new(x, y);
            shell.request_redraw();
            return;
        }
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) && shell.redraw_request() != window::RedrawRequest::Wait {
            tree.state.downcast_mut::<SmoothState>().kick = true;
        }
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let state = tree.state.downcast_mut::<SmoothState>();
            if std::mem::take(&mut state.kick) || crate::ui::scroll_moving() {
                shell.request_redraw();
            }
            if state.left == Vector::ZERO {
                state.last = None;
                return;
            }
            let elapsed = state.last.map_or(1.0 / 60.0, |last| now.duration_since(last).as_secs_f32());
            state.last = Some(*now);
            let part = wheel_part(elapsed);
            let mut step = Vector::new(state.left.x * part, state.left.y * part);
            if state.left.x.abs().max(state.left.y.abs()) < WHEEL_REST {
                step = state.left;
            }
            state.left = state.left - step;
            let wheel = Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x: step.x, y: step.y } });
            self.content.as_widget_mut().update(&mut tree.children[0], &wheel, layout, cursor, renderer, clipboard, shell, viewport);
            shell.request_redraw();
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

/// За сколько секунд ссылка откликается на наведение.
const LINE_FADE: f32 = 0.16;

#[derive(Default)]
struct LineState {
    level: f32,
    hovered: bool,
    pressed: bool,
    last: Option<Instant>,
}

/// Текст-ссылка: имя исполнителя, название трека, строка песни. Обычная кнопка меняет цвет
/// мгновенно, поэтому цвет здесь считается сам и передаётся при отрисовке.
pub struct HoverLine<'a> {
    content: Element<'a, Message>,
    rest: Color,
    hover: Color,
    on_press: Message,
}

pub fn hover_line<'a>(content: impl Into<Element<'a, Message>>, rest: Color, hover: Color, on_press: Message) -> HoverLine<'a> {
    HoverLine { content: content.into(), rest, hover, on_press }
}

impl Widget<Message, Theme, Renderer> for HoverLine<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<LineState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(LineState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits)
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        // Внутри может быть подсказка или бегущая строка: им тоже нужно знать о курсоре.
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        let state = tree.state.downcast_mut::<LineState>();
        let hovered = cursor.is_over(layout.bounds());
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                let elapsed = state.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
                let target = if hovered { 1.0 } else { 0.0 };
                state.hovered = hovered;
                let delta = elapsed / LINE_FADE;
                state.level = if hovered { (state.level + delta).min(1.0) } else { (state.level - delta).max(0.0) };
                state.last = (state.level != target).then_some(*now);
                if state.level != target {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if hovered => {
                // Событие забираем сразу, чтобы не сработала строка, внутри которой стоит ссылка.
                state.pressed = true;
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.pressed => {
                state.pressed = false;
                if hovered {
                    shell.publish(self.on_press.clone());
                }
                shell.capture_event();
            }
            _ if hovered != state.hovered => {
                state.hovered = hovered;
                shell.request_redraw();
            }
            _ => {}
        }
    }

    fn mouse_interaction(&self, _: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, _: &Rectangle, _: &Renderer) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, _: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let level = ease_out(tree.state.downcast_ref::<LineState>().level);
        let mix = |from: f32, to: f32| from + (to - from) * level;
        let color = Color { r: mix(self.rest.r, self.hover.r), g: mix(self.rest.g, self.hover.g), b: mix(self.rest.b, self.hover.b), a: mix(self.rest.a, self.hover.a) };
        self.content.as_widget().draw(&tree.children[0], renderer, theme, &renderer::Style { text_color: color }, layout, cursor, viewport);
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a> From<HoverLine<'a>> for Element<'a, Message> {
    fn from(line: HoverLine<'a>) -> Self {
        Element::new(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_keeps_value_until_faded() {
        let mut fade = Fade::new(0.1, 0.1);
        fade.sync(Some(1));
        assert!(fade.busy());
        fade.step(0.1);
        assert!(!fade.busy());
        assert_eq!(fade.shown(), 1.0);
        fade.sync(None);
        fade.step(0.05);
        assert_eq!(fade.item, Some(1));
        fade.step(0.05);
        assert_eq!(fade.item, None);
        assert!(!fade.busy());
    }

    #[test]
    fn wheel_part_grows_with_time() {
        assert_eq!(wheel_part(0.0), 0.0);
        assert!(wheel_part(1.0 / 120.0) < wheel_part(1.0 / 60.0));
        assert!(wheel_part(1.0) > 0.99);
    }

    #[test]
    fn new_value_pops_again() {
        let mut fade = Fade::new(0.1, 0.1);
        fade.sync(Some(1));
        fade.step(0.1);
        fade.sync(Some(2));
        assert!(fade.busy());
        assert_eq!(fade.item, Some(2));
    }
}

/// Раскрывающийся блок: занимает такую долю своей настоящей высоты, а остальное обрезано.
/// Высота берётся из раскладки, поэтому блок растёт ровно до конца текста, без прикидок.
pub struct Grow<'a> {
    content: Element<'a, Message>,
    level: f32,
}

pub fn grow<'a>(content: impl Into<Element<'a, Message>>, level: f32) -> Element<'a, Message> {
    Element::new(Grow { content: content.into(), level: level.clamp(0.0, 1.0) })
}

impl Widget<Message, Theme, Renderer> for Grow<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.content.as_widget().size().width, Length::Shrink)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let inner = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, &limits.loose());
        let full = inner.size();
        layout::Node::with_children(Size::new(full.width, (full.height * self.level).round()), vec![inner])
    }

    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        let inner = layout.children().next().unwrap();
        self.content.as_widget_mut().operate(&mut tree.children[0], inner, renderer, operation);
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        if self.level > 0.0 {
            let inner = layout.children().next().unwrap();
            let cursor = if cursor.is_over(layout.bounds()) { cursor } else { mouse::Cursor::Unavailable };
            self.content.as_widget_mut().update(&mut tree.children[0], event, inner, cursor, renderer, clipboard, shell, viewport);
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        if self.level == 0.0 || !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(&tree.children[0], layout.children().next().unwrap(), cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let Some(seen) = layout.bounds().intersection(viewport) else { return };
        let inner = layout.children().next().unwrap();
        renderer.with_layer(seen, |renderer| self.content.as_widget().draw(&tree.children[0], renderer, theme, style, inner, cursor, &seen));
    }
}
