//! Видео клипов: декодирует и сводит звук с картинкой libmpv (`libmpv-2.dll` рядом с
//! программой), а кадр рисует наш шейдер прямо в окне. Поэтому видео — обычная часть
//! интерфейса: поверх него ложатся кнопки, меню и подсказки, оно сдвигается и
//! обрезается вместе со страницей.
//!
//! Библиотека подгружается при первом клипе: без неё Tune работает как раньше, а клип
//! открывается в браузере.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use iced::widget::shader::{self, Viewport};
use iced::{mouse, wgpu, Rectangle};

/// Имя библиотеки mpv: сборка shinchiro для Windows.
pub const LIBRARY: &str = "libmpv-2.dll";

/// Шире кадр не рисуем: 4K в клипах нет, а больший кадр только грузит процессор.
const MAX_WIDTH: i64 = 1920;
const MAX_HEIGHT: i64 = 1080;

const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;
const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_SW_SIZE: c_int = 17;
const PARAM_SW_FORMAT: c_int = 18;
const PARAM_SW_STRIDE: c_int = 19;
const PARAM_SW_POINTER: c_int = 20;
const UPDATE_FRAME: u64 = 1;

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    reply: u64,
    data: *mut c_void,
}

#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

type Handle = *mut c_void;
type Context = *mut c_void;

/// Нужные функции libmpv.
struct Lib {
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    get_property: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    terminate_destroy: unsafe extern "C" fn(Handle),
    wait_event: unsafe extern "C" fn(Handle, f64) -> *mut Event,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    render_create: unsafe extern "C" fn(*mut Context, Handle, *mut RenderParam) -> c_int,
    render_set_update_callback: unsafe extern "C" fn(Context, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    render_update: unsafe extern "C" fn(Context) -> u64,
    render_render: unsafe extern "C" fn(Context, *mut RenderParam) -> c_int,
    render_free: unsafe extern "C" fn(Context),
}

fn load() -> Result<&'static Lib, String> {
    static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();
    LIB.get_or_init(|| unsafe { open_library() }).as_ref().map_err(Clone::clone)
}

/// Есть ли рядом с программой видеоплеер.
pub fn available() -> bool {
    load().is_ok()
}

unsafe fn open_library() -> Result<Lib, String> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS};
    // Только из папки программы: чужая библиотека с тем же именем из PATH не подхватится.
    let dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(LIBRARY))).ok_or("Не найдена папка программы")?;
    let wide: Vec<u16> = dir.as_os_str().encode_wide_null();
    let module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if module.is_null() {
        return Err(format!("Нет видеоплеера: рядом с Tune не найден {LIBRARY}"));
    }
    macro_rules! get {
        ($name:literal) => {{
            let f = GetProcAddress(module, concat!($name, "\0").as_ptr()).ok_or(concat!("В видеоплеере нет ", $name))?;
            std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(f)
        }};
    }
    Ok(Lib {
        create: get!("mpv_create"),
        initialize: get!("mpv_initialize"),
        set_option_string: get!("mpv_set_option_string"),
        set_property_string: get!("mpv_set_property_string"),
        get_property: get!("mpv_get_property"),
        command: get!("mpv_command"),
        terminate_destroy: get!("mpv_terminate_destroy"),
        wait_event: get!("mpv_wait_event"),
        error_string: get!("mpv_error_string"),
        render_create: get!("mpv_render_context_create"),
        render_set_update_callback: get!("mpv_render_context_set_update_callback"),
        render_update: get!("mpv_render_context_update"),
        render_render: get!("mpv_render_context_render"),
        render_free: get!("mpv_render_context_free"),
    })
}

trait WideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl WideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

/// Последний готовый кадр: `rgb0`, строки по `stride` байт.
#[derive(Default)]
struct Frame {
    width: u32,
    height: u32,
    stride: usize,
    pixels: Vec<u8>,
}

struct Inner {
    lib: &'static Lib,
    mpv: Handle,
    render: Mutex<Context>,
    frame: Mutex<Frame>,
    /// Номер кадра: видеокарта перекладывает картинку, только когда он сменился.
    serial: AtomicU64,
    wake: (Mutex<bool>, Condvar),
    stop: AtomicBool,
    ended: AtomicBool,
    loaded: AtomicBool,
    failed: Mutex<Option<String>>,
    /// Живой фон темы: кадр поменьше, а без показа на экране плеер встаёт на паузу.
    backdrop: bool,
    /// Когда фон последний раз рисовался, в миллисекундах от `epoch()`.
    seen: AtomicU64,
    /// Фон остановлен настройкой: стоит кадр, а стекло панелей всё равно его размывает.
    hold: AtomicBool,
}

// mpv сам защищает свой объект, кадр — под замком.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

impl Inner {
    fn get<T: Default>(&self, name: &str, format: c_int) -> Option<T> {
        let name = CString::new(name).ok()?;
        let mut value = T::default();
        let code = unsafe { (self.lib.get_property)(self.mpv, name.as_ptr(), format, &mut value as *mut T as *mut c_void) };
        (code >= 0).then_some(value)
    }

