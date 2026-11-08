//! Скорость воспроизведения без изменения высоты голоса (WSOLA). Звук режется на окна по 40 мс
//! с перекрытием в половину; при ускорении окна берутся из входа чаще, чем выходят, и каждое
//! сдвигается в пределах нескольких миллисекунд туда, где оно лучше всего ложится на предыдущее.
//! На скорости 1 звук проходит как есть.
//!
//! Если тон должен меняться вместе со скоростью (найткор и слоуд, как у Tune Client), звук не
//! растягивается, а пересчитывается на другую частоту: быстрее — выше, медленнее — ниже.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Половина окна и шаг на выходе.
const HOP_SECS: f64 = 0.02;
/// Насколько окно может сдвинуться в поисках лучшего совпадения.
const SEEK_SECS: f64 = 0.012;
/// Сравниваются не все отсчёты, а каждый N-й: на слух не отличить, а считать вчетверо меньше.
const STRIDE: usize = 4;

/// Скорость, общая для плеера и играющего трека: меняется на ходу. Вторая половина —
/// меняется ли с ней тон.
#[derive(Clone)]
pub struct Speed(Arc<AtomicU32>, Arc<AtomicBool>);

impl Default for Speed {
    fn default() -> Self {
        Self(Arc::new(AtomicU32::new(1.0f32.to_bits())), Arc::new(AtomicBool::new(false)))
    }
}

impl Speed {
    pub fn set(&self, speed: f32) {
        self.0.store(speed.clamp(0.5, 3.0).to_bits(), Ordering::Release);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Acquire))
    }

    pub fn set_pitch(&self, pitch: bool) {
        self.1.store(pitch, Ordering::Release);
    }

    pub fn pitch(&self) -> bool {
        self.1.load(Ordering::Acquire)
    }
}

/// Сколько кадров пересчитывается за раз в режиме с тоном.
const RESAMPLE_FRAMES: usize = 512;

/// Место в треке: при ускорении на выходе меньше отсчётов, чем прошло трека, поэтому счётчик
/// вывода врёт, а это место — нет.
#[derive(Clone, Default)]
pub struct Clock(Arc<AtomicU64>);

impl Clock {
    pub fn get(&self) -> Duration {
        Duration::from_micros(self.0.load(Ordering::Acquire))
    }

    fn set(&self, seconds: f64) {
        self.0.store((seconds.max(0.0) * 1e6) as u64, Ordering::Release);
    }
}

enum Mode {
    /// Как есть, без задержки.
    Direct,
    /// Растяжение: `prev` — начало последнего взятого окна, `nominal` — где вход должен быть
    /// без сдвигов.
    Stretch { prev: u64, nominal: f64 },
    /// Скорость вернулась к 1: дотекает то, что уже лежит в буфере, начиная с кадра `next`.
    Drain { next: u64 },
    /// Тон вместе со скоростью: `pos` — дробный номер кадра входа, который звучит сейчас.
    Resample { pos: f64 },
}

pub struct Stretched<S> {
    source: S,
    speed: Speed,
    clock: Clock,
    mode: Mode,
    channels: usize,
    rate: f64,
    hop: usize,
    reach: usize,
    /// Входные отсчёты вперемешку по каналам и их сумма по каналам для сравнения окон.
    buf: Vec<f32>,
    mono: Vec<f32>,
    /// Номер первого кадра в буфере от места последней перемотки.
    buf_start: u64,
    /// Сколько кадров взято из источника от места последней перемотки.
    taken: u64,
    /// Откуда считается время: место последней перемотки.
    base: f64,
    /// Готовый выход и позиция в нём.
    out: Vec<f32>,
    out_at: usize,
    /// Отсчёты прямого режима: канал внутри кадра.
    channel: usize,
    ended: bool,
}

impl<S: Source> Stretched<S> {
    pub fn new(source: S, speed: Speed, clock: Clock) -> Self {
        let channels = source.channels().get() as usize;
        let rate = source.sample_rate().get() as f64;
        clock.set(0.0);
        Self {
            source,
            speed,
            clock,
            mode: Mode::Direct,
            channels,
            rate,
            hop: (rate * HOP_SECS) as usize,
            reach: (rate * SEEK_SECS) as usize,
            buf: Vec::new(),
            mono: Vec::new(),
            buf_start: 0,
            taken: 0,
            base: 0.0,
            out: Vec::new(),
            out_at: 0,
            channel: 0,
            ended: false,
        }
    }

