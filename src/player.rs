//! Воспроизведение. Файл качается во временный файл, а декодер читает его по мере загрузки:
//! звук начинается сразу, а в памяти трек не лежит.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use aes::cipher::{KeyIvInit, StreamCipher};
use rodio::mixer::MixerSource;
use rodio::{ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, SampleRate, Source};

use crate::api::FileInfo;
use crate::crossfade::{FadeOut, Faded};
use crate::equalizer::{EqSettings, Equalized, Shared};
use crate::stretch::{Clock, Speed, Stretched};

type Ctr128 = ctr::Ctr128BE<aes::Aes128>;
type Ctr256 = ctr::Ctr128BE<aes::Aes256>;

const WAIT: Duration = Duration::from_millis(20);
/// Столько ждём байтов от сервера, прежде чем считать связь оборванной.
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// Пауза между попытками продолжить загрузку или открыть устройство вывода; растёт до потолка.
const RETRY: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(8);

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

pub fn hint(codec: &str) -> &'static str {
    match codec {
        "mp3" => "mp3",
        "flac" => "flac",
        "aac" | "he-aac" => "aac",
        _ => "m4a",
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).timeout_per_read(Some(READ_TIMEOUT)).build().into()
}

/// Тело файла с байта `from` и полная длина файла. Продолжение просится заголовком `Range`.
fn request(agent: &ureq::Agent, url: &str, from: u64) -> Result<(impl Read + use<>, u64), String> {
    let mut req = agent.get(url);
    if from > 0 {
        req = req.header("Range", format!("bytes={from}-"));
    }
    let resp = req.call().map_err(|e| format!("Не удалось получить файл трека: {e}"))?;
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let len = if from > 0 {
        // Ответ 206 с `Content-Range: bytes 100-999/1000`; без него сервер прислал бы файл с начала.
        if resp.status() != 206 {
            return Err("Сервер не продолжил загрузку с места обрыва".into());
        }
        header("content-range").and_then(|v| v.rsplit('/').next()?.parse().ok())
    } else {
        header("content-length").and_then(|v| v.parse().ok())
    }
    .ok_or("Сервер не сообщил размер файла")?;
    Ok((resp.into_body().into_with_config().limit(1024 * 1024 * 1024).reader(), len))
}

/// Перекачивает тело в файл, расшифровывая на ходу. Связь может оборваться посреди файла:
/// тогда загрузка продолжается с того же байта, пока не дойдёт до конца или её не отменят.
/// `written` растёт по мере записи. Вернёт ошибку, только если продолжить нельзя совсем.
fn pump(agent: &ureq::Agent, url: &str, mut body: Box<dyn Read>, len: u64, cipher: &mut Cipher, out: &mut File, written: &AtomicU64, cancelled: &AtomicBool) -> Result<(), String> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut retry = RETRY;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let done = written.load(Ordering::Acquire);
        match body.read(&mut buf) {
            Ok(0) if done >= len => return Ok(()),
            Ok(n) if n > 0 => {
                // Поток CTR непрерывный, поэтому кусок любой длины расшифровывается на месте.
                cipher.apply(&mut buf[..n]);
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                written.fetch_add(n as u64, Ordering::Release);
                retry = RETRY;
                continue;
            }
            // Тело кончилось раньше файла или чтение упало: связь оборвалась.
            _ => {}
        }
        // Ждём сеть и просим остаток файла, пока не выйдет или трек не сменят.
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            std::thread::sleep(retry);
            retry = (retry * 2).min(RETRY_MAX);
            match request(agent, url, done) {
                Ok((rest, _)) => {
                    body = Box::new(rest);
                    break;
                }
                Err(e) if e.starts_with("Сервер не продолжил") => return Err(e),
                Err(_) => {}
            }
        }
    }
}

/// Качает файл в фоне и возвращает его длину, когда известен ответ сервера.
fn download(info: &FileInfo, path: &Path, progress: Arc<Progress>) -> Result<u64, String> {
    let mut cipher = Cipher::new(info.key.as_deref())?;
    let agent = agent();
    let (body, len) = request(&agent, &info.url, 0)?;
    let mut out = File::create(path).map_err(|e| e.to_string())?;
    let url = info.url.clone();
    std::thread::spawn(move || {
        let _ = pump(&agent, &url, Box::new(body) as Box<dyn Read>, len, &mut cipher, &mut out, &progress.written, &progress.cancelled);
        progress.finished.store(true, Ordering::Release);
    });
    Ok(len)
}

/// Качает файл целиком и расшифровывает: так сохраняются скачанные треки. Блокирует поток.
pub fn save_file(info: &FileInfo, path: &Path) -> Result<u64, String> {
    let mut cipher = Cipher::new(info.key.as_deref())?;
    let agent = agent();
    let (body, len) = request(&agent, &info.url, 0)?;
    let mut out = File::create(path).map_err(|e| e.to_string())?;
    let written = AtomicU64::new(0);
    pump(&agent, &info.url, Box::new(body) as Box<dyn Read>, len, &mut cipher, &mut out, &written, &AtomicBool::new(false))?;
    Ok(written.into_inner())
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
    /// Скорость и место играющего трека: при ускорении счётчик вывода отстаёт от трека.
    speed: Speed,
    clock: Arc<Mutex<Option<Clock>>>,
    _device: Arc<Mutex<Option<SendDevice>>>,
    /// Сигнал, что устройство пропало и его надо открыть заново.
    device_lost: Sender<()>,
    levels: Arc<[AtomicU32; 3]>,
}

