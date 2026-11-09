//! Плавный переход между треками по точкам, которые присылает Яндекс: новый трек нарастает
//! от `inStart` до `inStop`, старый затихает от `outStart` до `outStop` и там кончается.

use std::f32::consts::FRAC_PI_2;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Затухание играющего трека. Пока его не включили, трек играет до конца как обычно.
#[derive(Clone, Default)]
pub struct FadeOut(Arc<AtomicU64>);

impl FadeOut {
    /// Начало и конец затухания в секундах от начала трека.
    pub fn arm(&self, start: f32, stop: f32) {
        // Старший бит отмечает, что затухание включено: точки всегда неотрицательные.
        self.0.store(1 << 63 | (start.max(0.0).to_bits() as u64) << 32 | stop.max(0.0).to_bits() as u64, Ordering::Release);
    }

    fn get(&self) -> Option<(f32, f32)> {
        let bits = self.0.load(Ordering::Acquire);
        (bits != 0).then(|| (f32::from_bits((bits >> 32) as u32 & 0x7fff_ffff), f32::from_bits(bits as u32)))
    }
}

/// Доля пути от `start` до `stop`, от 0 до 1.
fn part(position: f32, (start, stop): (f32, f32)) -> f32 {
    if stop <= start {
        return if position >= start { 1.0 } else { 0.0 };
    }
    ((position - start) / (stop - start)).clamp(0.0, 1.0)
}

/// Громкость по плавной S-кривой: в начале и в конце окна она меняется медленно, без рывка,
/// а громкости двух треков в сумме дают единицу. С равной мощностью два громких места
/// складывались в перегруз посередине перехода — так он звучал резко.
pub fn gain(position: f32, fade_in: Option<(f32, f32)>, fade_out: Option<(f32, f32)>) -> f32 {
    let rise = fade_in.map_or(1.0, |range| (part(position, range) * FRAC_PI_2).sin().powi(2));
    let fall = fade_out.map_or(1.0, |range| (part(position, range) * FRAC_PI_2).cos().powi(2));
    rise * fall
}

pub struct Faded<S: Source> {
    source: S,
    fade_in: Option<(f32, f32)>,
    fade_out: FadeOut,
    /// Сыгранные кадры с последней перемотки и место, с которого начали считать.
    frames: u64,
    offset: f32,
    channel: u16,
    gain: f32,
}

impl<S: Source> Faded<S> {
    pub fn new(source: S, fade_in: Option<(f32, f32)>, fade_out: FadeOut) -> Self {
        Self { source, fade_in, fade_out, frames: 0, offset: 0.0, channel: 0, gain: 1.0 }
    }

    fn place(&self) -> f32 {
        self.offset + self.frames as f32 / self.source.sample_rate().get() as f32
    }
}

impl<S: Source> Iterator for Faded<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        // Громкость считается раз на кадр, чтобы каналы шли вместе.
        if self.channel == 0 {
            let position = self.place();
            let fade_out = self.fade_out.get();
            if fade_out.is_some_and(|(_, stop)| position >= stop) {
                return None;
            }
            self.gain = gain(position, self.fade_in, fade_out);
        }
        let sample = self.source.next()?;
        self.channel += 1;
        if self.channel >= self.source.channels().get() {
            self.channel = 0;
            self.frames += 1;
        }
        Some(sample * self.gain)
    }
}

impl<S: Source> Source for Faded<S> {
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
        self.source.try_seek(position)?;
        self.offset = position.as_secs_f32();
        self.frames = 0;
        self.channel = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_follows_points() {
        assert_eq!(gain(5.0, None, None), 1.0);
        assert_eq!(gain(0.0, Some((0.0, 1.5)), None), 0.0);
        assert!((gain(1.5, Some((0.0, 1.5)), None) - 1.0).abs() < 1e-6);
        assert!((gain(118.2, None, Some((118.2, 123.2))) - 1.0).abs() < 1e-6);
        assert!(gain(123.2, None, Some((118.2, 123.2))).abs() < 1e-6);
        // Посередине оба трека звучат вполовину: вместе — как один.
        let middle = gain(0.75, Some((0.0, 1.5)), None);
        assert!((middle - 0.5).abs() < 1e-4, "{middle}");
        assert!((gain(0.3, Some((0.0, 1.5)), None) + gain(0.3, None, Some((0.0, 1.5))) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn fade_out_point_survives_packing() {
        let fade = FadeOut::default();
        assert_eq!(fade.get(), None);
        fade.arm(118.2, 123.2);
        assert_eq!(fade.get(), Some((118.2, 123.2)));
        fade.arm(0.0, 0.0);
        assert_eq!(fade.get(), Some((0.0, 0.0)));
    }

    #[test]
    fn source_rises_and_ends_after_fade_out() {
        let fade = FadeOut::default();
        let sine = rodio::source::SineWave::new(441.0).take_duration(Duration::from_secs(3));
        let mut faded = Faded::new(sine, Some((0.0, 1.0)), fade.clone());
        let rate = faded.sample_rate().get() as f32;
        let first: Vec<f32> = faded.by_ref().take((rate * 0.1) as usize).collect();
        assert!(first.iter().all(|x| x.abs() <= 0.16), "первые 0,1 с ещё тихие");
        fade.arm(1.5, 2.0);
        let rest = faded.count() as f32;
        // Источник кончился на точке конца затухания, а не через 3 секунды.
        assert!((rest / rate - 1.9).abs() < 0.01, "{}", rest / rate);
    }
}
