//! Эквалайзер как в официальном клиенте: десять полос, предусиление и те же пресеты.
//! Крайние полосы — полки, средние — пики с добротностью 1, формулы те же, что у WebAudio.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Частоты полос, Гц.
pub const BANDS: [f32; 10] = [60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0];
/// Пределы ползунков: полосы в децибелах, предусиление — множитель громкости.
pub const GAIN_LIMIT: f32 = 12.0;
pub const PREAMP_MIN: f32 = 0.5;
pub const PREAMP_MAX: f32 = 1.5;
/// Первый пресет — своя настройка: в него переходят, когда двигают полосы руками.
pub const CUSTOM: usize = 0;

/// Название и усиление полос. Порядок и числа из официального клиента.
pub const PRESETS: [(&str, [f32; 10]); 19] = [
    ("Своя настройка", [0.0; 10]),
    ("По умолчанию", [0.0; 10]),
    ("Классическая музыка", [-0.5, -0.5, -0.5, -0.5, -0.5, -0.5, -3.5, -3.5, -3.5, -4.5]),
    ("Клубная музыка", [-0.5, -0.5, 4.0, 2.5, 2.5, 2.5, 1.5, -0.5, -0.5, -0.5]),
    ("Танцевальная музыка", [4.5, 3.5, 1.0, -0.5, -0.5, -2.5, -3.5, -3.5, -0.5, -0.5]),
    ("Усиление НЧ", [4.0, 4.5, 4.5, 2.5, 0.5, -2.0, -4.0, -5.0, -5.5, -5.5]),
    ("Усиление НЧ и ВЧ", [3.5, 2.5, -0.5, -3.5, -2.0, 0.5, 4.0, 5.5, 6.0, 6.0]),
    ("Усиление ВЧ", [-4.5, -4.5, -4.5, -2.0, 1.0, 5.5, 8.0, 8.0, 8.0, 8.0]),
    ("Колонки ноутбука", [2.0, 5.5, 2.5, -1.5, -1.0, 0.5, 2.0, 4.5, 6.0, 7.0]),
    ("Большой зал", [5.0, 5.0, 2.5, 2.5, -0.5, -2.0, -2.0, -2.0, -0.5, -0.5]),
    ("Концерт", [-2.0, -0.5, 2.0, 2.5, 2.5, 2.5, 2.0, 1.0, 1.0, 1.0]),
    ("Вечеринка", [3.5, 3.5, -0.5, -0.5, -0.5, -0.5, -0.5, -0.5, 3.5, 3.5]),
    ("Поп", [-0.5, 2.0, 3.5, 4.0, 2.5, -0.5, -1.0, -1.0, -0.5, -0.5]),
    ("Регги", [-0.5, -0.5, -0.5, -2.5, -0.5, 3.0, 3.0, -0.5, -0.5, -0.5]),
    ("Рок", [4.0, 2.0, -2.5, -4.0, -1.5, 2.0, 4.0, 5.5, 5.5, 5.5]),
    ("Ска", [-1.0, -2.0, -2.0, -0.5, 2.0, 2.5, 4.0, 4.5, 5.5, 4.5]),
    ("Мягкое звучание", [2.0, 0.5, -0.5, -1.0, -0.5, 2.0, 4.0, 4.5, 5.5, 6.0]),
    ("Софт-рок", [2.0, 2.0, 1.0, -0.5, -2.0, -2.5, -1.5, -0.5, 1.0, 4.0]),
    ("Техно", [4.0, 2.5, -0.5, -2.5, -2.0, -0.5, 4.0, 4.5, 4.5, 4.0]),
];

/// Что выбрано в эквалайзере. Хранится в настройках приложения.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    pub preset: usize,
    pub preamp: f32,
    pub gains: [f32; 10],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self { enabled: false, preset: 1, preamp: 1.0, gains: [0.0; 10] }
    }
}

