//! Настройки, которые переживают перезапуск.

use std::fs;

use serde_json::{json, Value};

use crate::api;
use crate::equalizer::{self, EqSettings};

/// Тема оформления. «Как в системе» следит за настройкой Windows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    Dark,
    Light,
    System,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Dark, Look::Light, Look::System];

    pub fn label(self) -> &'static str {
        match self {
            Look::Dark => "Тёмная",
            Look::Light => "Светлая",
            Look::System => "Как в системе",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Look::Dark => "dark",
            Look::Light => "light",
            Look::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    /// Порядок как у кнопки в оригинале.
    pub fn next(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quality {
    Low,
    Normal,
    Lossless,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Normal, Quality::Lossless];

    /// Как качество называется в запросе файла.
    pub fn code(self) -> &'static str {
        match self {
            Quality::Low => "lq",
            Quality::Normal => "nq",
            Quality::Lossless => "lossless",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Low => "Экономичное",
            Quality::Normal => "Оптимальное",
            Quality::Lossless => "Превосходное",
        }
    }
}

/// Настройки волны: название группы и варианты. Первый вариант в группе значит «любое»,
/// у него нет зерна. Номера вариантов хранятся в настройках приложения.
pub const WAVE_GROUPS: [(&str, &[(&str, &str)]); 4] = [
    ("Занятие", &[("Любое", ""), ("Просыпаюсь", "activity:wake-up"), ("В дороге", "activity:road-trip"), ("Работаю", "activity:work-background"), ("Тренируюсь", "activity:workout"), ("Засыпаю", "activity:fall-asleep")]),
    ("По характеру", &[("Любое", ""), ("Любимое", "settingDiversity:favorite"), ("Незнакомое", "settingDiversity:discover"), ("Популярное", "settingDiversity:popular")]),
    ("Под настроение", &[("Любое", ""), ("Бодрое", "settingMoodEnergy:active"), ("Весёлое", "settingMoodEnergy:fun"), ("Спокойное", "settingMoodEnergy:calm"), ("Грустное", "settingMoodEnergy:sad")]),
    ("По языку", &[("Любой", ""), ("Русский", "settingLanguage:russian"), ("Иностранный", "settingLanguage:not-russian"), ("Без слов", "settingLanguage:without-words")]),
];

/// Зёрна выбранных настроек волны.
pub fn wave_seeds(tuning: [usize; 4]) -> Vec<String> {
    WAVE_GROUPS.iter().zip(tuning).filter_map(|((_, options), choice)| options.get(choice)).filter(|(_, seed)| !seed.is_empty()).map(|(_, seed)| seed.to_string()).collect()
}

/// Названия выбранных настроек волны.
pub fn wave_names(tuning: [usize; 4]) -> Vec<&'static str> {
    WAVE_GROUPS.iter().zip(tuning).filter_map(|((_, options), choice)| options.get(choice)).filter(|(_, seed)| !seed.is_empty()).map(|(name, _)| *name).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub volume: f32,
    pub muted: bool,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub quality: Quality,
    pub wave: [usize; 4],
    pub eq: EqSettings,
    /// Плавные переходы между треками. В оригинале включены с самого начала.
    pub crossfade: bool,
    /// Одинаковая громкость треков. В оригинале её нет, поэтому выключена.
    pub normalize: bool,
    /// Пункт «Детям» в боковой панели.
    pub kids: bool,
    /// Скорость подкастов и книг; музыка всегда играет как есть.
    pub speed: f32,
    pub look: Look,
}

/// Скорости, как в оригинале: кнопка перебирает их по кругу.
pub const SPEEDS: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];

