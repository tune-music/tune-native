//! Пространственное вращение 8D: панорама с сохранением мощности, интерауральная задержка
//! (ITD до ~0,48 мс), частотное затенение дальнего уха и спереди/сзади, а также авто-режимы
//! по общей динамике трека и по ударам баса.

use std::f32::consts::{FRAC_PI_4, TAU};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};

pub const PERIOD_MIN: f32 = 2.0;
pub const PERIOD_MAX: f32 = 20.0;
pub const SENSITIVITY_MIN: f32 = 0.1;
pub const SENSITIVITY_MAX: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EightDMode {
    Fixed,
    Energy,
    Beats,
}

impl EightDMode {
    pub const ALL: [EightDMode; 3] = [EightDMode::Fixed, EightDMode::Energy, EightDMode::Beats];

    pub fn code(self) -> &'static str {
        match self {
            EightDMode::Fixed => "fixed",
            EightDMode::Energy => "energy",
            EightDMode::Beats => "beats",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "energy" => EightDMode::Energy,
            "beats" => EightDMode::Beats,
            _ => EightDMode::Fixed,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EightDMode::Fixed => "Фиксированный",
            EightDMode::Energy => "По динамике трека",
            EightDMode::Beats => "Под биты",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            EightDMode::Fixed => "Равномерное перемещение между каналами с заданным периодом.",
            EightDMode::Energy => "На громких участках и припевах звук движется быстрее, на тихих — плавнее.",
            EightDMode::Beats => "Удары баса и бочки ускоряют и перебрасывают звук между каналами.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EightDSettings {
    pub enabled: bool,
    pub mode: EightDMode,
    pub period: f32,
    pub smoothness: f32,
    pub depth: f32,
    pub sensitivity: f32,
}

impl Default for EightDSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: EightDMode::Fixed,
            period: 8.0,
            smoothness: 0.85,
            depth: 0.85,
            sensitivity: 0.65,
        }
    }
}

#[derive(Clone, Default)]
pub struct Shared {
    settings: Arc<Mutex<EightDSettings>>,
    version: Arc<AtomicU64>,
}

impl Shared {
    pub fn set(&self, settings: EightDSettings) {
        *self.settings.lock().unwrap() = settings;
        self.version.fetch_add(1, Ordering::Release);
    }
}

struct Spatial8D {
    rate: f32,
    delay_mask: usize,
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    write_index: usize,

    phase: f32,
    lowpass_state: f32,
    fast_bass_env: f32,
    slow_bass_env: f32,
    fast_energy_env: f32,
    slow_energy_env: f32,
    beat_boost: f32,
    beat_cooldown: usize,
    direction: f32,

    shade_l: f32,
    shade_r: f32,

    bass_alpha: f32,
    fast_bass_attack: f32,
    fast_bass_release: f32,
    slow_bass_alpha: f32,
    fast_energy_alpha: f32,
    slow_energy_alpha: f32,
    beat_decay: f32,
    min_beat_gap: usize,
    max_itd_samples: f32,
    base_shade_alpha: f32,
}

impl Spatial8D {
    fn new(sample_rate: u32) -> Self {
        let rate = (sample_rate as f32).max(8000.0);
        let mut size = 64usize;
        let needed = (rate * 0.002).ceil() as usize + 4;
        while size < needed {
            size <<= 1;
        }
        Self {
            rate,
            delay_mask: size - 1,
            delay_l: vec![0.0; size],
            delay_r: vec![0.0; size],
            write_index: 0,
            phase: 0.0,
            lowpass_state: 0.0,
            fast_bass_env: 0.0,
            slow_bass_env: 0.01,
            fast_energy_env: 0.0,
            slow_energy_env: 0.02,
            beat_boost: 0.0,
            beat_cooldown: 0,
            direction: 1.0,
            shade_l: 0.0,
            shade_r: 0.0,
            bass_alpha: (TAU * 130.0 / rate).min(0.5),
            fast_bass_attack: 1.0 - (-1.0 / (rate * 0.004)).exp(),
            fast_bass_release: 1.0 - (-1.0 / (rate * 0.045)).exp(),
            slow_bass_alpha: 1.0 - (-1.0 / (rate * 0.38)).exp(),
            fast_energy_alpha: 1.0 - (-1.0 / (rate * 0.12)).exp(),
            slow_energy_alpha: 1.0 - (-1.0 / (rate * 2.2)).exp(),
            beat_decay: (-1.0 / (rate * 0.14)).exp(),
            min_beat_gap: (rate * 0.2) as usize,
            max_itd_samples: rate * 0.00048,
            base_shade_alpha: (TAU * 3400.0 / rate).min(0.85),
        }
    }