impl EqSettings {
    /// Ничего не меняет в звуке: фильтры можно не считать.
    fn is_flat(&self) -> bool {
        !self.enabled || (self.preamp == 1.0 && self.gains.iter().all(|gain| *gain == 0.0))
    }
}

/// Настройки, общие для плеера и звукового потока: ползунок меняет звук сразу.
#[derive(Clone, Default)]
pub struct Shared {
    settings: Arc<Mutex<EqSettings>>,
    version: Arc<AtomicU64>,
}

impl Shared {
    pub fn set(&self, settings: EqSettings) {
        *self.settings.lock().unwrap() = settings;
        self.version.fetch_add(1, Ordering::Release);
    }
}

/// Биквадратный фильтр в транспонированной второй форме.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy)]
enum Kind {
    LowShelf,
    Peaking,
    HighShelf,
}

impl Biquad {
    fn new(kind: Kind, frequency: f32, gain: f32, rate: f32) -> Self {
        // Выше половины частоты дискретизации полосы нет: фильтр пропускает звук как есть.
        if gain == 0.0 || frequency >= rate * 0.49 {
            return Self { b0: 1.0, ..Self::default() };
        }
        let a = 10f32.powf(gain / 40.0);
        let w = std::f32::consts::TAU * frequency / rate;
        let (sin, cos) = w.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match kind {
            Kind::Peaking => {
                let alpha = sin / 2.0;
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
            Kind::LowShelf | Kind::HighShelf => {
                // Наклон полки 1, как у WebAudio.
                let k = 2.0 * a.sqrt() * sin / 2.0 * std::f32::consts::SQRT_2;
                let sign = if matches!(kind, Kind::LowShelf) { 1.0 } else { -1.0 };
                let b0 = a * ((a + 1.0) - sign * (a - 1.0) * cos + k);
                let b1 = sign * 2.0 * a * ((a - 1.0) - sign * (a + 1.0) * cos);
                let b2 = a * ((a + 1.0) - sign * (a - 1.0) * cos - k);
                let a0 = (a + 1.0) + sign * (a - 1.0) * cos + k;
                let a1 = -sign * 2.0 * ((a - 1.0) + sign * (a + 1.0) * cos);
                let a2 = (a + 1.0) + sign * (a - 1.0) * cos - k;
                (b0, b1, b2, a0, a1, a2)
            }
        };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn run(&self, state: &mut [f32; 2], x: f32) -> f32 {
        let y = self.b0 * x + state[0];
        state[0] = self.b1 * x - self.a1 * y + state[1];
        state[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Звуковой поток, пропущенный через эквалайзер.
pub struct Equalized<S> {
    source: S,
    shared: Shared,
    version: u64,
    settings: EqSettings,
    filters: [Biquad; 10],
    /// Память фильтров для каждого канала.
    states: Vec<[[f32; 2]; 10]>,
    channel: usize,
    format: (u16, u32),
}

impl<S: Source> Equalized<S> {
    pub fn new(source: S, shared: Shared) -> Self {
        let mut equalized = Self { source, shared, version: u64::MAX, settings: EqSettings::default(), filters: [Biquad::default(); 10], states: Vec::new(), channel: 0, format: (0, 0) };
        equalized.refresh();
        equalized
    }

    /// Пересчитывает фильтры, если сменились настройки или формат звука.
    fn refresh(&mut self) {
        let version = self.shared.version.load(Ordering::Acquire);
        let format = (self.source.channels().get(), self.source.sample_rate().get());
        if version == self.version && format == self.format {
            return;
        }
        if format != self.format {
            self.states = vec![[[0.0; 2]; 10]; format.0 as usize];
            self.format = format;
        }
        self.version = version;
        self.settings = *self.shared.settings.lock().unwrap();
        let last = BANDS.len() - 1;
        self.filters = std::array::from_fn(|band| {
            let kind = match band {
                0 => Kind::LowShelf,
                band if band == last => Kind::HighShelf,
                _ => Kind::Peaking,
            };
            Biquad::new(kind, BANDS[band], self.settings.gains[band], format.1 as f32)
        });
    }
}

impl<S: Source> Iterator for Equalized<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        // Настройки проверяются в начале каждого кадра, чтобы каналы не разъезжались.
        if self.channel == 0 {
            self.refresh();
        }
        let sample = self.source.next()?;
        let channel = self.channel;
        self.channel = (self.channel + 1) % self.states.len().max(1);
        if self.settings.is_flat() {
            return Some(sample);
        }
        let state = &mut self.states[channel];
        let mut value = sample * self.settings.preamp;
        for (filter, memory) in self.filters.iter().zip(state.iter_mut()) {
            value = filter.run(memory, value);
        }
        Some(value)
    }
}

impl<S: Source> Source for Equalized<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
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
        // После перемотки старая память фильтров дала бы щелчок.
        for state in &mut self.states {
            *state = [[0.0; 2]; 10];
        }
        self.channel = 0;
        self.source.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Размах синуса после фильтра, когда он уже установился.
    fn swing(gain: f32, frequency: f32) -> f32 {
        let rate = 44100.0;
        let filter = Biquad::new(Kind::Peaking, 1000.0, gain, rate);
        let mut state = [0.0; 2];
        let mut peak: f32 = 0.0;
        for n in 0..20000 {
            let x = (std::f32::consts::TAU * frequency * n as f32 / rate).sin();
            let y = filter.run(&mut state, x);
            if n > 10000 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn peaking_band_boosts_its_frequency() {
        // +6 дБ — это вдвое по размаху.
        assert!((swing(6.0, 1000.0) - 2.0).abs() < 0.05);
        assert!((swing(0.0, 1000.0) - 1.0).abs() < 0.01);
        // Далеко от полосы звук почти не меняется.
        assert!((swing(6.0, 100.0) - 1.0).abs() < 0.1);
    }

    #[test]
    fn shelves_lift_their_side() {
        let rate = 44100.0;
        let response = |filter: Biquad, frequency: f32| {
            let mut state = [0.0; 2];
            let mut peak: f32 = 0.0;
            for n in 0..40000 {
                let y = filter.run(&mut state, (std::f32::consts::TAU * frequency * n as f32 / rate).sin());
                if n > 30000 {
                    peak = peak.max(y.abs());
                }
            }
            peak
        };
        let low = Biquad::new(Kind::LowShelf, 60.0, 6.0, rate);
        assert!(response(low, 20.0) > 1.4 && response(low, 5000.0) < 1.05);
        let high = Biquad::new(Kind::HighShelf, 16000.0, 6.0, rate);
        assert!(response(high, 20000.0) > 1.4 && response(high, 500.0) < 1.05);
    }

    /// Настоящий поток rodio: выключенный эквалайзер не трогает звук, включённый меняет сразу.
    #[test]
    fn source_follows_settings() {
        let shared = Shared::default();
        let sine = rodio::source::SineWave::new(1000.0).take_duration(Duration::from_millis(500));
        let mut source = Equalized::new(sine, shared.clone());
        let plain: Vec<f32> = source.by_ref().take(4410).collect();
        let reference: Vec<f32> = rodio::source::SineWave::new(1000.0).take(4410).collect();
        assert_eq!(plain, reference);
        let mut boost = PRESETS[1].1;
        boost[4] = 6.0;
        shared.set(EqSettings { enabled: true, preset: CUSTOM, preamp: 1.0, gains: boost });
        let loud = source.skip(4410).take(4410).fold(0f32, |peak, x| peak.max(x.abs()));
        assert!((loud - 2.0).abs() < 0.1, "{loud}");
    }

    #[test]
    fn presets_match_original() {
        assert_eq!(PRESETS.len(), 19);
        assert_eq!(PRESETS[14].0, "Рок");
        assert!(PRESETS.iter().all(|(_, gains)| gains.iter().all(|gain| gain.abs() <= GAIN_LIMIT)));
    }
}
