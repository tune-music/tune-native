//! Перемотка заливкой: пройденная часть обрезана по форме панели, как в оригинале.
//! Обычный ползунок скругляет саму заливку, и в начале трека она выглядит каплей.

use iced::mouse;
use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Path};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use crate::Message;

pub struct Seek {
    pub value: f32,
    pub total: f32,
    pub radius: f32,
    pub base: Color,
    pub fill: Color,
}

/// Отступ верхней кромки скругления на расстоянии `x` от края.
fn inset(x: f32, radius: f32) -> f32 {
    if x >= radius {
        return 0.0;
    }
    let d = radius - x;
    radius - (radius * radius - d * d).max(0.0).sqrt()
}

/// Контур части панели левее `to`.
fn fill_outline(to: f32, size: Size, radius: f32) -> Vec<Point> {
    let edge = |x: f32| inset(x, radius).max(inset(size.width - x, radius));
    let steps = (to / size.width * 64.0).ceil().max(2.0) as usize + (radius as usize);
    let xs: Vec<f32> = (0..=steps).map(|i| to * i as f32 / steps as f32).collect();
    xs.iter().map(|&x| Point::new(x, edge(x))).chain(xs.iter().rev().map(|&x| Point::new(x, size.height - edge(x)))).collect()
}

fn value_at(cursor: Point, bounds: Rectangle, total: f32) -> f32 {
    ((cursor.x - bounds.x) / bounds.width).clamp(0.0, 1.0) * total
}

impl canvas::Program<Message> for Seek {
    /// Тянут ли панель сейчас.
    type State = bool;

    fn update(&self, dragging: &mut bool, event: &Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds).map(|_| cursor.position())??;
                *dragging = true;
                Some(Action::publish(Message::Seek(value_at(position, bounds, self.total))).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if *dragging => Some(Action::publish(Message::Seek(value_at(*position, bounds, self.total)))),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if *dragging => {
                *dragging = false;
                Some(Action::publish(Message::SeekDone))
            }
            _ => None,
        }
    }

    fn draw(&self, _: &bool, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        if self.base.a > 0.0 {
            frame.fill(&Path::rounded_rectangle(Point::ORIGIN, bounds.size(), self.radius.into()), self.base);
        }
        let to = (self.value / self.total.max(0.001)).clamp(0.0, 1.0) * bounds.width;
        if to >= 1.0 {
            let outline = fill_outline(to, bounds.size(), self.radius);
            let path = Path::new(|b| {
                b.move_to(outline[0]);
                outline[1..].iter().for_each(|p| b.line_to(*p));
                b.close();
            });
            frame.fill(&path, self.fill);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, dragging: &bool, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if *dragging || cursor.is_over(bounds) {
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
    fn fill_follows_rounded_edge() {
        assert!((inset(0.0, 28.0) - 28.0).abs() < 0.01);
        assert_eq!(inset(28.0, 28.0), 0.0);
        let outline = fill_outline(10.0, Size::new(320.0, 56.0), 28.0);
        // В начале заливка уже панели и прижата к её середине, а не торчит каплей.
        assert!((outline[0].y - 28.0).abs() < 0.01);
        assert!(outline.iter().all(|p| p.x <= 10.0 && (0.0..=56.0).contains(&p.y)));
    }

    #[test]
    fn value_follows_cursor() {
        let bounds = Rectangle::new(Point::new(100.0, 0.0), Size::new(200.0, 50.0));
        assert_eq!(value_at(Point::new(200.0, 10.0), bounds, 60.0), 30.0);
        assert_eq!(value_at(Point::new(0.0, 10.0), bounds, 60.0), 0.0);
    }
}