/// Устройство только держим живым и из других потоков не трогаем.
struct SendDevice(#[allow(dead_code)] MixerDeviceSink);
unsafe impl Send for SendDevice {}
unsafe impl Sync for SendDevice {}

/// Громкость басов (до 250 Гц), середины (500 Гц — 2 кГц) и верхов (2—4 кГц) того, что
/// звучит, — те же полосы, что берёт оригинал для анимации. Фильтры простые, однополюсные:
/// анимации хватает.
struct Bands {
    /// Доля шага фильтра для частот среза 250, 500, 2000 и 4000 Гц.
    steps: [f32; 4],
    /// Доля шага сглаживания энергии.
    smooth: f32,
    filters: [f32; 4],
    energy: [f32; 3],
    /// Сумма каналов текущего кадра и сколько каналов уже пришло.
    frame: f32,
    channel: u16,
    channels: u16,
    frames: u32,
    out: Arc<[AtomicU32; 3]>,
}

/// Уровни обновляются примерно 90 раз в секунду, энергия сглаживается за 30 мс.
const BANDS_EVERY: u32 = 512;
const BANDS_SMOOTH: f32 = 0.03;

impl Bands {
    fn new(channels: ChannelCount, rate: SampleRate, out: Arc<[AtomicU32; 3]>) -> Self {
        let rate = rate.get() as f32;
        let step = |cut: f32| 1.0 - (-std::f32::consts::TAU * cut / rate).exp();
        let smooth = 1.0 - (-1.0 / (BANDS_SMOOTH * rate)).exp();
        Self { steps: [step(250.0), step(500.0), step(2000.0), step(4000.0)], smooth, filters: [0.0; 4], energy: [0.0; 3], frame: 0.0, channel: 0, channels: channels.get(), frames: 0, out }
    }

    fn push(&mut self, sample: f32) {
        self.frame += sample;
        self.channel += 1;
        if self.channel < self.channels {
            return;
        }
        let x = self.frame / self.channels as f32;
        (self.frame, self.channel) = (0.0, 0);
        for (filter, step) in self.filters.iter_mut().zip(self.steps) {
            *filter += step * (x - *filter);
        }
        let [low, from, to, top] = self.filters;
        let smooth = self.smooth;
        for (energy, band) in self.energy.iter_mut().zip([low, to - from, top - to]) {
            *energy += smooth * (band * band - *energy);
        }
        self.frames += 1;
        if self.frames >= BANDS_EVERY {
            self.frames = 0;
            for (out, energy) in self.out.iter().zip(self.energy) {
                out.store(energy.sqrt().to_bits(), Ordering::Relaxed);
            }
        }
    }
}

/// Звук плеера на пути к устройству. Оба вывода играют в свой микшер, а устройство только
/// забирает отсюда сэмплы: при смене устройства новое подхватывает тот же звук с того же места.
#[derive(Clone)]
struct Relay {
    source: Arc<Mutex<(MixerSource, Bands)>>,
    channels: ChannelCount,
    rate: SampleRate,
}

impl Iterator for Relay {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let mut output = self.source.lock().unwrap();
        let (source, bands) = &mut *output;
        // Пустой микшер кончается, а устройство выкинуло бы его навсегда: вместо конца тишина.
        let sample = source.next().unwrap_or(0.0);
        bands.push(sample);
        Some(sample)
    }
}

impl Source for Relay {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Открывает устройство вывода по умолчанию. Если оно пропадёт, `lost` получит сигнал.
fn open_device(lost: Sender<()>) -> Result<MixerDeviceSink, String> {
    let on_error = move |e: rodio::cpal::Error| {
        // Смену устройства по умолчанию Windows проводит сама и только сообщает о ней.
        if matches!(e.kind(), rodio::cpal::ErrorKind::DeviceNotAvailable | rodio::cpal::ErrorKind::StreamInvalidated) {
            let _ = lost.send(());
        }
    };
    let mut device = DeviceSinkBuilder::from_default_device()
        .map_err(|e| format!("Нет устройства вывода звука: {e}"))?
        .with_error_callback(on_error)
        .open_sink_or_fallback()
        .map_err(|e| format!("Нет устройства вывода звука: {e}"))?;
    device.log_on_drop(false);
    Ok(device)
}

/// Ждёт, пока устройство пропадёт, и открывает новое, как только оно появится; звук продолжается
/// с того же места. Кончается вместе с плеером.
fn watch_device(slot: Weak<Mutex<Option<SendDevice>>>, relay: Relay, lost: mpsc::Receiver<()>, signal: Sender<()>) {
    std::thread::spawn(move || loop {
        match lost.recv_timeout(RETRY_MAX) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) if slot.strong_count() > 0 => continue,
            Err(_) => return,
        }
        let mut retry = RETRY;
        loop {
            let Some(slot) = slot.upgrade() else { return };
            // Старый поток закрываем раньше, чем открываем новый: звук не должен уйти в оба.
            slot.lock().unwrap().take();
            while lost.try_recv().is_ok() {}
            if let Ok(device) = open_device(signal.clone()) {
                device.mixer().add(relay.clone());
                *slot.lock().unwrap() = Some(SendDevice(device));
                break;
            }
            drop(slot);
            std::thread::sleep(retry);
            retry = (retry * 2).min(RETRY_MAX);
        }
    });
}