fn parse(v: &Value) -> Settings {
    Settings {
        volume: v["volume"].as_f64().map_or(0.8, |x| x.clamp(0.0, 1.0) as f32),
        muted: v["muted"].as_bool().unwrap_or(false),
        repeat: match v["repeat"].as_str() {
            Some("all") => Repeat::All,
            Some("one") => Repeat::One,
            _ => Repeat::Off,
        },
        shuffle: v["shuffle"].as_bool().unwrap_or(false),
        wave: std::array::from_fn(|group| v["wave"][group].as_u64().map_or(0, |choice| choice as usize).min(WAVE_GROUPS[group].1.len() - 1)),
        quality: Quality::ALL.into_iter().find(|q| v["quality"].as_str() == Some(q.code())).unwrap_or(Quality::Normal),
        eq: parse_eq(&v["equalizer"]),
        crossfade: v["crossfade"].as_bool().unwrap_or(true),
        normalize: v["normalize"].as_bool().unwrap_or(false),
        kids: v["kids"].as_bool().unwrap_or(true),
        speed: v["speed"].as_f64().map(|x| x as f32).filter(|x| SPEEDS.contains(x)).unwrap_or(1.0),
        look: Look::ALL.into_iter().find(|look| v["theme"].as_str() == Some(look.code())).unwrap_or(Look::Dark),
    }
}

fn parse_eq(v: &Value) -> EqSettings {
    let base = EqSettings::default();
    let limit = equalizer::GAIN_LIMIT as f64;
    EqSettings {
        enabled: v["enabled"].as_bool().unwrap_or(base.enabled),
        preset: v["preset"].as_u64().map_or(base.preset, |preset| (preset as usize).min(equalizer::PRESETS.len() - 1)),
        preamp: v["preamp"].as_f64().map_or(base.preamp, |x| x.clamp(equalizer::PREAMP_MIN as f64, equalizer::PREAMP_MAX as f64) as f32),
        gains: std::array::from_fn(|band| v["gains"][band].as_f64().map_or(0.0, |x| x.clamp(-limit, limit) as f32)),
    }
}

pub fn load() -> Settings {
    let text = fs::read(api::data_dir().join("settings.json")).unwrap_or_default();
    parse(&serde_json::from_slice(&text).unwrap_or(Value::Null))
}

/// Настройка не стоит того, чтобы мешать музыке: ошибку записи не показываем.
pub fn save(settings: Settings) {
    let repeat = match settings.repeat {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    };
    let eq = settings.eq;
    let equalizer = json!({ "enabled": eq.enabled, "preset": eq.preset, "preamp": eq.preamp, "gains": eq.gains });
    let value = json!({ "volume": settings.volume, "muted": settings.muted, "repeat": repeat, "shuffle": settings.shuffle, "quality": settings.quality.code(), "wave": settings.wave, "equalizer": equalizer, "crossfade": settings.crossfade, "normalize": settings.normalize, "kids": settings.kids, "speed": settings.speed, "theme": settings.look.code() });
    let dir = api::data_dir();
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(dir.join("settings.json"), value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_saved_and_broken_values() {
        let saved = parse(&json!({ "volume": 0.25, "muted": true, "repeat": "one", "shuffle": true, "quality": "lossless", "normalize": true }));
        assert_eq!(saved, Settings { volume: 0.25, muted: true, repeat: Repeat::One, shuffle: true, wave: [0; 4], quality: Quality::Lossless, eq: EqSettings::default(), crossfade: true, normalize: true, kids: true, speed: 1.0, look: Look::Dark });
        let empty = parse(&Value::Null);
        assert_eq!(empty, Settings { volume: 0.8, muted: false, repeat: Repeat::Off, shuffle: false, wave: [0; 4], quality: Quality::Normal, eq: EqSettings::default(), crossfade: true, normalize: false, kids: true, speed: 1.0, look: Look::Dark });
        assert!(!parse(&json!({ "crossfade": false })).crossfade);
        assert!(!parse(&json!({ "kids": false })).kids);
        assert_eq!((parse(&json!({ "theme": "system" })).look, parse(&json!({ "theme": "pink" })).look), (Look::System, Look::Dark));
        assert_eq!((parse(&json!({ "speed": 1.5 })).speed, parse(&json!({ "speed": 7 })).speed), (1.5, 1.0));
        let eq = parse(&json!({ "equalizer": { "enabled": true, "preset": 14, "preamp": 9, "gains": [40, -3.5] } })).eq;
        assert_eq!((eq.enabled, eq.preset, eq.preamp, eq.gains[0], eq.gains[1], eq.gains[2]), (true, 14, 1.5, 12.0, -3.5, 0.0));
        assert_eq!(parse(&json!({ "volume": 7 })).volume, 1.0);
    }
}