    fn set(&self, name: &str, value: &str) {
        if let (Ok(name), Ok(value)) = (CString::new(name), CString::new(value)) {
            unsafe { (self.lib.set_property_string)(self.mpv, name.as_ptr(), value.as_ptr()) };
        }
    }

    fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().filter_map(|arg| CString::new(*arg).ok()).collect();
        let mut pointers: Vec<*const c_char> = owned.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());
        unsafe { (self.lib.command)(self.mpv, pointers.as_ptr()) };
    }

    fn wake(&self) {
        let (lock, signal) = &self.wake;
        *lock.lock().unwrap() = true;
        signal.notify_one();
    }

    fn error(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.lib.error_string)(code)) }.to_string_lossy().into_owned()
    }
}

unsafe extern "C" fn on_update(data: *mut c_void) {
    // Вызывается из потока mpv: только будим свой поток, рисовать здесь нельзя.
    let inner = &*(data as *const Inner);
    inner.wake();
}

/// Что сейчас с видео.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub time: f32,
    pub duration: f32,
    pub paused: bool,
    /// Ждёт данных из сети.
    pub buffering: bool,
    /// Файл открылся и пошёл первый кадр.
    pub loaded: bool,
    pub ended: bool,
}

/// Играющий клип. Клон — тот же плеер; плеер закрывается, когда уходит последний.
#[derive(Clone)]
pub struct Video {
    inner: Arc<Inner>,
    _thread: Arc<Worker>,
}

impl std::fmt::Debug for Video {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Video")
    }
}

impl PartialEq for Video {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Поток отрисовки: при закрытии останавливается и освобождает mpv.
struct Worker {
    inner: Arc<Inner>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.wake();
        if let Some(handle) = self.handle.lock().unwrap().take() {
            let _ = handle.join();
        }
        let lib = self.inner.lib;
        let render = std::mem::replace(&mut *self.inner.render.lock().unwrap(), std::ptr::null_mut());
        unsafe {
            if !render.is_null() {
                (lib.render_free)(render);
            }
            (lib.terminate_destroy)(self.inner.mpv);
        }
    }
}

impl Video {
    /// Открывает поток и начинает играть. `volume` — от 0 до 1, как у музыки.
    pub fn open(url: &str, volume: f32, start: f32) -> Result<Video, String> {
        Self::start(url, volume, start, false)
    }

    /// Живой фон темы: файл по кругу, без звука. Пока фон не виден (окно свёрнуто или в
    /// трее), видео стоит на паузе и не грузит процессор.
    pub fn backdrop(path: &str) -> Result<Video, String> {
        Self::start(path, 0.0, 0.0, true)
    }

    fn start(url: &str, volume: f32, start: f32, backdrop: bool) -> Result<Video, String> {
        let lib = load()?;
        let mpv = unsafe { (lib.create)() };
        if mpv.is_null() {
            return Err("Видеоплеер не запустился".to_string());
        }
        let option = |name: &str, value: &str| {
            let (name, value) = (CString::new(name).unwrap(), CString::new(value).unwrap());
            unsafe { (lib.set_option_string)(mpv, name.as_ptr(), value.as_ptr()) };
        };
        // Картинку берём себе, своего окна у mpv нет. Декодирует видеокарта, кадр копируется в память.
        option("vo", "libmpv");
        // Декодирует процессор: кадры всё равно идут через память, а второй графический
        // интерфейс (D3D11) рядом с отрисовкой окна не нужен драйверу видеокарты.
        option("hwdec", "no");
        option("sw-fast", "yes");
        option("keep-open", "yes");
        option("idle", "yes");
        option("ytdl", "no");
        option("osc", "no");
        option("input-default-bindings", "no");
        option("terminal", "no");
        option("audio-client-name", "Tune");
        // Лучшее качество HLS, запас в сети на полминуты.
        option("hls-bitrate", "max");
        option("cache", "yes");
        option("demuxer-readahead-secs", "30");
        option("user-agent", crate::api::BROWSER);
        option("volume", &format!("{:.0}", (volume.clamp(0.0, 1.0) * 100.0)));
        // Журнал mpv для разбора ошибок: TUNE_MPV_LOG=путь.
        if let Some(path) = std::env::var_os("TUNE_MPV_LOG") {
            option("log-file", &path.to_string_lossy());
        }
        if start > 0.0 {
            option("start", &format!("{start:.1}"));
        }
        if backdrop {
            option("aid", "no");
            option("mute", "yes");
            option("loop-file", "inf");
            // Фон идёт всё время. Декодирует процессор: копировать кадр с видеокарты в память
            // выходило втрое дороже, чем декодировать самому (замер: 15% ядра против 50%).
            option("vd-lavc-skiploopfilter", "all");
            // Кадры переводятся в картинку процессором: 24 в секунду и быстрый масштаб вдвое дешевле.
            option("vf", "fps=24");
            option("sws-scaler", "fast-bilinear");
            option("audio-client-name", "Tune: фон");
        }
        let code = unsafe { (lib.initialize)(mpv) };
        if code < 0 {
            unsafe { (lib.terminate_destroy)(mpv) };
            return Err("Видеоплеер не запустился".to_string());
        }
        let inner = Arc::new(Inner {
            lib,
            mpv,
            render: Mutex::new(std::ptr::null_mut()),
            frame: Mutex::new(Frame::default()),
            serial: AtomicU64::new(0),
            wake: (Mutex::new(false), Condvar::new()),
            stop: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            loaded: AtomicBool::new(false),
            failed: Mutex::new(None),
            backdrop,
            seen: AtomicU64::new(epoch()),
            hold: AtomicBool::new(false),
        });
        let api = c"sw";
        let mut params = [RenderParam { kind: PARAM_API_TYPE, data: api.as_ptr() as *mut c_void }, RenderParam { kind: PARAM_INVALID, data: std::ptr::null_mut() }];
        let mut render: Context = std::ptr::null_mut();
        let code = unsafe { (lib.render_create)(&mut render, mpv, params.as_mut_ptr()) };
        if code < 0 {
            let error = inner.error(code);
            unsafe { (lib.terminate_destroy)(mpv) };
            return Err(format!("Видеоплеер не запустился: {error}"));
        }
        *inner.render.lock().unwrap() = render;
        unsafe { (lib.render_set_update_callback)(render, Some(on_update), Arc::as_ptr(&inner) as *mut c_void) };
        let worker = Arc::new(Worker { inner: inner.clone(), handle: Mutex::new(None) });
        let thread_inner = inner.clone();
        *worker.handle.lock().unwrap() = Some(std::thread::Builder::new().name("video".into()).spawn(move || run(&thread_inner)).map_err(|e| e.to_string())?);
        inner.command(&["loadfile", url]);
        Ok(Video { inner, _thread: worker })
    }

