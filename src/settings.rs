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

    /// Подпись под качеством, как в меню оригинала.
    pub fn about(self) -> &'static str {
        match self {
            Quality::Low => "Стабильное звучание при медленном интернете",
            Quality::Normal => "Сбалансированный звук для большинства устройств",
            Quality::Lossless => "Музыка в lossless и других высококачественных форматах для быстрого интернета и хорошей акустики",
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
    /// Облегчённая версия, как в оригинале: без живого фона и тяжёлых эффектов.
    pub lite: bool,
    /// Пункт «Детям» в боковой панели.
    pub kids: bool,
    /// Пункт «Книги и подкасты» в боковой панели.
    pub books: bool,
    /// Пункт «Концерты» в боковой панели.
    pub concerts: bool,
    /// Карточки «Слов в волне» — короткие ИИ-факты о треке на экране волны.
    pub words: bool,
    /// Колесо подсказок над Моей волной.
    pub wheel: bool,
    /// Скорость подкастов и книг; музыка всегда играет как есть.
    pub speed: f32,
    pub look: Look,
    /// После запуска в плеере трек и очередь, что играли при выходе. Хранятся у Tune,
    /// а не на сервере: Ynison у части аккаунтов их не отдаёт.
    pub keep_track: bool,
    /// Свои загруженные треки иногда попадаются в «Моей волне», как в Tune Client.
    pub own_in_wave: bool,
    /// Удалённые Яндексом треки, которые вернул «Без цензуры», подмешиваются в волну по вкусу.
    pub restored_in_wave: bool,
    /// Без цензуры по списку FckCensor, как в Tune Client.
    pub uncensor: bool,
    pub window: WindowPrefs,
    /// Громкость колесом мыши над плеером, как у Tune Client.
    pub volume_wheel: bool,
    /// Шаг громкости в процентах: для колеса и стрелок.
    pub volume_step: u8,
    /// Кнопка скорости для любых треков, а не только книг и подкастов, как у Tune Client.
    pub speed_control: bool,
    /// Шаг кнопки скорости.
    pub speed_step: f32,
    /// Тон меняется вместе со скоростью: найткор и слоуд.
    pub pitch: bool,
    /// Реверб, 0–100 %: доля зала в звуке.
    pub reverb: u8,
    /// Откуда брать текст песен.
    pub lyrics: crate::lyrics::Sources,
    /// Размер строк текста песен.
    pub lyrics_size: u8,
}

/// Размеры текста песен на выбор.
pub const LYRICS_SIZES: [u8; 4] = [20, 26, 32, 38];

/// Шаги кнопки скорости на выбор, как у Tune Client.
pub const SPEED_STEPS: [f32; 3] = [0.05, 0.1, 0.25];
pub const SPEED_STEP_MIN: f32 = 0.01;
pub const SPEED_STEP_MAX: f32 = 0.5;
/// Пределы скорости с кнопкой для любых треков.
pub const SPEED_MIN: f32 = 0.5;
pub const SPEED_MAX: f32 = 2.0;

/// Шаги громкости на выбор, в процентах; свой — от 1 до `VOLUME_STEP_MAX`.
pub const VOLUME_STEPS: [u8; 4] = [1, 2, 5, 10];
pub const VOLUME_STEP_MAX: u8 = 25;

/// Окно и запуск — как во вкладке «Окно» у Tune Client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPrefs {
    /// Запускаться вместе с Windows, свёрнутым в трей.
    pub autostart: bool,
    /// Окно не появляется при обычном запуске — его открывают из трея.
    pub start_minimized: bool,
    /// Крестик прячет окно в трей, музыка играет дальше.
    pub close_to_tray: bool,
    /// «Исполнитель — трек» в заголовке окна.
    pub track_title: bool,
    /// «Исполнитель — трек» на панели задач и в Alt+Tab.
    pub track_taskbar: bool,
    /// Ссылки `yandexmusic://` («Открыть в приложении» на сайте) открывает Tune.
    pub yandex_links: bool,
}