    fn tick(&self, frame: f64) {
        self.clock.set(self.base + frame / self.rate);
    }

    /// Дочитывает вход, пока в буфере нет кадра `until`. Ложь — трек кончился раньше.
    fn fill(&mut self, until: u64) -> bool {
        while self.buf_start + self.mono.len() as u64 <= until {
            let mut sum = 0.0;
            for _ in 0..self.channels {
                let Some(sample) = self.source.next() else {
                    // Неполный кадр выбрасывается, чтобы каналы не перепутались.
                    self.buf.truncate(self.mono.len() * self.channels);
                    return false;
                };
                self.buf.push(sample);
                sum += sample;
            }
            self.mono.push(sum);
            self.taken += 1;
        }
        true
    }

    /// Кадры до `frame` больше не понадобятся.
    fn forget(&mut self, frame: u64) {
        let drop = frame.saturating_sub(self.buf_start).min(self.mono.len() as u64) as usize;
        if drop > 0 {
            self.mono.drain(..drop);
            self.buf.drain(..drop * self.channels);
            self.buf_start += drop as u64;
        }
    }

    fn at(&self, frame: u64) -> usize {
        (frame - self.buf_start) as usize
    }

    /// Начало окна около `nominal`, которое лучше всего продолжает `natural`.
    fn best(&self, natural: u64, nominal: u64) -> u64 {
        let from = nominal.saturating_sub(self.reach as u64).max(self.buf_start);
        let last = self.buf_start + self.mono.len() as u64;
        // Окно должно целиком лежать в буфере: и его начало, и продолжение для следующего шага.
        let to = (nominal + self.reach as u64).min(last.saturating_sub(2 * self.hop as u64));
        if to < from {
            return natural;
        }
        let target = &self.mono[self.at(natural)..self.at(natural) + self.hop];
        let mut best = (f32::MIN, natural);
        for start in from..=to {
            let candidate = &self.mono[self.at(start)..self.at(start) + self.hop];
            let (mut cross, mut energy) = (0.0f32, 1e-9f32);
            for i in (0..self.hop).step_by(STRIDE) {
                cross += target[i] * candidate[i];
                energy += candidate[i] * candidate[i];
            }
            let score = cross / energy.sqrt();
            if score > best.0 {
                best = (score, start);
            }
        }
        best.1
    }

    /// Следующая порция выхода. Ложь — выхода больше не будет.
    fn produce(&mut self) -> bool {
        self.out.clear();
        self.out_at = 0;
        let speed = self.speed.get() as f64;
        let pitch = self.speed.pitch();
        let hop = self.hop as u64;
        match self.mode {
            Mode::Direct => unreachable!(),
            Mode::Resample { pos } => {
                // Скорость вернулась к 1 или тон больше не нужен: дотекает буфер, дальше
                // снова напрямую или растяжением.
                if speed == 1.0 || !pitch {
                    // Место может быть дальше прочитанного: сперва дочитываем до него.
                    let next = pos.ceil() as u64;
                    if !self.fill(next) {
                        self.ended = true;
                    }
                    self.mode = Mode::Drain { next };
                    return true;
                }
                let mut pos = pos;
                for _ in 0..RESAMPLE_FRAMES {
                    let frame = pos.floor() as u64;
                    if !self.fill(frame + 1) {
                        self.ended = true;
                        break;
                    }
                    let part = (pos - frame as f64) as f32;
                    let (a, b) = (self.at(frame) * self.channels, self.at(frame + 1) * self.channels);
                    for c in 0..self.channels {
                        self.out.push(self.buf[a + c] * (1.0 - part) + self.buf[b + c] * part);
                    }
                    pos += speed;
                }
                self.tick(pos);
                self.forget(pos.floor() as u64);
                if self.ended {
                    self.forget(u64::MAX);
                    self.mode = Mode::Direct;
                    return !self.out.is_empty();
                }
                self.mode = Mode::Resample { pos };
                return true;
            }
            Mode::Drain { next } => {
                // Дотекает буфер, дальше снова напрямую.
                let at = self.at(next.max(self.buf_start)).min(self.mono.len());
                self.out.extend_from_slice(&self.buf[at * self.channels..]);
                self.tick(next as f64);
                self.forget(u64::MAX);
                self.mode = Mode::Direct;
                return !self.out.is_empty() || !self.ended;
            }
            Mode::Stretch { prev, nominal } => {
                // Включили тон: растяжение доигрывает окно, как при возврате к скорости 1.
                let speed = if pitch { 1.0 } else { speed };
                let natural = prev + hop;
                let nominal = if speed == 1.0 { natural as f64 } else { nominal + hop as f64 * speed };
                let around = nominal.round() as u64;
                if !self.fill(around + self.reach as u64 + 2 * hop) {
                    self.ended = true;
                }
                if !self.ended || self.buf_start + (self.mono.len() as u64) >= natural + 2 * hop.max(1) {
                    let next = if speed == 1.0 { natural } else { self.best(natural, around) };
                    let window = 2 * self.hop;
                    for i in 0..self.hop {
                        // Окно Ханна: две половины в сумме дают ровно единицу.
                        let rise = (std::f64::consts::PI * i as f64 / window as f64).sin().powi(2) as f32;
                        let old = self.at(natural) + i;
                        let new = self.at(next) + i;
                        for c in 0..self.channels {
                            self.out.push(self.buf[old * self.channels + c] * (1.0 - rise) + self.buf[new * self.channels + c] * rise);
                        }
                    }
                    self.tick(nominal);
                    self.forget(next.min((nominal as u64).saturating_sub(self.reach as u64)));
                    self.mode = if speed == 1.0 { Mode::Drain { next: next + hop } } else { Mode::Stretch { prev: next, nominal } };
                    return true;
                }
                // Конец трека: остаток как есть.
                let at = self.at(natural.max(self.buf_start).min(self.buf_start + self.mono.len() as u64));
                self.out.extend_from_slice(&self.buf[at * self.channels..]);
                self.forget(u64::MAX);
                self.mode = Mode::Direct;
                !self.out.is_empty()
            }
        }
    }
}

