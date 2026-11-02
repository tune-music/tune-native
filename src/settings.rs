//! Настройки, которые переживают перезапуск.

use std::fs;

use serde_json::{json, Value};

use crate::api;

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
    /// Моя волна откреплена. На сервере она остаётся всегда, поэтому это хранится здесь.
    pub my_wave_hidden: bool,
}

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
        my_wave_hidden: v["my_wave_hidden"].as_bool().unwrap_or(false),
        wave: std::array::from_fn(|group| v["wave"][group].as_u64().map_or(0, |choice| choice as usize).min(WAVE_GROUPS[group].1.len() - 1)),
        quality: Quality::ALL.into_iter().find(|q| v["quality"].as_str() == Some(q.code())).unwrap_or(Quality::Normal),
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
    let value = json!({ "my_wave_hidden": settings.my_wave_hidden, "volume": settings.volume, "muted": settings.muted, "repeat": repeat, "shuffle": settings.shuffle, "quality": settings.quality.code(), "wave": settings.wave });
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
        let saved = parse(&json!({ "volume": 0.25, "muted": true, "repeat": "one", "shuffle": true, "quality": "lossless" }));
        assert_eq!(saved, Settings { volume: 0.25, muted: true, repeat: Repeat::One, shuffle: true, wave: [0; 4], my_wave_hidden: false, quality: Quality::Lossless });
        let empty = parse(&Value::Null);
        assert_eq!(empty, Settings { volume: 0.8, muted: false, repeat: Repeat::Off, shuffle: false, wave: [0; 4], my_wave_hidden: false, quality: Quality::Normal });
        assert_eq!(parse(&json!({ "volume": 7 })).volume, 1.0);
    }
}
