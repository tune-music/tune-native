//! Колесо волны: подсказки стоят в ряд, ближняя посередине крупнее всех, а к краям они
//! мельче, размытее и темнее, как на полосе, которая уходит в стороны. Колесо спрятано за
//! экраном волны и выезжает снизу, когда страница волны улетает вверх.

/// Сторона ближней обложки.
pub const FRONT: f32 = 156.0;
/// Самые дальние обложки — такая часть ближней.
const FAR: f32 = 0.42;
/// Насколько быстро к краям обложки уменьшаются и сжимаются к краю полосы.
const SHRINK: f32 = 0.55;
const SPREAD: f32 = 0.42;
/// Дальше этого шага подсказки растворяются и прячутся.
const VISIBLE: f32 = 3.6;
/// Как быстро колесо доворачивается до цели.
pub const SPEED: f32 = 9.0;
/// За сколько секунд колесо раскрывается и прячется.
pub const REVEAL: f32 = 0.3;

/// Место подсказки в ряду: смещение от ближней в шагах, от −n/2 до n/2.
pub fn offset(index: usize, turn: f32, count: usize) -> f32 {
    let count = count.max(1) as f32;
    let step = (index as f32 - turn).rem_euclid(count);
    if step > count / 2.0 { step - count } else { step }
}

/// Где подсказка и как она выглядит: размер, размытие и затемнение от 0 до 1, видимость.
pub struct Spot {
    pub x: f32,
    pub side: f32,
    pub blur: f32,
    pub dark: f32,
    pub shown: f32,
    /// Близость к середине: ближние рисуются поверх дальних.
    pub near: f32,
}

pub fn spot(offset: f32, width: f32) -> Spot {
    let far = offset.abs();
    let reach = (width / 2.0 - FRONT * FAR / 2.0 - 70.0).max(FRONT);
    let near = (-SHRINK * far * far).exp();
    Spot {
        x: width / 2.0 + reach * (offset * SPREAD).tanh(),
        side: FRONT * (FAR + (1.0 - FAR) * near),
        blur: (far / 1.4).min(1.0),
        dark: (far * 0.2).min(0.62),
        shown: ((VISIBLE - far) / 0.8).clamp(0.0, 1.0),
        near,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_shrinks_and_dims_to_sides() {
        assert_eq!(offset(0, 0.0, 10), 0.0);
        assert_eq!(offset(9, 0.0, 10), -1.0);
        assert_eq!(offset(2, 9.0, 10), 3.0);
        let front = spot(0.0, 1200.0);
        let side = spot(2.0, 1200.0);
        let left = spot(-2.0, 1200.0);
        assert!((front.x - 600.0).abs() < 1e-3 && front.side == FRONT && front.blur == 0.0 && front.dark == 0.0);
        assert!(side.side < front.side && side.blur > 0.9 && side.dark > 0.3 && side.x > 600.0);
        assert!((left.x - (1200.0 - side.x)).abs() < 1e-3);
        assert_eq!(spot(4.5, 1200.0).shown, 0.0);
    }
}