impl<S: Source> Iterator for Stretched<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        loop {
            if self.out_at < self.out.len() {
                self.out_at += 1;
                return Some(self.out[self.out_at - 1]);
            }
            if matches!(self.mode, Mode::Direct) {
                // Скорость меняется только на границе кадра, чтобы каналы не съехали.
                if self.channel == 0 && self.speed.get() != 1.0 && !self.ended && self.speed.pitch() {
                    self.buf_start = self.taken;
                    self.mode = Mode::Resample { pos: self.taken as f64 };
                    continue;
                }
                if self.channel == 0 && self.speed.get() != 1.0 && !self.ended {
                    let start = self.taken;
                    self.buf_start = start;
                    if !self.fill(start + 2 * self.hop as u64) {
                        self.ended = true;
                        self.out.clear();
                        self.out.extend_from_slice(&self.buf);
                        self.out_at = 0;
                        self.forget(u64::MAX);
                        continue;
                    }
                    // Первая половина окна идёт как есть, дальше окна накладываются.
                    self.out.clear();
                    self.out.extend_from_slice(&self.buf[..self.hop * self.channels]);
                    self.out_at = 0;
                    self.mode = Mode::Stretch { prev: start, nominal: start as f64 };
                    continue;
                }
                let sample = self.source.next()?;
                self.channel += 1;
                if self.channel == self.channels {
                    self.channel = 0;
                    self.taken += 1;
                    if self.taken % 512 == 0 {
                        self.tick(self.taken as f64);
                    }
                }
                return Some(sample);
            }
            if !self.produce() {
                return None;
            }
        }
    }
}

/// Длина куска в кадрах, о которой сообщаем очереди rodio.
const SPAN_FRAMES: usize = 1024;

impl<S: Source> Source for Stretched<S> {
    /// Частота трека не меняется, но очередь rodio перечитывает её только на границе куска.
    /// Без границ она так и осталась бы от тишины перед треком (44,1 кГц), и трек в 48 кГц
    /// звучал бы замедленным. Поэтому куски короткие: готовый выход или до границы блока.
    fn current_span_len(&self) -> Option<usize> {
        let pending = self.out.len() - self.out_at;
        Some(if pending > 0 { pending } else { SPAN_FRAMES * self.channels - self.channel })
    }

