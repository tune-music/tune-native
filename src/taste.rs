//! Удалённые треки в «Моей волне» — по вкусу, как в Tune Client. Рекомендации Яндекса
//! считаются на его серверах и треков, которые он удалил, не знают. Tune оценивает их сам,
//! по исполнителям: кого ставит текущая волна, кто на них похож (`artists/<id>/similar`) и
//! как человек реагирует в волне. Всё хранится только на этом компьютере.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

const TASTE_MIN: f64 = -10.0;
const TASTE_MAX: f64 = 10.0;
/// Исполнитель с такой оценкой в волну не попадает.
const TASTE_BLOCK: f64 = -3.0;
/// Меньше — трек не подмешиваем.
pub const WAVE_PASS: f64 = 2.0;
const DAY: u64 = 24 * 3600;
/// Старые реакции забываются: половина оценки за столько дней.
const HALF_LIFE: f64 = 60.0;
/// Похожие исполнители перечитываются раз в неделю.
const SIMILAR_DAYS: u64 = 7;
/// Сыгранный удалённый трек не повторяется неделю, и после перезапуска тоже.
const PLAYED_DAYS: u64 = 7;
/// Сколько похожих запрашивать за раз: волна их не ждёт, они нужны следующим пачкам.
const SIMILAR_BATCH: usize = 3;

#[derive(Default)]
struct Store {
    /// Исполнитель → оценка и когда она менялась (секунды).
    taste: HashMap<String, (f64, u64)>,
    similar: HashMap<String, (u64, HashSet<String>)>,
    played: HashMap<String, u64>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);
static SIMILAR_BUSY: AtomicBool = AtomicBool::new(false);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn file(name: &str) -> std::path::PathBuf {
    crate::api::data_dir().join(name)
}

fn read(name: &str) -> Value {
    std::fs::read(file(name)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null)
}

fn write(name: &str, value: &Value) {
    let _ = std::fs::write(file(name), value.to_string());
}

fn load() -> Store {
    let mut store = Store::default();
    for (id, entry) in read("wave-taste.json").as_object().into_iter().flatten() {
        if let (Some(score), Some(at)) = (entry[0].as_f64(), entry[1].as_u64()) {
            store.taste.insert(id.clone(), (score, at));
        }
    }
    for (id, entry) in read("wave-similar.json").as_object().into_iter().flatten() {
        let ids = entry["ids"].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_string)).collect();
        store.similar.insert(id.clone(), (entry["at"].as_u64().unwrap_or(0), ids));
    }
    for (id, at) in read("wave-played.json").as_object().into_iter().flatten() {
        store.played.insert(id.clone(), at.as_u64().unwrap_or(0));
    }
    store
}

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut guard = STORE.lock().unwrap();
    f(guard.get_or_insert_with(load))
}

fn taste_of(store: &Store, id: &str, at: u64) -> f64 {
    store.taste.get(id).map_or(0.0, |(score, since)| score * 0.5f64.powf(at.saturating_sub(*since) as f64 / (HALF_LIFE * DAY as f64)))
}

/// Сколько событие волны добавляет оценке исполнителя.
fn delta(kind: &str, played: Option<f32>, length: f32) -> f64 {
    match kind {
        "trackFinished" => {
            if length <= 0.0 || played.unwrap_or(length) / length >= 0.7 { 1.0 } else { 0.0 }
        }
        "skip" if played.is_some_and(|played| played < 30.0) => -1.0,
        "like" => 3.0,
        "unlike" => -3.0,
        "dislike" => -5.0,
        "undislike" => 5.0,
        _ => 0.0,
    }
}

/// Реакция в волне: оценки исполнителей трека меняются и сохраняются.
pub fn react(kind: &str, artists: &[String], played: Option<f32>, length: f32) {
    let change = delta(kind, played, length);
    if change == 0.0 || artists.is_empty() {
        return;
    }
    with(|store| {
        let at = now();
        for artist in artists {
            let score = (taste_of(store, artist, at) + change).clamp(TASTE_MIN, TASTE_MAX);
            store.taste.insert(artist.clone(), (score, at));
        }
        let value: serde_json::Map<String, Value> = store.taste.iter().map(|(id, (score, at))| (id.clone(), json!([score, at]))).collect();
        write("wave-taste.json", &Value::Object(value));
    });
}

/// Любимые исполнители волны: с оценкой от единицы, двадцать лучших.
pub fn liked_artists() -> Vec<String> {
    with(|store| {
        let at = now();
        let mut liked: Vec<(String, f64)> = store.taste.keys().map(|id| (id.clone(), taste_of(store, id, at))).filter(|(_, score)| *score >= 1.0).collect();
        liked.sort_by(|a, b| b.1.total_cmp(&a.1));
        liked.into_iter().take(20).map(|(id, _)| id).collect()
    })
}

