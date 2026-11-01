//! Появление и исчезновение. У iced нет общей прозрачности для целого дерева, поэтому обёртка
//! сдвигает и масштабирует содержимое при отрисовке и может накрыть его цветом фона.
//! Обёртка есть всегда, даже в покое: иначе смена дерева сбрасывала бы прокрутку и фокус.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{overlay, Clipboard, Renderer as _, Shell};
use iced::{mouse, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Transformation, Vector};

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
}

pub fn reveal<'a>(content: impl Into<Element<'a, Message>>) -> Reveal<'a> {
    Reveal { content: content.into(), shift: Vector::ZERO, scale: 1.0, origin: (0.5, 0.5), veil: Color::TRANSPARENT, inert: false }
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
        if self.shift == Vector::ZERO && self.scale == 1.0 {
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
        } else {
            let move_to = Transformation::translate(self.shift.x, self.shift.y) * scale_around(bounds, self.origin.0, self.origin.1, self.scale);
            renderer.with_transformation(move_to, |renderer| {
                self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
            });
        }
        if self.veil.a > 0.0 {
            renderer.with_layer(bounds, |renderer| {
                renderer.fill_quad(renderer::Quad { bounds, ..renderer::Quad::default() }, self.veil);
            });
        }
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

impl<'a> From<Reveal<'a>> for Element<'a, Message> {
    fn from(reveal: Reveal<'a>) -> Self {
        Element::new(reveal)
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
    fn new_value_pops_again() {
        let mut fade = Fade::new(0.1, 0.1);
        fade.sync(Some(1));
        fade.step(0.1);
        fade.sync(Some(2));
        assert!(fade.busy());
        assert_eq!(fade.item, Some(2));
    }
}