impl Default for WindowPrefs {
    fn default() -> Self {
        Self { autostart: false, start_minimized: false, close_to_tray: true, track_title: false, track_taskbar: false, yandex_links: false }
    }
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
        lite: v["lite"].as_bool().unwrap_or(false),
        kids: v["kids"].as_bool().unwrap_or(true),
        books: v["books"].as_bool().unwrap_or(true),
        concerts: v["concerts"].as_bool().unwrap_or(true),
        words: v["words"].as_bool().unwrap_or(true),
        wheel: v["wheel"].as_bool().unwrap_or(true),
        speed: v["speed"].as_f64().map(|x| ((x * 100.0).round() / 100.0) as f32).filter(|x| (SPEED_MIN..=SPEED_MAX).contains(x)).unwrap_or(1.0),
        look: Look::ALL.into_iter().find(|look| v["theme"].as_str() == Some(look.code())).unwrap_or(Look::Dark),
        keep_track: v["keepTrack"].as_bool().unwrap_or(true),
        uncensor: v["uncensor"].as_bool().unwrap_or(true),
        own_in_wave: v["ownInWave"].as_bool().unwrap_or(true),
        restored_in_wave: v["restoredInWave"].as_bool().unwrap_or(true),
        volume_wheel: v["volumeWheel"].as_bool().unwrap_or(true),
        speed_control: v["speedControl"].as_bool().unwrap_or(false),
        speed_step: v["speedStep"].as_f64().map(|x| ((x * 100.0).round() / 100.0) as f32).filter(|x| (SPEED_STEP_MIN..=SPEED_STEP_MAX).contains(x)).unwrap_or(0.25),
        pitch: v["pitchWithSpeed"].as_bool().unwrap_or(false),
        reverb: v["reverb"].as_u64().map_or(0, |x| x.min(100) as u8),
        lyrics: {
            let base = crate::lyrics::Sources::default();
            let l = &v["lyrics"];
            crate::lyrics::Sources {
                enabled: l["enabled"].as_bool().unwrap_or(base.enabled),
                always: l["always"].as_bool().unwrap_or(base.always),
                lrclib: l["lrclib"].as_bool().unwrap_or(base.lrclib),
                netease: l["netease"].as_bool().unwrap_or(base.netease),
                netease_first: l["neteaseFirst"].as_bool().unwrap_or(base.netease_first),
            }
        },
        lyrics_size: v["lyricsSize"].as_u64().map(|x| x as u8).filter(|x| LYRICS_SIZES.contains(x)).unwrap_or(26),
        volume_step: v["volumeStep"].as_u64().map(|step| step as u8).filter(|step| (1..=VOLUME_STEP_MAX).contains(step)).unwrap_or(5),
        window: {
            let base = WindowPrefs::default();
            let w = &v["window"];
            WindowPrefs {
                autostart: w["autostart"].as_bool().unwrap_or(base.autostart),
                start_minimized: w["startMinimized"].as_bool().unwrap_or(base.start_minimized),
                close_to_tray: w["closeToTray"].as_bool().unwrap_or(base.close_to_tray),
                // Ключ новый: прежний «trackTitle» был включён по умолчанию, а у Tune Client трека в
                // заголовке нет — старое значение не переносится.
                track_title: w["trackInTitle"].as_bool().unwrap_or(base.track_title),
                track_taskbar: w["trackInTaskbar"].as_bool().unwrap_or(base.track_taskbar),
                yandex_links: w["yandexLinks"].as_bool().unwrap_or(base.yandex_links),
            }
        },
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
    let l = settings.lyrics;
    let lyrics = json!({ "enabled": l.enabled, "always": l.always, "lrclib": l.lrclib, "netease": l.netease, "neteaseFirst": l.netease_first });
    let w = settings.window;
    let window = json!({ "autostart": w.autostart, "startMinimized": w.start_minimized, "closeToTray": w.close_to_tray, "trackInTitle": w.track_title, "trackInTaskbar": w.track_taskbar, "yandexLinks": w.yandex_links });
    let value = json!({ "volume": settings.volume, "muted": settings.muted, "repeat": repeat, "shuffle": settings.shuffle, "quality": settings.quality.code(), "wave": settings.wave, "equalizer": equalizer, "crossfade": settings.crossfade, "normalize": settings.normalize, "lite": settings.lite, "kids": settings.kids, "books": settings.books, "concerts": settings.concerts, "words": settings.words, "wheel": settings.wheel, "speed": settings.speed, "theme": settings.look.code(), "keepTrack": settings.keep_track, "uncensor": settings.uncensor, "ownInWave": settings.own_in_wave, "restoredInWave": settings.restored_in_wave, "window": window, "volumeWheel": settings.volume_wheel, "volumeStep": settings.volume_step, "speedControl": settings.speed_control, "speedStep": settings.speed_step, "pitchWithSpeed": settings.pitch, "reverb": settings.reverb, "lyrics": lyrics, "lyricsSize": settings.lyrics_size });
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
        assert_eq!(saved, Settings { volume: 0.25, muted: true, repeat: Repeat::One, shuffle: true, wave: [0; 4], quality: Quality::Lossless, eq: EqSettings::default(), crossfade: true, normalize: true, lite: false, kids: true, books: true, concerts: true, words: true, wheel: true, speed: 1.0, look: Look::Dark, keep_track: true, uncensor: true, own_in_wave: true, restored_in_wave: true, window: WindowPrefs::default(), volume_wheel: true, volume_step: 5, speed_control: false, speed_step: 0.25, pitch: false, reverb: 0, lyrics: crate::lyrics::Sources::default(), lyrics_size: 26 });
        let empty = parse(&Value::Null);
        assert_eq!(empty, Settings { volume: 0.8, muted: false, repeat: Repeat::Off, shuffle: false, wave: [0; 4], quality: Quality::Normal, eq: EqSettings::default(), crossfade: true, normalize: false, lite: false, kids: true, books: true, concerts: true, words: true, wheel: true, speed: 1.0, look: Look::Dark, keep_track: true, uncensor: true, own_in_wave: true, restored_in_wave: true, window: WindowPrefs::default(), volume_wheel: true, volume_step: 5, speed_control: false, speed_step: 0.25, pitch: false, reverb: 0, lyrics: crate::lyrics::Sources::default(), lyrics_size: 26 });
        assert!(!parse(&json!({ "crossfade": false })).crossfade);
        assert!(!parse(&json!({ "kids": false })).kids);
        assert!(!parse(&json!({ "books": false })).books && !parse(&json!({ "concerts": false })).concerts);
        assert!(!parse(&json!({ "keepTrack": false })).keep_track);
        assert_eq!((parse(&json!({ "volumeStep": 2 })).volume_step, parse(&json!({ "volumeStep": 7 })).volume_step, parse(&json!({ "volumeStep": 40 })).volume_step), (2, 7, 5));
        let window = parse(&json!({ "window": { "autostart": true, "closeToTray": false } })).window;
        assert_eq!(window, WindowPrefs { autostart: true, start_minimized: false, close_to_tray: false, track_title: false, track_taskbar: false, yandex_links: false });
        assert!(!parse(&json!({ "words": false })).words);
        assert_eq!((parse(&json!({ "theme": "system" })).look, parse(&json!({ "theme": "pink" })).look), (Look::System, Look::Dark));
        assert_eq!((parse(&json!({ "speed": 1.5 })).speed, parse(&json!({ "speed": 7 })).speed, parse(&json!({ "speed": 0.75 })).speed), (1.5, 1.0, 0.75));
        assert_eq!((parse(&json!({ "reverb": 250 })).reverb, parse(&json!({ "speedStep": 0.1 })).speed_step, parse(&json!({ "speedStep": 3 })).speed_step), (100, 0.1, 0.25));
        assert_eq!(parse(&json!({ "speedStep": 0.05000000074505806 })).speed_step, 0.05);
        let eq = parse(&json!({ "equalizer": { "enabled": true, "preset": 14, "preamp": 9, "gains": [40, -3.5] } })).eq;
        assert_eq!((eq.enabled, eq.preset, eq.preamp, eq.gains[0], eq.gains[1], eq.gains[2]), (true, 14, 1.5, 12.0, -3.5, 0.0));
        assert_eq!(parse(&json!({ "volume": 7 })).volume, 1.0);
    }
}
