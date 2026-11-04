//! Колесо волны: подсказки стоят по кольцу, которое уходит вглубь экрана. Ближняя — внизу
//! посередине и крупнее всех, дальние мельче, тусклее и выше, как на карусели, на которую
//! смотрят чуть сверху. Под кольцом нарисован его след на «полу» и отсвет ближней подсказки.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme};

use crate::{ui, Message};

/// Высота всего колеса вместе с подписью ближней подсказки.
pub const AREA: f32 = 330.0;
/// Сторона ближней и самой дальней обложки.
pub const FRONT: f32 = 104.0;
const BACK: f32 = 34.0;
/// Полуоси кольца: ширина зависит от окна, высота — наклон взгляда.
const RADIUS_X_MAX: f32 = 520.0;
const RADIUS_Y: f32 = 78.0;
/// Шаг между подсказками по кругу: видно около одиннадцати, остальные спрятаны за кольцом.
const STEP: f32 = 0.56;
/// Где на высоте колеса середина кольца.
const CENTER_Y: f32 = 112.0;
/// Сколько секунд колесо доворачивается до цели (постоянная времени).
pub const SPEED: f32 = 9.0;

/// Ширина кольца по окну.
pub fn radius_x(width: f32) -> f32 {
    (width * 0.36).min(RADIUS_X_MAX)
}

/// Место подсказки на кольце: смещение от ближней в шагах, от −n/2 до n/2.
pub fn offset(index: usize, turn: f32, count: usize) -> f32 {
    let count = count.max(1) as f32;
    let step = (index as f32 - turn).rem_euclid(count);
    if step > count / 2.0 { step - count } else { step }
}

/// Где на экране подсказка и насколько она близко: `depth` 1 — ближняя, 0 — самая дальняя.
pub struct Spot {
    pub x: f32,
    pub y: f32,
    pub depth: f32,
    pub side: f32,
    /// Насколько видна: у самого дальнего края подсказка растворяется, а за ним спрятана.
    pub shown: f32,
}

pub fn spot(offset: f32, width: f32) -> Spot {
    let angle = offset * STEP;
    let depth = (angle.cos() + 1.0) / 2.0;
    // Вдали кольцо сжимается сильнее: так глаз читает глубину.
    let side = BACK + (FRONT - BACK) * depth.powf(3.0);
    let edge = std::f32::consts::PI - angle.abs();
    let shown = (edge / 0.6).clamp(0.0, 1.0);
    Spot { x: width / 2.0 + radius_x(width) * angle.sin(), y: CENTER_Y + RADIUS_Y * angle.cos(), depth, side, shown }
}

/// Нижний край ближней обложки: под ним подпись.
pub fn front_bottom() -> f32 {
    CENTER_Y + RADIUS_Y + FRONT / 2.0
}

/// След кольца на «полу» и отсвет под ближней подсказкой её цветом.
pub struct Floor {
    pub glow: Color,
    /// Насколько ближняя подсказка встала на место: отсвет ярче, когда колесо стоит.
    pub settled: f32,
}

impl canvas::Program<Message> for Floor {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = bounds.width;
        let (rx, ry) = (radius_x(width), RADIUS_Y);
        let center = Point::new(width / 2.0, CENTER_Y + FRONT * 0.42);
        // Отсвет: несколько вложенных овалов, к краю всё прозрачнее — мягкое пятно света.
        for ring in (1..=10).rev() {
            let part = ring as f32 / 10.0;
            let glow = Color { a: 0.035 * self.settled * (1.0 - part).powf(0.6) + 0.004, ..self.glow };
            let path = Path::new(|p| {
                p.ellipse(canvas::path::arc::Elliptical {
                    center: Point::new(center.x, center.y + ry),
                    radii: iced::Vector::new(FRONT * 1.9 * part, FRONT * 0.42 * part),
                    rotation: iced::Radians(0.0),
                    start_angle: iced::Radians(0.0),
                    end_angle: iced::Radians(std::f32::consts::TAU),
                })
            });
            frame.fill(&path, glow);
        }
        // След кольца: ближняя половина ярче, дальняя почти растворяется.
        let steps = 96;
        let point = |i: usize| {
            let angle = i as f32 / steps as f32 * std::f32::consts::TAU;
            (Point::new(center.x + rx * angle.sin(), center.y + ry * angle.cos()), (angle.cos() + 1.0) / 2.0)
        };
        for i in 0..steps {
            let ((from, depth), (to, _)) = (point(i), point(i + 1));
            let color = Color { a: 0.03 + 0.16 * depth.powf(2.0), ..ui::c().text };
            frame.stroke(&Path::line(from, to), Stroke::default().with_width(1.0 + depth).with_color(color));
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps_and_recedes() {
        assert_eq!(offset(0, 0.0, 10), 0.0);
        assert_eq!(offset(9, 0.0, 10), -1.0);
        assert_eq!(offset(2, 9.0, 10), 3.0);
        let front = spot(0.0, 1000.0);
        let back = spot(std::f32::consts::PI / STEP, 1000.0);
        assert!((front.depth - 1.0).abs() < 1e-5 && back.depth < 1e-5 && back.shown == 0.0);
        assert!(front.y > back.y && front.side > back.side);
        assert!((front.x - 500.0).abs() < 1e-3);
    }
}
