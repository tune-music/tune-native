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
        option("hwdec", "auto-copy");
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
    loop {
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
        let (width, height) = fit(width, height);
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
    if width <= 0 || height <= 0 {
        return (8, 8);
    }
    let scale = (MAX_WIDTH as f64 / width as f64).min(MAX_HEIGHT as f64 / height as f64).min(1.0);
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
    srgb: bool,
}

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("video"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor { label: Some("video"), size: UNIFORMS, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("video"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
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
        Self { pipeline, uniforms, sampler, layout, texture: None, shown: (0, u64::MAX), srgb: format.is_srgb() }
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(&self, pipeline: &mut Pipeline, device: &wgpu::Device, queue: &wgpu::Queue, bounds: &Rectangle, viewport: &Viewport) {
        let owner = Arc::as_ptr(&self.inner) as usize;
        let serial = self.inner.serial.load(Ordering::SeqCst);
        if pipeline.shown != (owner, serial) {
            let frame = self.inner.frame.lock().unwrap();
            if frame.width > 0 {
                let same = pipeline.texture.as_ref().is_some_and(|(_, _, w, h)| *w == frame.width && *h == frame.height);
                if !same {
                    let format = if pipeline.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
                    let size = wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 };
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
                        layout: &pipeline.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: pipeline.uniforms.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&pipeline.sampler) },
                        ],
                    });
                    pipeline.texture = Some((texture, bind, frame.width, frame.height));
                }
                let (texture, ..) = pipeline.texture.as_ref().unwrap();
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &frame.pixels,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.stride as u32), rows_per_image: Some(frame.height) },
                    wgpu::Extent3d { width: frame.width, height: frame.height, depth_or_array_layers: 1 },
                );
            }
            pipeline.shown = (owner, serial);
        }
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
        let has = if pipeline.texture.is_some() && owner == pipeline.shown.0 && serial > 0 { 1.0 } else { 0.0 };
        let values: [[f32; 4]; 3] = [area, picture, [self.radius * scale, has, 0.0, 0.0]];
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|x| x.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, 0, &bytes);
    }

    fn draw(&self, pipeline: &Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some((_, bind, ..)) = pipeline.texture.as_ref() else {
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
