//! Реверб — эхо большого зала поверх трека, как у Tune Client; с замедлением получается
//! slowed + reverb. Tune Client сворачивает звук с шумовым импульсом зала, здесь — классический
//! Freeverb (восемь гребенчатых и четыре всепропускающих фильтра на канал): звучит так же
//! широко, а считается в сотни раз быстрее. Доля зала та же: зал × 0,8, прямой звук × (1 − 0,4 × доля).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

/// Длины фильтров Freeverb в отсчётах при 44,1 кГц.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// Правый канал чуть длиннее: зал звучит шире.
const SPREAD: usize = 23;
/// Длинный хвост, как у импульса Tune Client в 3,2 с.
const FEEDBACK: f32 = 0.86;
const DAMP: f32 = 0.25;
const INPUT: f32 = 0.015;

/// Доля зала 0…1, общая для плеера и играющего трека: меняется на ходу.
#[derive(Clone, Default)]
pub struct Amount(Arc<AtomicU32>);

impl Amount {
    pub fn set(&self, amount: f32) {
        self.0.store(amount.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Acquire))
    }
}

struct Comb {
    buffer: Vec<f32>,
    at: usize,
    store: f32,
}

impl Comb {
    fn run(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.at];
        self.store = output * (1.0 - DAMP) + self.store * DAMP;
        self.buffer[self.at] = input + self.store * FEEDBACK;
        self.at = (self.at + 1) % self.buffer.len();
        output
    }
}

struct Allpass {
    buffer: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn run(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.at];
        let output = delayed - input;
        self.buffer[self.at] = input + delayed * 0.5;
        self.at = (self.at + 1) % self.buffer.len();
        output
    }
}

/// Фильтры одного канала.
struct Hall {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Hall {
    fn new(rate: u32, spread: usize) -> Self {
        let scale = |length: usize| ((length + spread) as f64 * rate as f64 / 44100.0).round().max(1.0) as usize;
        Self {
            combs: COMBS.iter().map(|&length| Comb { buffer: vec![0.0; scale(length)], at: 0, store: 0.0 }).collect(),
            allpasses: ALLPASSES.iter().map(|&length| Allpass { buffer: vec![0.0; scale(length)], at: 0 }).collect(),
        }
    }

    fn run(&mut self, input: f32) -> f32 {
        let mut sum = 0.0;
        for comb in &mut self.combs {
            sum += comb.run(input);
        }
        for allpass in &mut self.allpasses {
            sum = allpass.run(sum);
        }
        sum
    }

    fn clear(&mut self) {
        for comb in &mut self.combs {
            comb.buffer.fill(0.0);
            comb.store = 0.0;
        }
        for allpass in &mut self.allpasses {
            allpass.buffer.fill(0.0);
        }
    }
}

pub struct Reverb<S> {
    source: S,
    amount: Amount,
    halls: Vec<Hall>,
    format: (u16, u32),
    channel: usize,
    /// Вход кадра: зал считается по сумме каналов, а выход у каждого канала свой.
    frame: Vec<f32>,
    wet: Vec<f32>,
    /// Зал выключен и уже затих: звук идёт как есть, фильтры не считаются.
    quiet: bool,
    /// Канал отсчёта, идущего напрямую: зал включается только с начала кадра.
    pass: usize,
}

impl<S: Source> Reverb<S> {
    pub fn new(source: S, amount: Amount) -> Self {
        Self { source, amount, halls: Vec::new(), format: (0, 0), channel: 0, frame: Vec::new(), wet: Vec::new(), quiet: true, pass: 0 }
    }

    fn refresh(&mut self) {
        let format = (self.source.channels().get(), self.source.sample_rate().get());
        if format != self.format {
            self.format = format;
            self.halls = (0..format.0 as usize).map(|channel| Hall::new(format.1, if channel % 2 == 1 { SPREAD } else { 0 })).collect();
            self.frame = vec![0.0; format.0 as usize];
            self.wet = vec![0.0; format.0 as usize];
        }
    }
}

impl<S: Source> Iterator for Reverb<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.channel == 0 {
            let amount = self.amount.get();
            if (amount <= 0.0 && self.quiet) || self.pass != 0 {
                let sample = self.source.next()?;
                self.pass = (self.pass + 1) % self.source.channels().get() as usize;
                return Some(sample);
            }
            self.refresh();
            // Кадр целиком: зал от суммы каналов.
            let mut mono = 0.0;
            for slot in self.frame.iter_mut() {
                *slot = self.source.next()?;
                mono += *slot;
            }
            let input = mono * INPUT;
            let mut loud = 0.0f32;
            for (hall, wet) in self.halls.iter_mut().zip(self.wet.iter_mut()) {
                *wet = hall.run(input);
                loud = loud.max(wet.abs());
            }
            // Выключили и хвост затих — дальше напрямую, фильтры начнут с тишины.
            self.quiet = amount <= 0.0 && loud < 1e-5;
            if self.quiet {
                for hall in &mut self.halls {
                    hall.clear();
                }
            }
            let (wet, dry) = (amount * 0.8, 1.0 - amount * 0.4);
            for (slot, hall) in self.frame.iter_mut().zip(self.wet.iter()) {
                *slot = *slot * dry + hall * wet;
            }
        }
        let sample = self.frame[self.channel];
        self.channel = (self.channel + 1) % self.frame.len();
        Some(sample)
    }
}

impl<S: Source> Source for Reverb<S> {
    /// Кадр уже взят у источника целиком: недоотданные его отсчёты — тоже часть куска.
    fn current_span_len(&self) -> Option<usize> {
        let held = if self.channel == 0 { 0 } else { self.frame.len() - self.channel };
        self.source.current_span_len().map(|length| length + held)
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
        for hall in &mut self.halls {
            hall.clear();
        }
        self.channel = 0;
        self.pass = 0;
        self.source.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    fn click() -> SamplesBuffer {
        let rate = 44100;
        let mut samples = vec![0.0; rate as usize * 2 * 2];
        samples[0] = 1.0;
        samples[1] = 1.0;
        SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(rate).unwrap(), samples)
    }

    #[test]
    fn off_passes_through() {
        let original: Vec<f32> = click().collect();
        let out: Vec<f32> = Reverb::new(click(), Amount::default()).collect();
        assert_eq!(out, original);
    }

    #[test]
    fn hall_rings_after_click() {
        let amount = Amount::default();
        amount.set(0.5);
        let out: Vec<f32> = Reverb::new(click(), amount).collect();
        assert_eq!(out.len(), 44100 * 2 * 2);
        // Через полсекунды после щелчка зал ещё звучит, а каналы различаются — звук широкий.
        let later = &out[44100..44100 + 2000];
        assert!(later.iter().any(|sample| sample.abs() > 1e-4));
        assert!(later.chunks(2).any(|frame| (frame[0] - frame[1]).abs() > 1e-5));
    }
}
