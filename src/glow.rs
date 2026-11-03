//! Плавная подсветка при наведении. Стиль кнопки меняется мгновенно, поэтому фон рисует
//! обёртка: сама считает яркость и просит перерисовку, только пока она меняется.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree, Widget};
use iced::advanced::{overlay, Clipboard, Renderer as _, Shell};
use iced::{border, mouse, window, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

use crate::{anim, ui, Message};

/// За сколько секунд подсветка загорается полностью.
const FADE: f32 = 0.14;
/// За сколько секунд кнопка проседает при нажатии и на сколько точек с каждой стороны.
const PRESS: f32 = 0.07;
const PRESS_INSET: f32 = 1.5;
/// Осветление кнопок, у которых есть свой фон.
const SHINE: f32 = 0.1;

#[derive(Clone, Copy)]
struct State {
    /// От 0 до 1.
    level: f32,
    hovered: bool,
    /// Насколько кнопка нажата, от 0 до 1.
    press: f32,
    pressed: bool,
    last: Option<Instant>,
}

/// Куда сдвинуть яркость за `elapsed` секунд.
fn step(level: f32, hovered: bool, elapsed: f32) -> f32 {
    let delta = elapsed / FADE;
    if hovered {
        (level + delta).min(1.0)
    } else {
        (level - delta).max(0.0)
    }
}

pub struct Glow<'a> {
    content: Element<'a, Message>,
    radius: f32,
    /// Осветлять поверх своего фона вместо подложки.
    shine: bool,
}

pub fn glow<'a>(content: impl Into<Element<'a, Message>>, radius: f32) -> Element<'a, Message> {
    Element::new(Glow { content: content.into(), radius, shine: false })
}

/// Для кнопок со своим фоном.
pub fn shine<'a>(content: impl Into<Element<'a, Message>>, radius: f32) -> Element<'a, Message> {
    Element::new(Glow { content: content.into(), radius, shine: true })
}

impl Widget<Message, Theme, Renderer> for Glow<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State { level: 0.0, hovered: false, press: 0.0, pressed: false, last: None })
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
        self.content.as_widget_mut().update(&mut tree.children[0], event, layout, cursor, renderer, clipboard, shell, viewport);
        let state = tree.state.downcast_mut::<State>();
        let hovered = cursor.is_over(layout.bounds());
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                let elapsed = state.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
                state.hovered = hovered;
                state.level = step(state.level, hovered, elapsed);
                state.press = (state.press + if state.pressed { elapsed } else { -elapsed } / PRESS).clamp(0.0, 1.0);
                let moving = state.level != if hovered { 1.0 } else { 0.0 } || state.press != if state.pressed { 1.0 } else { 0.0 };
                state.last = moving.then_some(*now);
                if moving {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if hovered => {
                state.pressed = true;
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.pressed => {
                state.pressed = false;
                shell.request_redraw();
            }
            _ if hovered != state.hovered => {
                state.hovered = hovered;
                shell.request_redraw();
            }
            _ => {}
        }
    }

    fn mouse_interaction(&self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let quad = renderer::Quad { bounds, border: border::rounded(self.radius), ..renderer::Quad::default() };
        let paint = |renderer: &mut Renderer| {
            if state.level > 0.0 && !self.shine {
                renderer.fill_quad(quad, Color { a: ui::c().control.a * state.level, ..ui::c().control });
            }
            self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
            // Фон рисуется раньше текста, поэтому осветление ложится между ними.
            if state.level > 0.0 && self.shine {
                renderer.fill_quad(quad, Color { a: SHINE * state.level, ..Color::WHITE });
            }
        };
        if state.press > 0.0 {
            let side = bounds.width.max(bounds.height).max(1.0);
            let scale = 1.0 - 2.0 * PRESS_INSET / side * anim::ease_out(state.press);
            renderer.with_transformation(anim::scale_around(bounds, 0.5, 0.5, scale), paint);
        } else {
            paint(renderer);
        }
    }

    fn overlay<'b>(&'b mut self, tree: &'b mut Tree, layout: Layout<'b>, renderer: &Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_moves_to_target_and_stops() {
        assert_eq!(step(0.0, true, FADE / 2.0), 0.5);
        assert_eq!(step(0.9, true, FADE), 1.0);
        assert_eq!(step(0.1, false, FADE), 0.0);
        assert_eq!(step(0.0, false, 1.0), 0.0);
    }
}