impl Player {
    pub fn new() -> Result<Self, String> {
        let (signal, lost) = mpsc::channel();
        let device = open_device(signal.clone())?;
        let (channels, rate) = (device.config().channel_count(), device.config().sample_rate());
        let (mixer, source) = rodio::mixer::mixer(channels, rate);
        let levels: Arc<[AtomicU32; 3]> = Arc::default();
        let relay = Relay { source: Arc::new(Mutex::new((source, Bands::new(channels, rate, levels.clone())))), channels, rate };
        device.mixer().add(relay.clone());
        let outputs = [rodio::Player::connect_new(&mixer), rodio::Player::connect_new(&mixer)];
        let slot = Arc::new(Mutex::new(Some(SendDevice(device))));
        watch_device(Arc::downgrade(&slot), relay, lost, signal.clone());
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
            speed: Speed::default(),
            clock: Arc::default(),
            _device: slot,
            device_lost: signal,
            levels,
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
        // HE-AAC целиком понимает только декодер Windows, остальное — symphonia внутри rodio.
        let decoder: Box<dyn Source + Send> = if codec.starts_with("he-aac") {
            Box::new(crate::heaac::HeAac::new(reader, len)?)
        } else {
            Box::new(
                Decoder::builder()
                    .with_data(reader)
                    .with_byte_len(len)
                    .with_seekable(true)
                    .with_hint(hint(codec))
                    .build()
                    .map_err(|e| format!("Не удалось прочитать файл трека: {e}"))?,
            )
        };
        // Пока декодер открывался, могли включить другой трек.
        if self.generation.load(Ordering::Acquire) != generation {
            return Ok(());
        }
        let fade_in = self.fade_in.lock().unwrap().take();
        let fade_out = FadeOut::default();
        *self.fade_out.lock().unwrap() = Some(fade_out.clone());
        let output = self.output();
        let clock = Clock::default();
        *self.clock.lock().unwrap() = Some(clock.clone());
        output.append(Stretched::new(Faded::new(Equalized::new(decoder, self.eq.clone()), fade_in, fade_out), self.speed.clone(), clock));
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

    /// Открывает устройство вывода заново, как после его пропажи. Нужно только для проверки.
    pub fn reopen_device(&self) {
        let _ = self.device_lost.send(());
    }

    /// Громкость басов, середины и верхов того, что звучит сейчас, после громкости плеера.
    pub fn levels(&self) -> [f32; 3] {
        std::array::from_fn(|band| f32::from_bits(self.levels[band].load(Ordering::Relaxed)))
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
        match self.clock.lock().unwrap().as_ref() {
            Some(clock) if !self.output().empty() => clock.get(),
            _ => self.output().get_pos(),
        }
    }

    /// Скорость слышна сразу, в том числе в играющем треке; высота голоса не меняется.
    pub fn set_speed(&self, speed: f32) {
        self.speed.set(speed);
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
    fn download_resumes_after_drop() {
        use std::io::BufRead;
        use std::net::TcpListener;
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/track", listener.local_addr().unwrap());
        let served = data.clone();
        let server = std::thread::spawn(move || {
            let mut ranges = Vec::new();
            // Первый ответ обрывается на 400-м байте, второй отдаёт остаток по `Range`.
            for cut in [Some(400), None] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut lines = io::BufReader::new(stream.try_clone().unwrap()).lines();
                let range = lines.by_ref().map_while(|l| l.ok()).take_while(|l| !l.is_empty()).find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(|r| r.trim_end_matches('-').parse::<usize>().unwrap()));
                ranges.push(range);
                let from = range.unwrap_or(0);
                let head = match range {
                    Some(_) => format!("HTTP/1.1 206 Partial Content
Content-Length: {}
Content-Range: bytes {from}-999/1000

", 1000 - from),
                    None => "HTTP/1.1 200 OK
Content-Length: 1000

".to_string(),
                };
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&served[from..cut.unwrap_or(1000)]).unwrap();
            }
            ranges
        });
        let path = std::env::temp_dir().join(format!("tune-native-resume-{}", std::process::id()));
        let info = FileInfo { url, key: None, codec: "mp3".into(), bitrate: 0, quality: String::new() };
        assert_eq!(save_file(&info, &path).unwrap(), 1000);
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(server.join().unwrap(), [None, Some(400)]);
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