    fn reset(&mut self) {
        self.delay_l.fill(0.0);
        self.delay_r.fill(0.0);
        self.write_index = 0;
        self.lowpass_state = 0.0;
        self.fast_bass_env = 0.0;
        self.slow_bass_env = 0.01;
        self.fast_energy_env = 0.0;
        self.slow_energy_env = 0.02;
        self.beat_boost = 0.0;
        self.beat_cooldown = 0;
        self.shade_l = 0.0;
        self.shade_r = 0.0;
    }

    fn shaped_pan(phase: f32, smoothness: f32, depth: f32) -> f32 {
        let raw = phase.sin();
        let sharpness = 1.0 + (1.0 - smoothness.clamp(0.0, 1.0)) * 8.0;
        let shaped = if sharpness <= 1.001 {
            raw
        } else {
            (raw * sharpness).tanh() / sharpness.tanh()
        };
        shaped * depth.clamp(0.0, 1.0)
    }

    fn process(&mut self, left: f32, right: f32, settings: EightDSettings) -> (f32, f32) {
        let mid = (left + right) * 0.5;
        let side = (left - right) * 0.5;

        let abs_mid = mid.abs();
        self.fast_energy_env += (abs_mid - self.fast_energy_env) * self.fast_energy_alpha;
        self.slow_energy_env += (abs_mid - self.slow_energy_env) * self.slow_energy_alpha;

        self.lowpass_state += (mid - self.lowpass_state) * self.bass_alpha;
        let bass = self.lowpass_state.abs();
        let bass_coeff = if bass > self.fast_bass_env {
            self.fast_bass_attack
        } else {
            self.fast_bass_release
        };
        self.fast_bass_env += (bass - self.fast_bass_env) * bass_coeff;
        self.slow_bass_env += (bass - self.slow_bass_env) * self.slow_bass_alpha;

        if self.beat_cooldown > 0 {
            self.beat_cooldown -= 1;
        }
        self.beat_boost *= self.beat_decay;

        let sensitivity = settings.sensitivity.clamp(SENSITIVITY_MIN, SENSITIVITY_MAX);
        let speed_multiplier = match settings.mode {
            EightDMode::Fixed => {
                self.direction = 1.0;
                1.0
            }
            EightDMode::Energy => {
                self.direction = 1.0;
                let ratio = self.fast_energy_env / self.slow_energy_env.max(0.008);
                let delta = (ratio - 1.0) * (1.2 + sensitivity * 2.2);
                (1.0 + delta).clamp(0.35, 3.4)
            }
            EightDMode::Beats => {
                let threshold = 1.95 - sensitivity * 0.72;
                if self.beat_cooldown == 0
                    && self.fast_bass_env > 0.018
                    && self.fast_bass_env > self.slow_bass_env * threshold
                {
                    let strength = ((self.fast_bass_env / self.slow_bass_env.max(0.008) - threshold) * 1.4).min(1.0);
                    self.beat_boost = (self.beat_boost + 1.6 + strength * (1.5 + sensitivity * 2.0)).min(4.5);
                    if strength > 0.55 && sensitivity > 0.45 {
                        self.phase += self.direction * (0.28 + 0.35 * sensitivity);
                    }
                    self.beat_cooldown = self.min_beat_gap;
                }
                0.65 + self.beat_boost
            }
        };

        let period = settings.period.clamp(PERIOD_MIN, PERIOD_MAX);
        let angular_step = (TAU / (period * self.rate)) * speed_multiplier * self.direction;
        self.phase += angular_step;
        if self.phase >= TAU {
            self.phase -= TAU;
        } else if self.phase < 0.0 {
            self.phase += TAU;
        }

        let depth = settings.depth.clamp(0.0, 1.0);
        let pan = Self::shaped_pan(self.phase, settings.smoothness, depth);
        let front_back = self.phase.cos();

        let stereo_keep = 0.35 * (1.0 - depth * 0.65);
        let source_l = mid + side * stereo_keep;
        let source_r = mid - side * stereo_keep;

        self.delay_l[self.write_index] = source_l;
        self.delay_r[self.write_index] = source_r;
        let delayed_l = self.read_delay(&self.delay_l, pan.max(0.0) * self.max_itd_samples);
        let delayed_r = self.read_delay(&self.delay_r, (-pan).max(0.0) * self.max_itd_samples);
        self.write_index = (self.write_index + 1) & self.delay_mask;

        let rear_shadow = (-front_back).max(0.0) * 0.25 * depth;
        let shadow_l = (pan.max(0.0) * 0.55 + rear_shadow).min(0.75);
        let shadow_r = ((-pan).max(0.0) * 0.55 + rear_shadow).min(0.75);

        self.shade_l += (delayed_l - self.shade_l) * self.base_shade_alpha;
        self.shade_r += (delayed_r - self.shade_r) * self.base_shade_alpha;

        let filtered_l = delayed_l * (1.0 - shadow_l) + self.shade_l * shadow_l;
        let filtered_r = delayed_r * (1.0 - shadow_r) + self.shade_r * shadow_r;

        let angle = (pan + 1.0) * FRAC_PI_4;
        let (gain_r, gain_l) = angle.sin_cos();

        let cross = 0.12 * (1.0 - depth * 0.4);
        let out_l = filtered_l * (gain_l + cross * gain_r);
        let out_r = filtered_r * (gain_r + cross * gain_l);
        (out_l, out_r)
    }