    pub fn state(&self) -> State {
        let inner = &self.inner;
        State {
            time: inner.get::<f64>("time-pos", FORMAT_DOUBLE).unwrap_or(0.0) as f32,
            duration: inner.get::<f64>("duration", FORMAT_DOUBLE).unwrap_or(0.0) as f32,
            paused: inner.get::<c_int>("pause", FORMAT_FLAG).unwrap_or(0) != 0,
            buffering: inner.get::<c_int>("paused-for-cache", FORMAT_FLAG).unwrap_or(0) != 0 || !inner.loaded.load(Ordering::SeqCst),
            loaded: inner.loaded.load(Ordering::SeqCst),
            ended: inner.ended.load(Ordering::SeqCst) || inner.get::<c_int>("eof-reached", FORMAT_FLAG).unwrap_or(0) != 0,
        }
    }

    /// Ошибка открытия потока, если была.
    pub fn failed(&self) -> Option<String> {
        self.inner.failed.lock().unwrap().clone()
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.set("pause", if paused { "yes" } else { "no" });
    }

    pub fn seek(&self, seconds: f32) {
        self.inner.ended.store(false, Ordering::SeqCst);
        self.inner.command(&["seek", &format!("{:.2}", seconds.max(0.0)), "absolute+keyframes"]);
    }

    /// С начала: после конца клипа.
    pub fn restart(&self) {
        self.seek(0.0);
        self.set_paused(false);
    }

    pub fn set_volume(&self, volume: f32) {
        self.inner.set("volume", &format!("{:.0}", volume.clamp(0.0, 1.0) * 100.0));
    }

    pub fn set_speed(&self, speed: f32) {
        self.inner.set("speed", &format!("{speed:.2}"));
    }

    /// Другое качество того же клипа: поток меняется, место остаётся.
    pub fn switch(&self, url: &str, at: f32) {
        self.inner.ended.store(false, Ordering::SeqCst);
        self.inner.command(&["loadfile", url, "replace", "-1", &format!("start={at:.2}")]);
    }

    /// Остановить живой фон на текущем кадре или пустить дальше.
    pub fn set_hold(&self, hold: bool) {
        self.inner.hold.store(hold, Ordering::Relaxed);
        self.inner.wake();
    }

    /// Номер последнего кадра: меняется, когда пришла новая картинка.
    pub fn serial(&self) -> u64 {
        self.inner.serial.load(Ordering::SeqCst)
    }

    /// Виджет с картинкой клипа. Края скругляются на `radius`.
    pub fn view(&self, radius: f32) -> View {
        View { inner: self.inner.clone(), radius }
    }
}

/// Буфер под кадр с началом на границе 64 байт: так mpv переводит цвета быстрее.
struct Aligned {
    bytes: Vec<u8>,
    offset: usize,
}

impl Aligned {
    fn new(size: usize) -> Self {
        let bytes = vec![0u8; size + 64];
        let offset = (64 - (bytes.as_ptr() as usize % 64)) % 64;
        Self { bytes, offset }
    }

    fn slice(&mut self, size: usize) -> &mut [u8] {
        &mut self.bytes[self.offset..self.offset + size]
    }
}