/// Подгружает похожих исполнителей в фоне, понемногу. Ошибка не важна: повторится позже.
pub fn load_similar(seeds: Vec<String>, token: String) {
    let need: Vec<String> = with(|store| {
        let at = now();
        seeds.into_iter().filter(|id| store.similar.get(id).is_none_or(|(since, _)| at.saturating_sub(*since) > SIMILAR_DAYS * DAY)).take(SIMILAR_BATCH).collect()
    });
    if need.is_empty() || SIMILAR_BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        for id in need {
            if let Ok(ids) = crate::api::similar_artist_ids(&id, &token) {
                with(|store| {
                    store.similar.insert(id, (now(), ids.into_iter().collect()));
                    let value: serde_json::Map<String, Value> = store.similar.iter().map(|(id, (at, ids))| (id.clone(), json!({ "at": at, "ids": ids }))).collect();
                    write("wave-similar.json", &Value::Object(value));
                });
            }
        }
        SIMILAR_BUSY.store(false, Ordering::Release);
    });
}

/// Удалённый трек уже был в волне на этой неделе.
pub fn played(id: &str) -> bool {
    with(|store| store.played.get(id).is_some_and(|at| now().saturating_sub(*at) < PLAYED_DAYS * DAY))
}

pub fn mark_played(id: &str) {
    with(|store| {
        let at = now();
        store.played.retain(|_, since| at.saturating_sub(*since) < PLAYED_DAYS * DAY);
        store.played.insert(id.to_string(), at);
        let value: serde_json::Map<String, Value> = store.played.iter().map(|(id, at)| (id.clone(), json!(at))).collect();
        write("wave-played.json", &Value::Object(value));
    });
}

/// Оценка трека по его исполнителям; `None` — в волну он не годится совсем.
/// `recent` — исполнители последних треков волны, `liked` — любимые исполнители волны.
pub fn score(artists: &[String], liked_track: bool, recent: &HashSet<String>, liked: &[String]) -> Option<f64> {
    with(|store| score_in(store, now(), artists, liked_track, recent, liked))
}

fn score_in(store: &Store, at: u64, artists: &[String], liked_track: bool, recent: &HashSet<String>, liked: &[String]) -> Option<f64> {
    if artists.is_empty() {
        return None;
    }
    let tastes: Vec<f64> = artists.iter().map(|id| taste_of(store, id, at)).collect();
    if tastes.iter().any(|taste| *taste <= TASTE_BLOCK) {
        return None;
    }
    let mut score = if liked_track { 2.0 } else { 0.0 };
    if artists.iter().any(|id| recent.contains(id)) {
        score += 2.0;
    }
    score += tastes.iter().copied().fold(f64::MIN, f64::max).clamp(-2.0, 3.0);
    // Похожий на исполнителя, которого дослушивают, — сильный довод; на исполнителя из волны —
    // слабый: одного такого совпадения мало, нужны два.
    let similar_to = |seed: &String| store.similar.get(seed).is_some_and(|(_, list)| artists.iter().any(|id| list.contains(id)));
    let similar = liked.iter().filter(|seed| similar_to(seed)).count() as f64 * 2.0 + recent.iter().filter(|seed| similar_to(seed)).count() as f64;
    Some(score + similar.min(2.0))
}

/// Случайный выбор с весом по оценке. `random` — от 0 до 1.
pub fn pick_weighted(scores: &[f64], random: f64) -> Option<usize> {
    let total: f64 = scores.iter().sum();
    if scores.is_empty() || total <= 0.0 {
        return None;
    }
    let mut at = random * total;
    for (index, score) in scores.iter().enumerate() {
        at -= score;
        if at < 0.0 {
            return Some(index);
        }
    }
    Some(scores.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taste_and_similar_make_the_score() {
        let mut store = Store::default();
        store.taste.insert("bad".into(), (-4.0, 100));
        store.taste.insert("fan".into(), (5.0, 100));
        store.similar.insert("fan".into(), (100, ["near".to_string()].into()));
        let recent: HashSet<String> = ["wave".to_string()].into();
        let liked = vec!["fan".to_string()];
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(score_in(&store, 100, &ids(&["bad", "near"]), false, &recent, &liked), None);
        assert_eq!(score_in(&store, 100, &[], false, &recent, &liked), None);
        // Любимый исполнитель: оценка 5 обрезается до 3.
        assert_eq!(score_in(&store, 100, &ids(&["fan"]), false, &recent, &liked), Some(3.0));
        // Похожий на любимого, в лайках, и его исполнитель уже звучал в волне.
        assert_eq!(score_in(&store, 100, &ids(&["near", "wave"]), true, &recent, &liked), Some(2.0 + 2.0 + 0.0 + 2.0));
        // Через 60 дней оценка вдвое меньше.
        assert!((taste_of(&store, "fan", 100 + 60 * DAY) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn reactions_and_weighted_pick() {
        assert_eq!(delta("trackFinished", Some(100.0), 120.0), 1.0);
        assert_eq!(delta("trackFinished", Some(30.0), 120.0), 0.0);
        assert_eq!(delta("skip", Some(10.0), 120.0), -1.0);
        assert_eq!(delta("skip", Some(60.0), 120.0), 0.0);
        assert_eq!(pick_weighted(&[1.0, 3.0], 0.1), Some(0));
        assert_eq!(pick_weighted(&[1.0, 3.0], 0.5), Some(1));
        assert_eq!(pick_weighted(&[], 0.5), None);
    }
}