    fn read_delay(&self, buffer: &[f32], offset: f32) -> f32 {
        let whole = offset as usize;
        let frac = offset - whole as f32;
        let i0 = self.write_index.wrapping_sub(whole) & self.delay_mask;
        let i1 = i0.wrapping_sub(1) & self.delay_mask;
        buffer[i0] * (1.0 - frac) + buffer[i1] * frac
    }
}

pub struct Spatial<S> {
    source: S,
    shared: Shared,
    version: u64,
    settings: EightDSettings,
    dsp: Option<Spatial8D>,
    format: (u16, u32),
    channel: usize,
    frame: Vec<f32>,
}

impl<S: Source> Spatial<S> {
    pub fn new(source: S, shared: Shared) -> Self {
        let mut out = Self {
            source,
            shared,
            version: u64::MAX,
            settings: EightDSettings::default(),
            dsp: None,
            format: (0, 0),
            channel: 0,
            frame: Vec::new(),
        };
        out.refresh();
        out
    }

    fn refresh(&mut self) {
        let version = self.shared.version.load(Ordering::Acquire);
        let format = (self.source.channels().get(), self.source.sample_rate().get());
        if version == self.version && format == self.format {
            return;
        }
        if format != self.format {
            self.format = format;
            self.frame = vec![0.0; format.0 as usize];
            self.dsp = if format.0 >= 2 { Some(Spatial8D::new(format.1)) } else { None };
        }
        self.version = version;
        let next = *self.shared.settings.lock().unwrap();
        if !next.enabled && self.settings.enabled {
            if let Some(dsp) = &mut self.dsp {
                dsp.reset();
            }
        }
        self.settings = next;
    }
}

impl<S: Source> Iterator for Spatial<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.channel == 0 {
            self.refresh();
            if !self.settings.enabled || self.format.0 < 2 {
                return self.source.next();
            }
            for slot in self.frame.iter_mut() {
                *slot = self.source.next()?;
            }
            if let Some(dsp) = &mut self.dsp {
                let (l, r) = dsp.process(self.frame[0], self.frame[1], self.settings);
                self.frame[0] = l;
                self.frame[1] = r;
            }
        }
        let sample = self.frame[self.channel];
        self.channel = (self.channel + 1) % self.frame.len().max(1);
        Some(sample)
    }
}

impl<S: Source> Source for Spatial<S> {
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
        if let Some(dsp) = &mut self.dsp {
            dsp.reset();
        }
        self.channel = 0;
        self.source.try_seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    #[test]
    fn off_passes_through() {
        let samples = vec![0.25f32, -0.5, 0.75, -0.1];
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(44100).unwrap(), samples.clone());
        let out: Vec<f32> = Spatial::new(src, Shared::default()).collect();
        assert_eq!(out, samples);
    }

    #[test]
    fn shaped_pan_respects_smoothness_and_depth() {
        let smooth = Spatial8D::shaped_pan(PI * 0.25, 1.0, 1.0);
        let sharp = Spatial8D::shaped_pan(PI * 0.25, 0.0, 1.0);
        assert!(sharp > smooth);
        assert!(Spatial8D::shaped_pan(PI * 0.5, 1.0, 0.0).abs() < 1e-6);
    }

    #[test]
    fn rotates_stereo_channels() {
        let samples = vec![0.5f32; 44100 * 2 * 2];
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(44100).unwrap(), samples);
        let shared = Shared::default();
        shared.set(EightDSettings {
            enabled: true,
            mode: EightDMode::Fixed,
            period: 2.0,
            smoothness: 1.0,
            depth: 1.0,
            sensitivity: 0.65,
        });
        let out: Vec<f32> = Spatial::new(src, shared).collect();
        let diffs: Vec<f32> = out.chunks_exact(2).step_by(4410).map(|f| f[0] - f[1]).collect();
        assert!(diffs.iter().any(|&d| d > 0.15) && diffs.iter().any(|&d| d < -0.15));
    }
}