/// Поток отрисовки: ждёт сигнала от mpv, разбирает события и рисует кадр в память.
fn run(inner: &Inner) {
    let mut buffer: Option<(usize, Aligned)> = None;
    let mut asleep = false;
    loop {
        // Фон давно не рисовался — его не видно: пауза, пока не покажут снова.
        if inner.backdrop {
            let hidden = epoch().saturating_sub(inner.seen.load(Ordering::Relaxed)) > BACKDROP_HIDDEN_MS || inner.hold.load(Ordering::Relaxed);
            if hidden != asleep {
                asleep = hidden;
                inner.set("pause", if hidden { "yes" } else { "no" });
            }
        }
        {
            let (lock, signal) = &inner.wake;
            let mut woken = lock.lock().unwrap();
            if !*woken {
                woken = signal.wait_timeout(woken, Duration::from_millis(100)).unwrap().0;
            }
            *woken = false;
        }
        if inner.stop.load(Ordering::SeqCst) {
            return;
        }
        loop {
            let event = unsafe { &*(inner.lib.wait_event)(inner.mpv, 0.0) };
            match event.id {
                EVENT_NONE => break,
                EVENT_SHUTDOWN => return,
                EVENT_FILE_LOADED => inner.loaded.store(true, Ordering::SeqCst),
                EVENT_END_FILE => {
                    // 0 — доиграл, 4 — ошибка. Смена качества тоже закрывает файл (2), но это не конец.
                    match unsafe { (event.data as *const EndFile).as_ref() } {
                        Some(end) if end.reason == 4 => {
                            *inner.failed.lock().unwrap() = Some(format!("Клип не открылся: {}", inner.error(end.error)));
                            inner.ended.store(true, Ordering::SeqCst);
                        }
                        Some(end) if end.reason == 0 => inner.ended.store(true, Ordering::SeqCst),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let render = *inner.render.lock().unwrap();
        if render.is_null() {
            return;
        }
        let flags = unsafe { (inner.lib.render_update)(render) };
        if flags & UPDATE_FRAME == 0 {
            continue;
        }
        let width = inner.get::<i64>("dwidth", FORMAT_INT64).unwrap_or(0);
        let height = inner.get::<i64>("dheight", FORMAT_INT64).unwrap_or(0);
        let (width, height) = if inner.backdrop { fit_into(width, height, BACKDROP_WIDTH, BACKDROP_HEIGHT) } else { fit(width, height) };
        let stride = (width as usize * 4).div_ceil(64) * 64;
        let size = stride * height as usize;
        if buffer.as_ref().is_none_or(|(have, _)| *have < size) {
            buffer = Some((size, Aligned::new(size)));
        }
        let (_, aligned) = buffer.as_mut().unwrap();
        let pixels = aligned.slice(size);
        let mut dims = [width as c_int, height as c_int];
        let mut stride_value = stride;
        let format = c"rgb0";
        let mut params = [
            RenderParam { kind: PARAM_SW_SIZE, data: dims.as_mut_ptr() as *mut c_void },
            RenderParam { kind: PARAM_SW_FORMAT, data: format.as_ptr() as *mut c_void },
            RenderParam { kind: PARAM_SW_STRIDE, data: &mut stride_value as *mut usize as *mut c_void },
            RenderParam { kind: PARAM_SW_POINTER, data: pixels.as_mut_ptr() as *mut c_void },
            RenderParam { kind: PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        let code = unsafe { (inner.lib.render_render)(render, params.as_mut_ptr()) };
        // Пока кадра нет, mpv рисует чёрное поле; его не показываем.
        if code < 0 || !inner.loaded.load(Ordering::SeqCst) || width < 16 {
            continue;
        }
        let mut frame = inner.frame.lock().unwrap();
        frame.width = width as u32;
        frame.height = height as u32;
        frame.stride = stride;
        frame.pixels.clear();
        frame.pixels.extend_from_slice(pixels);
        drop(frame);
        inner.serial.fetch_add(1, Ordering::SeqCst);
    }
}

/// Размер кадра в памяти: как у видео, но не больше Full HD. Пока размера нет — заглушка.
fn fit(width: i64, height: i64) -> (i64, i64) {
    fit_into(width, height, MAX_WIDTH, MAX_HEIGHT)
}

fn fit_into(width: i64, height: i64, max_width: i64, max_height: i64) -> (i64, i64) {
    if width <= 0 || height <= 0 {
        return (8, 8);
    }
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64).min(1.0);
    (((width as f64 * scale).round() as i64).max(2) & !1, ((height as f64 * scale).round() as i64).max(2) & !1)
}

/// Картинка клипа на странице: вписана в рамку с полями, края скруглены.
pub struct View {
    inner: Arc<Inner>,
    radius: f32,
}

impl<Message> shader::Program<Message> for View {
    type State = ();
    type Primitive = Primitive;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> Primitive {
        Primitive { inner: self.inner.clone(), radius: self.radius }
    }
}

pub struct Primitive {
    inner: Arc<Inner>,
    radius: f32,
}

impl std::fmt::Debug for Primitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("video::Primitive")
    }
}

/// Четыре `vec4`: рамка, место кадра в рамке, радиус и признак кадра.
const UNIFORMS: u64 = 3 * 16;

pub struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    layout: wgpu::BindGroupLayout,
    texture: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    /// Какой кадр уже на видеокарте и от какого плеера.
    shown: (usize, u64),
    /// На видеокарте кадр именно этого плеера: иначе не рисуем, под видео видна обложка.
    ready: bool,
    srgb: bool,
    /// Пустая текстура 1×1: фону без кадра она нужна для привязки.
    empty: Option<(wgpu::Texture, wgpu::BindGroup)>,
    /// Размер ячейки параметров, если их несколько.
    slot: Option<u64>,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Pipeline::build(device, queue, format, SHADER, UNIFORMS, None)
    }
}

impl Pipeline {
    /// `slot` — размер ячейки параметров, если их несколько (фон и панели), иначе `None`.
    fn build(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, source: &str, uniforms_size: u64, slot: Option<u64>) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("video"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor { label: Some("video"), size: uniforms_size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("video"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: slot.is_some(), min_binding_size: slot.and_then(wgpu::BufferSize::new) }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("video"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let mut built = Self { pipeline, uniforms, sampler, layout, texture: None, shown: (0, u64::MAX), ready: false, srgb: format.is_srgb(), empty: None, slot };
        let (texture, bind) = built.texture_for(device, 1, 1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &[0, 0, 0, 0],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        built.empty = Some((texture, bind));
        built
    }

    fn texture_for(&self, device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup) {
        let format = if self.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: match self.slot {
                    Some(size) => wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniforms, offset: 0, size: wgpu::BufferSize::new(size) }),
                    None => self.uniforms.as_entire_binding(),
                } },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        (texture, bind)
    }

    /// Новый кадр плеера — на видеокарту; тот же кадр второй раз не возится.
    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, inner: &Inner, owner: usize, serial: u64) {
        if self.shown == (owner, serial) {
            return;
        }
        let frame = inner.frame.lock().unwrap();
        if frame.width > 0 {
            let same = self.texture.as_ref().is_some_and(|(_, _, w, h)| *w == frame.width && *h == frame.height);
            if !same {
                let (texture, bind) = self.texture_for(device, frame.width, frame.height);
                self.texture = Some((texture, bind, frame.width, frame.height));
            }
            let (texture, ..) = self.texture.as_ref().unwrap();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &frame.pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.stride as u32), rows_per_image: Some(frame.height) },
                wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
            );
        }
        self.shown = (owner, serial);
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(&self, pipeline: &mut Pipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let owner = Arc::as_ptr(&self.inner) as usize;
        let serial = self.inner.serial.load(Ordering::SeqCst);
        pipeline.upload(device, queue, &self.inner, owner, serial);
        let scale = viewport.scale_factor();
        let area = [bounds.x * scale, bounds.y * scale, bounds.width * scale, bounds.height * scale];
        // Кадр вписан в рамку с сохранением пропорций, по краям — чёрные поля.
        let picture = match pipeline.texture.as_ref() {
            Some((_, _, w, h)) => {
                let fit = (area[2] / *w as f32).min(area[3] / *h as f32);
                let (pw, ph) = (*w as f32 * fit, *h as f32 * fit);
                [area[0] + (area[2] - pw) / 2.0, area[1] + (area[3] - ph) / 2.0, pw, ph]
            }
            None => [0.0; 4],
        };
        pipeline.ready = pipeline.texture.is_some() && owner == pipeline.shown.0 && serial > 0;
        let has = if pipeline.ready { 1.0 } else { 0.0 };
        let values: [[f32; 4]; 3] = [area, picture, [self.radius * scale, has, 0.0, 0.0]];
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|x| x.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, 0, &bytes);
    }

    fn draw(&self, pipeline: &Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some((_, bind, ..)) = pipeline.texture.as_ref().filter(|_| pipeline.ready) else {
            return true;
        };
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, bind, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

const SHADER: &str = r#"
struct Uniforms {
    area: vec4<f32>,
    picture: vec4<f32>,
    extra: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var smooth_: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// Расстояние до скруглённого прямоугольника: меньше нуля — внутри.
fn rounded(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let half = u.area.zw * 0.5;
    let d = rounded(pos.xy - u.area.xy - half, half, u.extra.x);
    let cover = clamp(0.5 - d, 0.0, 1.0);
    var color = vec3<f32>(0.0);
    let uv = (pos.xy - u.picture.xy) / u.picture.zw;
    if (u.extra.y > 0.5 && all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
        color = textureSampleLevel(frame, smooth_, uv, 0.0).rgb;
    }
    return vec4<f32>(color * cover, cover);
}
"#;

/// Кадр фона: под затемнением и за стеклом больше 540p не видно, а кадр
/// поменьше вдвое дешевле переводить в цвета и возить на видеокарту.
const BACKDROP_WIDTH: i64 = 960;
const BACKDROP_HEIGHT: i64 = 540;
/// Фон не рисовался дольше — окна не видно, видео на паузу.
const BACKDROP_HIDDEN_MS: u64 = 1500;
/// Фон перерисовывается сам, без пересборки интерфейса: 24 кадра в секунду, как видео фона.
const BACKDROP_FRAME: Duration = Duration::from_micros(41_667);

/// Миллисекунды от запуска программы: часы фона и пузырей.
fn epoch() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Сколько места у одной ячейки параметров фона: фон и каждая стеклянная панель пишут свою.
const SLOT_STRIDE: u64 = 256;
const SLOT_SIZE: u64 = 8 * 16;
/// Ячейки: 0 — фон окна, дальше панели.
pub const SLOT_BACKDROP: u32 = 0;
pub const SLOT_CONTENT: u32 = 1;
pub const SLOT_PLAYER: u32 = 2;
const SLOTS: u64 = 4;

/// Стеклянная панель над фоном: под ней тот же кусок видео, что и на фоне, только
/// размытый и подкрашенный. С `glass` — Liquid Glass: края преломляют фон, по кромке блики.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub radius: f32,
    /// Цвет стекла, `a` — насколько оно плотное.
    pub tint: iced::Color,
    pub blur: f32,
    /// Сила Liquid Glass от 0 до 1; 0 — обычное матовое стекло.
    pub glass: f32,
}

/// Живой фон темы на всё окно: кадр видео как `cover` (обрезается по краям, `focus` — какая
/// точка кадра остаётся на виду), приглушённый под текст, при желании размытый, и пузыри
/// поверх. Тот же виджет с `panel` — стекло панели над этим фоном.
pub struct Backdrop {
    inner: Option<Arc<Inner>>,
    /// Яркость и насыщенность кадра.
    pub bright: f32,
    pub saturation: f32,
    /// Сколько пузырей: 0 — без них.
    pub bubbles: f32,
    pub focus: (f32, f32),
    /// Размытие фона в точках.
    pub blur: f32,
    panel: Option<Panel>,
    slot: u32,
}

impl Backdrop {
    pub fn new(video: Option<&Video>) -> Self {
        Backdrop { inner: video.map(|video| video.inner.clone()), bright: 0.62, saturation: 0.8, bubbles: 14.0, focus: (0.4, 0.5), blur: 0.0, panel: None, slot: SLOT_BACKDROP }
    }

    /// Стекло панели над фоном `backdrop` (яркость, насыщенность и место кадра — его же).
    pub fn panel(backdrop: &Backdrop, slot: u32, panel: Panel) -> Self {
        Backdrop { inner: backdrop.inner.clone(), bubbles: 0.0, panel: Some(panel), slot, ..*backdrop }
    }
}

impl<Message> shader::Program<Message> for Backdrop {
    type State = ();
    type Primitive = BackdropPrimitive;

    fn update(&self, _: &mut (), event: &iced::Event, _: Rectangle, _: mouse::Cursor) -> Option<shader::Action<Message>> {
        // Фон сам заказывает следующий кадр: интерфейс при этом не пересобирается. Панели
        // перерисовываются вместе с ним.
        if self.panel.is_some() {
            return None;
        }
        let iced::Event::Window(iced::window::Event::RedrawRequested(now)) = event else { return None };
        if let Some(inner) = &self.inner {
            inner.seen.store(epoch(), Ordering::Relaxed);
        }
        Some(shader::Action::request_redraw_at(*now + BACKDROP_FRAME))
    }

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> BackdropPrimitive {
        BackdropPrimitive {
            inner: self.inner.clone(),
            look: [self.bright, self.saturation, self.bubbles, (epoch() % 3_600_000) as f32 / 1000.0],
            focus: self.focus,
            blur: self.blur,
            panel: self.panel,
            slot: self.slot,
        }
    }
}

pub struct BackdropPrimitive {
    inner: Option<Arc<Inner>>,
    look: [f32; 4],
    focus: (f32, f32),
    blur: f32,
    panel: Option<Panel>,
    slot: u32,
}

impl std::fmt::Debug for BackdropPrimitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("video::BackdropPrimitive")
    }
}

