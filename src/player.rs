//! Воспроизведение. Файл качается во временный файл, а декодер читает его по мере загрузки:
//! звук начинается сразу, а в памяти трек не лежит.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes::cipher::{KeyIvInit, StreamCipher};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink};

use crate::api::FileInfo;
use crate::crossfade::{FadeOut, Faded};
use crate::equalizer::{EqSettings, Equalized, Shared};

type Ctr128 = ctr::Ctr128BE<aes::Aes128>;
type Ctr256 = ctr::Ctr128BE<aes::Aes256>;

const WAIT: Duration = Duration::from_millis(20);

enum Cipher {
    None,
    Aes128(Box<Ctr128>),
    Aes256(Box<Ctr256>),
}

impl Cipher {
    /// Файл с transport=encraw зашифрован AES-CTR с нулевым счётчиком.
    fn new(key: Option<&str>) -> Result<Self, String> {
        let Some(hex) = key else { return Ok(Cipher::None) };
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
            .collect::<Option<Vec<u8>>>()
            .ok_or("Неверный ключ файла")?;
        let iv = [0u8; 16];
        match bytes.len() {
            16 => Ok(Cipher::Aes128(Box::new(Ctr128::new_from_slices(&bytes, &iv).map_err(|e| e.to_string())?))),
            32 => Ok(Cipher::Aes256(Box::new(Ctr256::new_from_slices(&bytes, &iv).map_err(|e| e.to_string())?))),
            _ => Err("Неверный ключ файла".into()),
        }
    }

    fn apply(&mut self, buf: &mut [u8]) {
        match self {
            Cipher::None => {}
            Cipher::Aes128(c) => c.apply_keystream(buf),
            Cipher::Aes256(c) => c.apply_keystream(buf),
        }
    }
}

#[derive(Default)]
struct Progress {
    written: AtomicU64,
    /// Загрузка закончилась: успешно, с ошибкой или её отменили.
    finished: AtomicBool,
    cancelled: AtomicBool,
}

/// Читает временный файл, ожидая байты, которые ещё не докачались.
struct Reader {
    file: File,
    pos: u64,
    len: u64,
    progress: Arc<Progress>,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let written = self.progress.written.load(Ordering::Acquire);
            if self.progress.cancelled.load(Ordering::Acquire) {
                return Ok(0);
            }
            if written > self.pos {
                let max = buf.len().min((written - self.pos) as usize);
                let n = self.file.read(&mut buf[..max])?;
                self.pos += n as u64;
                return Ok(n);
            }
            if self.pos >= self.len || self.progress.finished.load(Ordering::Acquire) {
                return Ok(0);
            }
            std::thread::sleep(WAIT);
        }
    }
}

impl Seek for Reader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let pos = match from {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.len as i64 + d,
        };
        let pos = u64::try_from(pos).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        self.pos = self.file.seek(SeekFrom::Start(pos))?;
        Ok(self.pos)
    }
}

fn hint(codec: &str) -> &'static str {
    match codec {
        "mp3" => "mp3",
        "flac" => "flac",
        "aac" | "he-aac" => "aac",
        _ => "m4a",
    }
}

/// Качает файл в фоне и возвращает его длину, когда известен ответ сервера.
fn download(info: &FileInfo, path: &Path, progress: Arc<Progress>) -> Result<u64, String> {
    let mut cipher = Cipher::new(info.key.as_deref())?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).build().into();
    let resp = agent.get(&info.url).call().map_err(|e| format!("Не удалось получить файл трека: {e}"))?;
    let len = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or("Сервер не сообщил размер файла")?;
    let mut out = File::create(path).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let mut body = resp.into_body().into_with_config().limit(1024 * 1024 * 1024).reader();
        let mut buf = vec![0u8; 64 * 1024];
        while !progress.cancelled.load(Ordering::Acquire) {
            let Ok(n) = body.read(&mut buf) else { break };
            if n == 0 {
                break;
            }
            // Поток CTR непрерывный, поэтому кусок любой длины расшифровывается на месте.
            cipher.apply(&mut buf[..n]);
            if out.write_all(&buf[..n]).is_err() {
                break;
            }
            progress.written.fetch_add(n as u64, Ordering::Release);
        }
        progress.finished.store(true, Ordering::Release);
    });
    Ok(len)
}