    fn channels(&self) -> ChannelCount {
        self.source.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.source.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        self.source.try_seek(position)?;
        self.buf.clear();
        self.mono.clear();
        self.out.clear();
        self.out_at = 0;
        self.buf_start = 0;
        self.taken = 0;
        self.channel = 0;
        self.ended = false;
        self.mode = Mode::Direct;
        self.base = position.as_secs_f64();
        self.tick(0.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    fn tone(seconds: f32) -> SamplesBuffer {
        let rate = 8000;
        let samples: Vec<f32> = (0..(rate as f32 * seconds) as usize).flat_map(|n| {
            let x = (std::f32::consts::TAU * 200.0 * n as f32 / rate as f32).sin() * 0.5;
            [x, -x]
        }).collect();
        SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(rate).unwrap(), samples)
    }

    #[test]
    fn same_speed_passes_through() {
        let original: Vec<f32> = tone(1.0).collect();
        let speed = Speed::default();
        let out: Vec<f32> = Stretched::new(tone(1.0), speed, Clock::default()).collect();
        assert_eq!(out, original);
    }

    #[test]
    fn double_speed_halves_length_and_keeps_clock() {
        let speed = Speed::default();
        speed.set(2.0);
        let clock = Clock::default();
        let out: Vec<f32> = Stretched::new(tone(2.0), speed, clock.clone()).collect();
        let seconds = out.len() as f32 / 2.0 / 8000.0;
        assert!((seconds - 1.0).abs() < 0.08, "{seconds}");
        assert!(clock.get().as_secs_f32() > 1.85, "{:?}", clock.get());
        // Каналы не перепутались: правый всё ещё зеркало левого.
        assert!(out.chunks(2).all(|frame| (frame[0] + frame[1]).abs() < 1e-4));
        // Высота та же: у синуса 200 Гц по-прежнему 400 переходов через ноль в секунду.
        let left: Vec<f32> = out.chunks(2).map(|frame| frame[0]).collect();
        let crossings = left.windows(2).filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0)).count() as f32 / seconds;
        assert!((crossings - 400.0).abs() < 20.0, "{crossings}");
    }

    #[test]
    fn pitch_mode_raises_tone_with_speed() {
        let speed = Speed::default();
        speed.set(2.0);
        speed.set_pitch(true);
        let clock = Clock::default();
        let out: Vec<f32> = Stretched::new(tone(2.0), speed, clock.clone()).collect();
        let seconds = out.len() as f32 / 2.0 / 8000.0;
        assert!((seconds - 1.0).abs() < 0.02, "{seconds}");
        assert!(clock.get().as_secs_f32() > 1.95, "{:?}", clock.get());
        assert!(out.chunks(2).all(|frame| (frame[0] + frame[1]).abs() < 1e-4));
        // Тон вдвое выше: синус 200 Гц звучит как 400 Гц — 800 переходов через ноль в секунду.
        let left: Vec<f32> = out.chunks(2).map(|frame| frame[0]).collect();
        let crossings = left.windows(2).filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0)).count() as f32 / seconds;
        assert!((crossings - 800.0).abs() < 30.0, "{crossings}");
    }

    #[test]
    fn pitch_back_to_normal_speed_midway() {
        // Так было с правым кликом по кнопке скорости: тон включён, скорость сбрасывается в 1×.
        for start in [2.0, 1.75, 0.5, 0.55] {
            let speed = Speed::default();
            speed.set(start);
            speed.set_pitch(true);
            let mut stretched = Stretched::new(tone(2.0), speed.clone(), Clock::default());
            let mut count = 0;
            for (n, _) in stretched.by_ref().enumerate() {
                count += 1;
                if n == 6001 {
                    speed.set(1.0);
                }
            }
            assert!(count > 6001, "{start}");
        }
    }

    #[test]
    fn pitch_switches_midway_without_losing_samples() {
        let speed = Speed::default();
        speed.set(0.5);
        let mut stretched = Stretched::new(tone(2.0), speed.clone(), Clock::default());
        let mut count = 0;
        for (n, _) in stretched.by_ref().enumerate() {
            count += 1;
            if n == 8000 {
                speed.set_pitch(true);
            }
            if n == 24000 {
                speed.set_pitch(false);
            }
        }
        // Вдвое медленнее и так и эдак: около четырёх секунд.
        let seconds = count as f32 / 2.0 / 8000.0;
        assert!(seconds > 3.7 && seconds < 4.3, "{seconds}");
    }

    #[test]
    fn speed_changes_midway_without_losing_samples() {
        let speed = Speed::default();
        let mut stretched = Stretched::new(tone(3.0), speed.clone(), Clock::default());
        let mut count = 0;
        for (n, _) in stretched.by_ref().enumerate() {
            count += 1;
            if n == 8000 {
                speed.set(1.5);
            }
            if n == 24000 {
                speed.set(1.0);
            }
        }
        // 0,5 с как есть, потом ускорение, потом снова как есть: всего меньше трёх секунд.
        let seconds = count as f32 / 2.0 / 8000.0;
        assert!(seconds > 2.4 && seconds < 2.9, "{seconds}");
    }
}
