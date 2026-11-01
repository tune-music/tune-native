//! Полоска перемотки: при наведении плавно становится толще и показывает бегунок.

use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path};
use iced::{window, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::{ui, Message};

const THIN: f32 = 3.0;
const THICK: f32 = 5.0;
const KNOB: f32 = 6.0;
/// За сколько секунд полоска откликается на наведение.
const FADE: f32 = 0.12;

pub struct Seek {
    pub value: f32,
    pub total: f32,
}

#[derive(Default)]
pub struct State {
    dragging: bool,
    hovered: bool,
    level: f32,
    last: Option<Instant>,
}

fn value_at(cursor: Point, bounds: Rectangle, total: f32) -> f32 {
    ((cursor.x - bounds.x) / bounds.width).clamp(0.0, 1.0) * total
}

impl canvas::Program<Message> for Seek {
    type State = State;

    fn update(&self, state: &mut State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        let hovered = cursor.is_over(bounds);
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds).map(|_| cursor.position())??;
                state.dragging = true;
                Some(Action::publish(Message::Seek(value_at(position, bounds, self.total))).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => Some(Action::publish(Message::Seek(value_at(*position, bounds, self.total)))),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                Some(Action::publish(Message::SeekDone))
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                let elapsed = state.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
                let target = if hovered || state.dragging { 1.0 } else { 0.0 };
                state.hovered = hovered;
                state.level = if target > state.level { (state.level + elapsed / FADE).min(1.0) } else { (state.level - elapsed / FADE).max(0.0) };
                state.last = (state.level != target).then_some(*now);
                (state.level != target).then(Action::request_redraw)
            }
            _ if hovered != state.hovered => {
                state.hovered = hovered;
                Some(Action::request_redraw())
            }
            _ => None,
        }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let thick = THIN + (THICK - THIN) * state.level;
        let top = (bounds.height - thick) / 2.0;
        let bar = |frame: &mut Frame, width: f32, color: Color| {
            frame.fill(&Path::rounded_rectangle(Point::new(0.0, top), Size::new(width, thick), (thick / 2.0).into()), color);
        };
        bar(&mut frame, bounds.width, ui::CONTROL_PRESSED);
        let to = (self.value / self.total.max(0.001)).clamp(0.0, 1.0) * bounds.width;
        bar(&mut frame, to, if state.level > 0.0 { ui::ACCENT } else { ui::WHITE });
        if state.level > 0.0 {
            let center = Point::new(to.clamp(KNOB, (bounds.width - KNOB).max(KNOB)), bounds.height / 2.0);
            frame.fill(&Path::circle(center, KNOB * state.level), ui::WHITE);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.dragging || cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_follows_cursor() {
        let bounds = Rectangle::new(Point::new(100.0, 0.0), Size::new(200.0, 50.0));
        assert_eq!(value_at(Point::new(200.0, 10.0), bounds, 60.0), 30.0);
        assert_eq!(value_at(Point::new(0.0, 10.0), bounds, 60.0), 0.0);
    }
}