/// Своё хранилище на видеокарте: фон и открытый клип рисуются одновременно. Параметры —
/// по ячейке на фон и каждую панель.
pub struct BackdropPipeline(Pipeline);

impl shader::Pipeline for BackdropPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        BackdropPipeline(Pipeline::build(device, queue, format, BACKDROP_SHADER, SLOT_STRIDE * SLOTS, Some(SLOT_SIZE)))
    }
}

impl shader::Primitive for BackdropPrimitive {
    type Pipeline = BackdropPipeline;

    fn prepare(&self, pipeline: &mut BackdropPipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let pipeline = &mut pipeline.0;
        let owner = self.inner.as_ref().map_or(0, |inner| Arc::as_ptr(inner) as usize);
        let serial = self.inner.as_ref().map_or(0, |inner| inner.serial.load(Ordering::SeqCst));
        if let Some(inner) = &self.inner {
            pipeline.upload(device, queue, inner, owner, serial);
        }
        let scale = viewport.scale_factor();
        let area = [bounds.x * scale, bounds.y * scale, bounds.width * scale, bounds.height * scale];
        let window = viewport.physical_size();
        let (width, height) = (window.width as f32, window.height as f32);
        // Кадр закрывает всё окно, лишнее обрезается; у панелей то же место кадра, что у фона.
        let picture = match pipeline.texture.as_ref() {
            Some((_, _, w, h)) => {
                let fill = (width / *w as f32).max(height / *h as f32);
                let (pw, ph) = (*w as f32 * fill, *h as f32 * fill);
                [(width - pw) * self.focus.0, (height - ph) * self.focus.1, pw, ph]
            }
            None => [0.0; 4],
        };
        let ready = pipeline.texture.is_some() && owner != 0 && owner == pipeline.shown.0 && serial > 0;
        let has = if ready { 1.0 } else { 0.0 };
        let (mode, radius, tint, panel_blur, glass) = match self.panel {
            Some(panel) => (1.0, panel.radius * scale, [panel.tint.r, panel.tint.g, panel.tint.b, panel.tint.a], panel.blur * scale, panel.glass),
            None => (0.0, 0.0, [0.0; 4], 0.0, 0.0),
        };
        let values: [[f32; 4]; 8] = [area, picture, [scale, has, radius, mode], self.look, [self.blur * scale, panel_blur, glass, 0.0], tint, [width, height, 0.0, 0.0], [0.0; 4]];
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|x| x.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, u64::from(self.slot) * SLOT_STRIDE, &bytes);
    }

    fn draw(&self, pipeline: &BackdropPipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let pipeline = &pipeline.0;
        // Без кадра шейдеру всё равно нужна привязка: пустая текстура 1×1.
        let Some(bind) = pipeline.texture.as_ref().map(|(_, bind, ..)| bind).or(pipeline.empty.as_ref().map(|(_, bind)| bind)) else { return true };
        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, bind, &[(u64::from(self.slot) * SLOT_STRIDE) as u32]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

const BACKDROP_SHADER: &str = r#"
struct Uniforms {
    area: vec4<f32>,
    picture: vec4<f32>,
    // масштаб, есть ли кадр, радиус панели, 0 — фон / 1 — панель
    extra: vec4<f32>,
    // яркость, насыщенность, пузыри, время
    look: vec4<f32>,
    // размытие фона, размытие панели, Liquid Glass
    blur: vec4<f32>,
    tint: vec4<f32>,
    window: vec4<f32>,
    spare: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var smooth_: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(n: f32) -> f32 {
    return fract(sin(n * 127.1 + 311.7) * 43758.5453);
}

// Шум для поворота узора размытия: без него при сильном размытии видны кольца.
fn noise(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// Точка кадра под точкой окна, приглушённая в холодную бирюзу, как у темы в Tune Client.
fn video(p: vec2<f32>) -> vec3<f32> {
    let uv = clamp((p - u.picture.xy) / u.picture.zw, vec2<f32>(0.0), vec2<f32>(1.0));
    let raw = textureSampleLevel(frame, smooth_, uv, 0.0).rgb;
    let gray = dot(raw, vec3<f32>(0.299, 0.587, 0.114));
    var tinted = mix(vec3<f32>(gray), raw, u.look.y) * u.look.x;
    tinted = (tinted - 0.5) * 1.05 + 0.5;
    // Края окна темнее, как в глубине аквариума.
    let center = p / u.window.xy - vec2<f32>(0.45, 0.35);
    let shade = mix(1.0, 0.65, clamp(length(center) / 0.9, 0.0, 1.0));
    return clamp(tinted * shade, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Размытие: 16 точек по спирали, узор повёрнут шумом.
fn blurred(p: vec2<f32>, radius: f32, seed: vec2<f32>) -> vec3<f32> {
    if (radius < 0.5) {
        return video(p);
    }
    var sum = vec3<f32>(0.0);
    let turn = noise(seed) * 6.2831853;
    for (var i = 0; i < 16; i = i + 1) {
        let k = f32(i);
        let angle = k * 2.3999632 + turn;
        let r = radius * sqrt((k + 0.5) / 16.0);
        sum = sum + video(p + vec2<f32>(cos(angle), sin(angle)) * r);
    }
    return sum / 16.0;
}

// Расстояние до скруглённого прямоугольника: меньше нуля — внутри.
fn rounded(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn bubbles(local: vec2<f32>, size: vec2<f32>, base: vec4<f32>) -> vec4<f32> {
    var color = base.rgb;
    var alpha = base.a;
    let side = min(size.x, size.y);
    let count = i32(u.look.z);
    for (var i = 0; i < count; i = i + 1) {
        let n = f32(i);
        let period = 16.0 + hash(n) * 16.0;
        let phase = fract(u.look.w / period + hash(n + 7.0));
        let sway = (hash(n + 3.0) * 2.0 - 1.0) * 0.09 * size.x;
        let lift = phase * 1.15 * size.y;
        let drift = sway * sin(phase * 3.14159);
        let radius = side * 0.011 * (0.6 + hash(n + 11.0) * 1.2);
        let center = vec2<f32>(hash(n + 5.0) * size.x + drift, size.y * 1.08 - lift);
        let offset = local - center;
        let d = length(offset) / radius;
        if (d < 1.4) {
            // Тонкая светлая кромка, бирюзовое тело и блик сверху слева.
            let rim = smoothstep(0.78, 0.96, d) * (1.0 - smoothstep(0.96, 1.04, d));
            let body = (1.0 - smoothstep(0.0, 1.0, d)) * 0.18;
            let glint = 1.0 - smoothstep(0.0, 0.28, length(offset / radius - vec2<f32>(-0.36, -0.44)));
            let glow = (1.0 - smoothstep(1.0, 1.4, d)) * 0.12;
            let a = clamp(rim * 0.75 + body + glint * 0.9 + glow, 0.0, 1.0) * 0.6;
            let tint = mix(vec3<f32>(0.63, 1.0, 0.94), vec3<f32>(1.0), glint);
            color = color * (1.0 - a) + tint * a;
            alpha = alpha + a * (1.0 - alpha);
        }
    }
    return vec4<f32>(color, alpha);
}

fn backdrop(pos: vec2<f32>) -> vec4<f32> {
    let local = pos - u.area.xy;
    if (any(local < vec2<f32>(0.0)) || any(local > u.area.zw)) {
        return vec4<f32>(0.0);
    }
    var base = vec4<f32>(0.0);
    if (u.extra.y > 0.5) {
        base = vec4<f32>(blurred(pos, u.blur.x, pos), 1.0);
    }
    return bubbles(local, u.area.zw, base);
}

fn panel(pos: vec2<f32>) -> vec4<f32> {
    let half = u.area.zw * 0.5;
    let q = pos - u.area.xy - half;
    let r = u.extra.z;
    let d = rounded(q, half, r);
    let inside = clamp(0.5 - d, 0.0, 1.0);
    if (inside <= 0.0) {
        return vec4<f32>(0.0);
    }
    let scale = u.extra.x;
    let glass = u.blur.z;
    // Наружу от края: по разнице расстояний в соседних точках.
    let e = 1.0;
    let normal = normalize(vec2<f32>(rounded(q + vec2<f32>(e, 0.0), half, r) - rounded(q - vec2<f32>(e, 0.0), half, r), rounded(q + vec2<f32>(0.0, e), half, r) - rounded(q - vec2<f32>(0.0, e), half, r)) + vec2<f32>(1e-5));
    // 0 у самого края, 1 — дальше полосы преломления.
    let depth = clamp(-d / (44.0 * scale), 0.0, 1.0);
    let lens = pow(1.0 - depth, 2.0);
    // Liquid Glass прозрачнее матового стекла: фон за ним виден, а держит форму кромка.
    let density = u.tint.a * (1.0 - 0.55 * glass);
    var color = u.tint.rgb;
    if (u.extra.y > 0.5) {
        // Liquid Glass: у края фон тянется внутрь, как в толстом стекле, и расслаивается по цветам.
        let p = pos - normal * lens * 70.0 * scale * glass;
        let spread = lens * 6.0 * scale * glass;
        // Посередине стекло чище, размытие сильнее к краю.
        let soft = u.blur.y * mix(1.0, 0.6 + 0.4 * lens, glass);
        var seen = blurred(p, soft, pos);
        if (spread > 0.35) {
            seen = vec3<f32>(blurred(p - normal * spread, soft, pos).r, seen.g, blurred(p + normal * spread, soft, pos).b);
        }
        color = mix(seen, u.tint.rgb, density);
    }
    if (glass > 0.0) {
        let y = (pos.y - u.area.y) / u.area.w;
        // Мягкий блик сверху и бирюзовое свечение снизу, как свет над водой.
        color = color + vec3<f32>(1.0) * 0.06 * glass * (1.0 - smoothstep(0.0, 0.35, y));
        color = color + vec3<f32>(0.47, 0.94, 0.88) * 0.08 * glass * smoothstep(0.7, 1.0, y);
        // Кромка: яркая там, куда падает свет (сверху слева), слабее напротив.
        let light = normalize(vec2<f32>(-0.55, -0.85));
        let band = 1.0 - smoothstep(0.0, 2.5 * scale, -d);
        let wide = 1.0 - smoothstep(0.0, 14.0 * scale, -d);
        let spec = pow(max(dot(normal, light), 0.0), 1.6);
        let back = pow(max(dot(normal, -light), 0.0), 2.0);
        // Вторая, тонкая кромка чуть внутри: у толстого стекла их две.
        let inner = (1.0 - smoothstep(0.0, 1.2 * scale, abs(-d - 5.0 * scale)));
        color = color + vec3<f32>(1.0) * band * (0.9 * spec + 0.18) * glass;
        color = color + vec3<f32>(0.63, 1.0, 0.94) * band * back * 0.45 * glass;
        color = color + vec3<f32>(1.0) * inner * (0.25 * spec + 0.05) * glass;
        color = color + vec3<f32>(1.0) * wide * spec * 0.08 * glass;
    }
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(color * inside, inside);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    if (u.extra.w > 0.5) {
        return panel(pos.xy);
    }
    return backdrop(pos.xy);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_fits_full_hd_and_stays_even() {
        assert_eq!(fit(1280, 720), (1280, 720));
        assert_eq!(fit(3840, 2160), (1920, 1080));
        assert_eq!(fit(1440, 1080), (1440, 1080));
        assert_eq!(fit(0, 0), (8, 8));
        assert_eq!(fit(853, 480), (852, 480));
    }
}