/// Качает файл целиком и расшифровывает: так сохраняются скачанные треки. Блокирует поток.
pub fn save_file(info: &FileInfo, path: &Path) -> Result<u64, String> {
    let mut cipher = Cipher::new(info.key.as_deref())?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).build().into();
    let resp = agent.get(&info.url).call().map_err(|e| format!("Не удалось получить файл трека: {e}"))?;
    let mut body = resp.into_body().into_with_config().limit(1024 * 1024 * 1024).reader();
    let mut out = File::create(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("Загрузка оборвалась: {e}"))?;
        if n == 0 {
            break;
        }
        cipher.apply(&mut buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        total += n as u64;
    }
    Ok(total)
}

/// Следующий трек, который уже начал качаться.
struct Prepared {
    key: String,
    codec: String,
    sound: crate::api::Sound,
    path: PathBuf,
    progress: Arc<Progress>,
    len: u64,
}

impl Prepared {
    fn discard(self) {
        self.progress.cancelled.store(true, Ordering::Release);
        let _ = std::fs::remove_file(self.path);
    }
}

/// Устройство вывода и плеер. Клонируется в фоновые задачи.
#[derive(Clone)]
pub struct Player {
    /// Два вывода: на переходе старый трек доигрывает на одном, новый начинается на другом.
    outputs: Arc<[rodio::Player; 2]>,
    active: Arc<AtomicUsize>,
    /// Затухание играющего трека и нарастание следующего.
    fade_out: Arc<Mutex<Option<FadeOut>>>,
    fade_in: Arc<Mutex<Option<(f32, f32)>>>,
    /// Загрузка и файл трека, который затихает после перехода.
    fading: Arc<Mutex<Option<(Option<Arc<Progress>>, Option<PathBuf>)>>>,
    current: Arc<Mutex<Option<Arc<Progress>>>>,
    /// Файл играющего трека, если он был подготовлен заранее.
    current_file: Arc<Mutex<Option<PathBuf>>>,
    prepared: Arc<Mutex<Option<Prepared>>>,
    generation: Arc<AtomicU64>,
    /// Следующий трек откроется на паузе: его ещё надо перемотать или он и был на паузе.
    hold: Arc<AtomicBool>,
    /// Эквалайзер, через который идёт каждый трек.
    eq: Shared,
    _device: Arc<SendDevice>,
}

/// Устройство только держим живым и из других потоков не трогаем.
struct SendDevice(#[allow(dead_code)] MixerDeviceSink);
unsafe impl Send for SendDevice {}
unsafe impl Sync for SendDevice {}

impl Player {
    pub fn new() -> Result<Self, String> {
        let mut device = DeviceSinkBuilder::open_default_sink().map_err(|e| format!("Нет устройства вывода звука: {e}"))?;
        device.log_on_drop(false);
        let outputs = [rodio::Player::connect_new(device.mixer()), rodio::Player::connect_new(device.mixer())];
        Ok(Self {
            outputs: Arc::new(outputs),
            active: Arc::default(),
            fade_out: Arc::default(),
            fade_in: Arc::default(),
            fading: Arc::default(),
            current: Arc::default(),
            current_file: Arc::default(),
            prepared: Arc::default(),
            generation: Arc::default(),
            hold: Arc::default(),
            eq: Shared::default(),
            _device: Arc::new(SendDevice(device)),
        })
    }

    fn temp_path(generation: u64) -> PathBuf {
        std::env::temp_dir().join(format!("tune-native-{}-{generation}.part", std::process::id()))
    }

    /// Вывод играющего трека.
    fn output(&self) -> &rodio::Player {
        &self.outputs[self.active.load(Ordering::Acquire)]
    }

    /// Убирает трек, который затихал после перехода, и его файл.
    fn clear_fading(&self) {
        self.outputs[1 - self.active.load(Ordering::Acquire)].clear();
        if let Some((progress, file)) = self.fading.lock().unwrap().take() {
            if let Some(progress) = progress {
                progress.cancelled.store(true, Ordering::Release);
            }
            if let Some(file) = file {
                let _ = std::fs::remove_file(file);
            }
        }
    }

    /// Останавливает текущий трек и отменяет его загрузку. Вернёт номер нового запуска.
    pub fn stop(&self) -> u64 {
        self.clear_fading();
        self.fade_in.lock().unwrap().take();
        self.output().clear();
        if let Some(old) = self.current.lock().unwrap().take() {
            old.cancelled.store(true, Ordering::Release);
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = std::fs::remove_file(Self::temp_path(generation - 1));
        if let Some(file) = self.current_file.lock().unwrap().take() {
            let _ = std::fs::remove_file(file);
        }
        generation
    }

    /// Начинает переход: играющий трек затихает между `out` и там кончается, следующий
    /// запуск нарастает между `fade_in` на другом выводе. Вернёт номер нового запуска.
    pub fn crossfade(&self, out: (f32, f32), fade_in: (f32, f32)) -> u64 {
        self.clear_fading();
        if let Some(fade) = self.fade_out.lock().unwrap().take() {
            fade.arm(out.0, out.1);
        }
        let generation = self.generation.load(Ordering::Acquire);
        let progress = self.current.lock().unwrap().take();
        let file = self.current_file.lock().unwrap().take().unwrap_or_else(|| Self::temp_path(generation));
        *self.fading.lock().unwrap() = Some((progress, Some(file)));
        self.active.fetch_xor(1, Ordering::AcqRel);
        self.output().clear();
        *self.fade_in.lock().unwrap() = Some(fade_in);
        self.hold.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Блокирует поток до начала звука, поэтому зовётся в фоне.
    pub fn play(&self, generation: u64, info: &FileInfo) -> Result<(), String> {
        let path = Self::temp_path(generation);
        let progress = Arc::new(Progress::default());
        *self.current.lock().unwrap() = Some(progress.clone());
        let len = download(info, &path, progress.clone())?;
        self.start(generation, &path, progress, len, &info.codec)
    }

    /// Играет скачанный файл: он уже целиком на диске и после трека не удаляется.
    pub fn play_file(&self, generation: u64, path: &Path, codec: &str) -> Result<(), String> {
        let len = std::fs::metadata(path).map_err(|_| "Скачанный файл пропал".to_string())?.len();
        let progress = Arc::new(Progress::default());
        progress.written.store(len, Ordering::Release);
        progress.finished.store(true, Ordering::Release);
        *self.current.lock().unwrap() = Some(progress.clone());
        self.start(generation, path, progress, len, codec)
    }

    /// Начинает качать следующий трек заранее, чтобы он зазвучал без паузы.
    pub fn preload(&self, key: &str, info: &FileInfo) -> Result<(), String> {
        let path = std::env::temp_dir().join(format!("tune-native-{}-next-{}.part", std::process::id(), key.replace(':', "-")));
        let progress = Arc::new(Progress::default());
        let len = download(info, &path, progress.clone())?;
        let new = Prepared { key: key.to_string(), codec: info.codec.clone(), sound: info.sound(), path, progress, len };
        if let Some(old) = self.prepared.lock().unwrap().replace(new) {
            old.discard();
        }
        Ok(())
    }

    /// Запускает подготовленный файл. `None` — такого нет, файл надо запросить обычным путём.
    pub fn play_prepared(&self, generation: u64, key: &str) -> Option<Result<crate::api::Sound, String>> {
        let prepared = {
            let mut slot = self.prepared.lock().unwrap();
            if slot.as_ref().is_none_or(|p| p.key != key) {
                return None;
            }
            slot.take()?
        };
        *self.current.lock().unwrap() = Some(prepared.progress.clone());
        *self.current_file.lock().unwrap() = Some(prepared.path.clone());
        Some(self.start(generation, &prepared.path, prepared.progress, prepared.len, &prepared.codec).map(|()| prepared.sound))
    }

    fn start(&self, generation: u64, path: &Path, progress: Arc<Progress>, len: u64, codec: &str) -> Result<(), String> {
        let reader = Reader { file: File::open(path).map_err(|e| e.to_string())?, pos: 0, len, progress };
        let decoder = Decoder::builder()
            .with_data(reader)
            .with_byte_len(len)
            .with_seekable(true)
            .with_hint(hint(codec))
            .build()
            .map_err(|e| format!("Не удалось прочитать файл трека: {e}"))?;
        // Пока декодер открывался, могли включить другой трек.
        if self.generation.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        let fade_in = self.fade_in.lock().unwrap().take();
        let fade_out = FadeOut::default();
        *self.fade_out.lock().unwrap() = Some(fade_out.clone());
        let output = self.output();
        output.append(Faded::new(Equalized::new(decoder, self.eq.clone()), fade_in, fade_out));
        // Если переход начинается не с нуля, новый трек сразу встаёт на свою точку.
        if let Some((start, _)) = fade_in.filter(|(start, _)| *start > 0.05) {
            let _ = output.try_seek(Duration::from_secs_f32(start));
        }
        if self.hold.swap(false, Ordering::AcqRel) {
            output.pause();
        } else {
            output.play();
        }
        Ok(())
    }

    pub fn hold_next(&self, hold: bool) {
        self.hold.store(hold, Ordering::Release);
    }

    /// Пауза останавливает и затихающий трек, чтобы переход продолжился с того же места.
    pub fn toggle(&self) {
        let paused = self.output().is_paused();
        for output in self.outputs.iter() {
            if paused {
                output.play();
            } else {
                output.pause();
            }
        }
    }

    pub fn is_paused(&self) -> bool {
        self.output().is_paused()
    }

    /// Трек после перехода ещё затихает.
    pub fn fading(&self) -> bool {
        !self.outputs[1 - self.active.load(Ordering::Acquire)].empty()
    }

    pub fn is_empty(&self) -> bool {
        self.output().empty()
    }

    pub fn position(&self) -> Duration {
        self.output().get_pos()
    }

    /// Новые настройки эквалайзера слышны сразу, в том числе в играющем треке.
    pub fn set_eq(&self, settings: EqSettings) {
        self.eq.set(settings);
    }

    pub fn set_volume(&self, volume: f32) {
        for output in self.outputs.iter() {
            output.set_volume(volume);
        }
    }

    /// Ждёт, пока докачается нужное место, поэтому зовётся в фоне.
    pub fn seek(&self, position: Duration) -> Result<(), String> {
        self.output().try_seek(position).map_err(|e| format!("Не удалось перемотать: {e}"))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Последняя копия: убираем временный файл.
        if Arc::strong_count(&self.outputs) == 1 {
            self.stop();
            if let Some(prepared) = self.prepared.lock().unwrap().take() {
                prepared.discard();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_waits_and_stops_on_cancel() {
        let path = std::env::temp_dir().join(format!("tune-native-test-{}", std::process::id()));
        std::fs::write(&path, b"abcdef").unwrap();
        let progress = Arc::new(Progress::default());
        progress.written.store(3, Ordering::Release);
        let mut reader = Reader { file: File::open(&path).unwrap(), pos: 0, len: 6, progress: progress.clone() };
        let mut buf = [0u8; 6];
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"abc");
        progress.cancelled.store(true, Ordering::Release);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
        drop(reader);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn ctr_roundtrip() {
        let key = "000102030405060708090a0b0c0d0e0f";
        let mut data = b"hello tune native".to_vec();
        let orig = data.clone();
        let mut a = Cipher::new(Some(key)).unwrap();
        a.apply(&mut data[..5]);
        a.apply(&mut data[5..]);
        assert_ne!(data, orig);
        Cipher::new(Some(key)).unwrap().apply(&mut data);
        assert_eq!(data, orig);
    }
}
