#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod fonts;
mod login;
mod media;
mod player;
mod seek;
mod settings;
mod ui;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::ops::Range;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::futures::channel::{mpsc, oneshot};
use iced::widget::text::Wrapping;
use iced::keyboard;
use iced::widget::{button, canvas, column, container, image, mouse_area, operation, row, scrollable, slider, stack, text, text_input, Id, Space};
use iced::{event, mouse, time, window, Alignment, Event, Element, Length, Padding, Size, Subscription, Task};

use api::{Album, Artist, Card, Found, Pin, PinTarget, Playlist, Profile, Source, Track, Wave, NEED_LOGIN};
use media::Media;
use player::Player;
use raw_window_handle::RawWindowHandle;
use settings::{Quality, Repeat, Settings};

const TICK: Duration = Duration::from_millis(500);
const COVER: f32 = 40.0;
const BAR_COVER: f32 = 64.0;
const BAR_HEIGHT: f32 = 80.0;
const CARD: f32 = 190.0;
const WAVE_COVER: f32 = 120.0;
const PILL_WIDTH: f32 = 320.0;
const MENU_WIDTH: f32 = 270.0;
const MENU_ITEM: f32 = 40.0;
const NOTICE_TIME: Duration = Duration::from_secs(4);

/// Где курсор и какого размера окно. Меняются постоянно, поэтому не идут через сообщения:
/// иначе интерфейс перестраивался бы от каждого движения мыши.
static CURSOR: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
static WINDOW: Mutex<(f32, f32)> = Mutex::new((1280.0, 820.0));
const PIN_COVER_PREFIX: &str = "pin:";
const SIDEBAR: f32 = 224.0;
const LIKE: f32 = 32.0;
/// Постоянное место под полосу прокрутки: колонка времени не прыгает на коротких списках.
const SCROLLBAR: f32 = 14.0;
const SEARCH_INPUT: &str = "search";
const COVER_WORKERS: usize = 6;
const LIKES_PAGE: usize = 100;
/// Сколько треков до конца очереди оставить, прежде чем просить у волны новые.
const WAVE_MARGIN: usize = 2;
const WAVE_HISTORY: usize = 50;
/// Сколько сбойных треков подряд пропустить, прежде чем остановиться: сеть могла пропасть.
const MAX_FAILURES: u32 = 3;
/// Высота строки трека. Одинаковая у всех: по ней считается, какие строки видны.
const ROW: f32 = 58.0;
/// Сколько строк держать готовыми за краем экрана.
const ROW_MARGIN: usize = 6;
const LIST: &str = "list";
/// Сколько результатов просить: на вкладке «Всё» и на отдельной вкладке.
const SEARCH_ALL: u32 = 30;
const SEARCH_TAB: u32 = 100;
/// Сколько треков показать на вкладке «Всё», а карточек — в блоках коллекции.
const SEARCH_ALL_TRACKS: usize = 5;
const COLLECTION_CARDS: usize = 5;
/// Пауза после нажатия клавиши, прежде чем искать.
const SEARCH_DELAY: Duration = Duration::from_millis(350);
/// Высота шапки альбома и исполнителя: альбом считает видимые строки с её учётом.
const PAGE_HEADER: f32 = 240.0;
const PAGE_COVER: f32 = 200.0;
const ALBUM_COVER: &str = "album:";
const ARTIST_COVER: &str = "artist:";
/// За сколько до конца трека начать качать следующий.
const PRELOAD_BEFORE: Duration = Duration::from_secs(30);
const QUEUE_LIST: &str = "queue";
const QUEUE_WIDTH: f32 = 360.0;
const QUALITY_WIDTH: f32 = 250.0;
/// Сколько места оставить над играющим треком, когда открывается очередь.
const QUEUE_ABOVE: f32 = 2.0 * ROW;
const SKELETON_ROWS: usize = 10;
/// После скольких обложек убирать те, что далеко от экрана.
const COVERS_LIMIT: usize = 400;
const COVERS_AROUND: usize = 100;
const QUEUE_COVERS_AROUND: usize = 5;
/// С этой секунды «предыдущий» возвращает к началу трека, как в оригинале.
const PREVIOUS_RESTART: Duration = Duration::from_secs(3);
const PLAYLIST_COVER: &str = "playlist:";

/// Какие строки длинного списка надо построить при такой прокрутке.
fn visible_rows(offset: f32, height: f32, len: usize) -> Range<usize> {
    let first = (offset.max(0.0) / ROW) as usize;
    let last = ((offset.max(0.0) + height) / ROW).ceil() as usize;
    let end = (last + ROW_MARGIN).min(len);
    first.saturating_sub(ROW_MARGIN).min(end)..end
}

/// Случайный порядок: выбранный трек становится первым, остальные перемешиваются.
fn shuffled<T>(items: &mut [T], keep: usize, mut seed: u64) {
    if items.is_empty() {
        return;
    }
    items.swap(0, keep);
    seed |= 1;
    for i in (2..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, 1 + (seed % i as u64) as usize);
    }
}

fn random_seed() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |t| t.as_nanos() as u64)
}

/// Блокирующая работа в отдельном потоке, чтобы не останавливать интерфейс.
fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl Future<Output = T> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("фоновая задача завершилась") }
}

enum Screen {
    /// true — открыто окно входа.
    Login(bool),
    Main(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Wave,
    Collection,
    Likes,
    Playlists,
    Artist,
    Albums,
    Artists,
    Search,
}

/// Вкладка поиска.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SearchTab {
    All,
    Tracks,
    Albums,
    Artists,
    Playlists,
}

impl SearchTab {
    const ALL: [SearchTab; 5] = [SearchTab::All, SearchTab::Tracks, SearchTab::Albums, SearchTab::Artists, SearchTab::Playlists];

    fn label(self) -> &'static str {
        match self {
            SearchTab::All => "Всё",
            SearchTab::Tracks => "Треки",
            SearchTab::Albums => "Альбомы",
            SearchTab::Artists => "Исполнители",
            SearchTab::Playlists => "Плейлисты",
        }
    }

    fn types(self) -> &'static str {
        match self {
            SearchTab::All => "album,artist,playlist,track",
            SearchTab::Tracks => "track",
            SearchTab::Albums => "album",
            SearchTab::Artists => "artist",
            SearchTab::Playlists => "playlist",
        }
    }
}

/// Куда возвращает кнопка «назад».
#[derive(Clone)]
struct Place {
    section: Section,
    playlist: Option<Playlist>,
    artist: Option<Artist>,
}

/// Запуск с готовым разделом: первый трек показывается в плеере, но не играет.
#[derive(Clone)]
enum Preview {
    Wave,
    /// Главная с перемоткой в самом начале трека.
    WaveSeek,
    Collection,
    Likes,
    /// Открытые очередь и выбор качества.
    Panels,
    /// Лайки, прокрученные до конца.
    LikesEnd,
    Playlists,
    /// Сразу открыть первый плейлист.
    Playlist,
    Album(String),
    Artist(String),
    /// Первый плейлист в очереди, открыта главная.
    PlaylistHome,
    /// Открытое меню первого трека.
    Menu,
    Search(String),
}

struct App {
    screen: Screen,
    section: Section,
    /// Откуда пришли: кнопка «назад» и боковая кнопка мыши.
    history: Vec<Place>,
    bar_hover: bool,
    profile: Option<Profile>,
    query: String,
    results: Vec<Track>,
    found: Found,
    search_tab: SearchTab,
    /// Что уже искали: для вкладок и подсказки.
    searched: bool,
    liked_albums: Vec<Card>,
    liked_artists: Vec<Card>,
    searching: bool,
    search_id: u64,
    uid: Option<String>,
    liked_ids: Vec<String>,
    liked_set: HashSet<String>,
    disliked: HashSet<String>,
    /// Открытое меню трека и где его показать.
    menu: Option<(Track, f32, f32)>,
    notice: Option<(u64, String)>,
    liked: Vec<Track>,
    likes_loading: bool,
    pins: Vec<Pin>,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    /// Открытый плейлист и его треки; показывается только начало списка.
    playlist: Option<Playlist>,
    playlist_tracks: Vec<Track>,
    /// Шапка открытого альбома.
    album: Option<Album>,
    artist: Option<Artist>,
    /// Какого исполнителя сейчас загружаем.
    artist_loading: Option<String>,
    /// Какой трек качается заранее.
    preloaded: Option<String>,
    covers: HashMap<String, image::Handle>,
    /// Какие обложки уже просили, чтобы при прокрутке не просить снова.
    requested: RefCell<HashSet<String>>,
    /// Сколько лайков уже запрошено и прокрутка открытого списка.
    likes_from: usize,
    list_offset: f32,
    /// Что повторить кнопкой под ошибкой.
    retry: Option<Message>,
    media: Option<Media>,
    failures: u32,
    player: Option<Player>,
    /// Что играет: смена раздела и новый поиск не прерывают музыку.
    queue: Vec<Track>,
    /// Сессия волны, если очередь — это волна.
    wave: Option<String>,
    /// Волна загружена, но ещё не запущена: сессия начнётся с первым треком.
    wave_pending: Option<String>,
    /// Откуда играет очередь, если это не волна.
    queue_title: Option<String>,
    /// В очереди треки волны. В режиме просмотра они есть и без сессии.
    wave_queue: bool,
    wave_loading: bool,
    wave_fetching: bool,
    play_id: u64,
    current: Option<usize>,
    /// Трек запущен и ещё не доиграл.
    active: bool,
    starting: bool,
    position: Duration,
    /// Куда тянут ползунок: пока его держат, ход песни его не двигает.
    seeking: Option<f32>,
    volume: f32,
    muted: bool,
    repeat: Repeat,
    shuffle: bool,
    /// Исходный порядок очереди, пока включён случайный.
    unshuffled: Option<Vec<Track>>,
    quality: Quality,
    /// Формат файла, который играет сейчас: Яндекс может отдать не то, что просили.
    codec: Option<String>,
    /// Выбор качества открыт в этой точке.
    quality_menu: Option<(f32, f32)>,
    queue_open: bool,
    queue_offset: f32,
    error: Option<String>,
    preview: Option<Preview>,
}

#[derive(Clone, Debug)]
enum Message {
    LoginOpen,
    LoginFinished,
    Logout,
    Account(Result<(Profile, Vec<String>, Vec<String>), String>),
    Back,
    BarHover(bool),
    Open(Section),
    Query(String),
    Search,
    Found(u64, Result<Found, String>),
    SearchTyped(u64),
    SearchTabSet(SearchTab),
    ListOpen(Playlist),
    LikedPages(Result<(Vec<Card>, Vec<Card>), String>),
    AlbumLike,
    ArtistLike,
    PageLikeSaved(Result<(), String>),
    LikesMore,
    LikesPage(usize, Result<Vec<Track>, String>),
    Scrolled(f32),
    Retry,
    ListReload,
    AlbumOpen(String, String),
    AlbumLoaded(String, Result<(Album, Vec<Track>), String>),
    ArtistOpen(String),
    ArtistLoad(String),
    ArtistLoaded(String, Result<Artist, String>),
    ArtistTracks,
    ToggleMute,
    RepeatNext,
    ShuffleToggle,
    QualityOpen,
    QualitySet(Quality),
    QueueToggle,
    QueueScrolled(f32),
    QueuePlay(usize),
    QueueRemove(usize),
    Like(Track),
    LikeSaved(Track, bool, Result<(), String>),
    WaveStart,
    WaveStartSeeds(Vec<String>),
    WaveStarted(bool, Result<Wave, String>),
    WaveMore(Result<Vec<Track>, String>),
    Cover(String, Vec<u8>),
    PlaylistsLoaded(Result<Vec<Playlist>, String>),
    PlaylistOpen(usize),
    PinOpen(usize),
    PinsLoaded(Result<Vec<Pin>, String>),
    PlaylistLoaded(String, Result<Vec<Track>, String>),
    MenuOpen(Track),
    MenuClose,
    Dislike(Track),
    DislikeSaved(String, bool, Result<(), String>),
    PlayNext(Track),
    PlayLast(Track),
    TrackWave(Track),
    OpenAlbum(Track),
    OpenArtist(Track),
    CopyLink(Track),
    NoticeClear(u64),
    WindowReady(Option<isize>),
    Media(media::Event),
    Play(Section, usize),
    Started(u64, Result<String, String>),
    Toggle,
    Next,
    Previous,
    Tick,
    Seek(f32),
    SeekDone,
    Seeked(Result<(), String>),
    Volume(f32),
}

impl App {
    fn new(preview: Option<Preview>) -> (Self, Task<Message>) {
        let (player, error) = match Player::new() {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        let saved = settings::load();
        let token = api::load_token();
        let task = match &token {
            Some(token) => load_account(token.clone()),
            None => Task::done(Message::LoginOpen),
        };
        let app = Self {
            screen: token.map_or(Screen::Login(false), Screen::Main),
            section: Section::Wave,
            history: Vec::new(),
            bar_hover: false,
            profile: None,
            query: String::new(),
            results: Vec::new(),
            found: Found::default(),
            search_tab: SearchTab::All,
            searched: false,
            liked_albums: Vec::new(),
            liked_artists: Vec::new(),
            searching: false,
            search_id: 0,
            uid: None,
            liked_ids: Vec::new(),
            liked_set: HashSet::new(),
            disliked: HashSet::new(),
            menu: None,
            notice: None,
            liked: Vec::new(),
            likes_loading: false,
            pins: Vec::new(),
            playlists: Vec::new(),
            playlists_loading: false,
            playlist: None,
            playlist_tracks: Vec::new(),
            album: None,
            artist: None,
            artist_loading: None,
            preloaded: None,
            covers: HashMap::new(),
            requested: RefCell::new(HashSet::new()),
            likes_from: 0,
            list_offset: 0.0,
            retry: None,
            media: None,
            failures: 0,
            player,
            queue: Vec::new(),
            wave: None,
            wave_pending: None,
            queue_title: None,
            wave_queue: false,
            wave_loading: false,
            wave_fetching: false,
            play_id: 0,
            current: None,
            active: false,
            starting: false,
            position: Duration::ZERO,
            seeking: None,
            volume: saved.volume,
            muted: saved.muted,
            repeat: saved.repeat,
            shuffle: saved.shuffle,
            unshuffled: None,
            quality: saved.quality,
            codec: None,
            quality_menu: None,
            queue_open: false,
            queue_offset: 0.0,
            error,
            preview,
        };
        app.apply_volume();
        (app, Task::batch([task, window_handle()]))
    }

    fn apply_volume(&self) {
        if let Some(p) = &self.player {
            p.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    fn save_settings(&self) {
        settings::save(Settings { volume: self.volume, muted: self.muted, repeat: self.repeat, shuffle: self.shuffle, quality: self.quality });
    }

    /// Куда идти дальше кнопкой или по концу трека. В волне повтора нет: она бесконечна.
    fn next_index(&self) -> Option<usize> {
        let index = self.current?;
        if index + 1 < self.queue.len() {
            Some(index + 1)
        } else {
            (self.repeat == Repeat::All && !self.wave_queue && !self.queue.is_empty()).then_some(0)
        }
    }

    fn row_count(&self, section: Section) -> usize {
        match section {
            // Ещё не загруженные лайки занимают своё место: полоса прокрутки не прыгает.
            Section::Likes => self.liked.len() + self.liked_ids.len().saturating_sub(self.likes_from),
            _ => self.list(section).len(),
        }
    }

    /// Какие строки открытого раздела сейчас строятся.
    fn visible(&self) -> Range<usize> {
        match self.section {
            Section::Wave => 0..0,
            Section::Collection => 0..self.liked.len().min(8),
            Section::Artist => 0..self.list(Section::Artist).len(),
            Section::Search if self.search_tab == SearchTab::All => 0..self.results.len().min(SEARCH_ALL_TRACKS),
            Section::Playlists if self.playlist.as_ref().is_some_and(|p| p.source == Source::Album) => {
                visible_rows(self.list_offset - PAGE_HEADER, WINDOW.lock().unwrap().1, self.row_count(Section::Playlists))
            }
            section => visible_rows(self.list_offset, WINDOW.lock().unwrap().1, self.row_count(section)),
        }
    }

    /// Какие строки панели очереди сейчас строятся.
    fn queue_rows(&self) -> Range<usize> {
        if self.queue_open {
            visible_rows(self.queue_offset, WINDOW.lock().unwrap().1, self.queue.len())
        } else {
            0..0
        }
    }

    fn visible_covers(&self) -> Task<Message> {
        let range = self.visible();
        let jobs = self.list(self.section).iter().skip(range.start).take(range.len()).filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect();
        let list = self.fetch_cover_jobs(jobs, "100x100");
        let range = self.queue_rows();
        let jobs = self.queue.iter().skip(range.start).take(range.len()).filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect();
        let queue = self.fetch_cover_jobs(jobs, "100x100");
        // Играющий трек показан и в плеере, и крупно на главной.
        let current = self.current_track().and_then(|t| Some((t.id.clone(), t.cover.clone()?))).into_iter().collect();
        Task::batch([list, queue, self.fetch_cover_jobs(current, "300x300")])
    }

    fn token(&self) -> Option<String> {
        match &self.screen {
            Screen::Main(token) => Some(token.clone()),
            Screen::Login(_) => None,
        }
    }

    fn fail_retry(&mut self, error: String, retry: Message) -> Task<Message> {
        let task = self.fail(error);
        if self.error.is_some() {
            self.retry = Some(retry);
        }
        task
    }

    fn fail(&mut self, error: String) -> Task<Message> {
        self.retry = None;
        if error == NEED_LOGIN {
            self.reset();
            self.error = Some("Вход устарел, войдите ещё раз".into());
            return Task::done(Message::LoginOpen);
        }
        self.error = Some(error);
        Task::none()
    }

    /// Выход из аккаунта: чужих данных в окне оставаться не должно.
    fn reset(&mut self) {
        if let Some(p) = &self.player {
            p.stop();
        }
        api::logout();
        self.screen = Screen::Login(false);
        self.section = Section::Wave;
        self.history.clear();
        self.profile = None;
        self.results.clear();
        self.found = Found::default();
        self.searched = false;
        self.liked_albums.clear();
        self.liked_artists.clear();
        self.queue.clear();
        self.liked.clear();
        self.liked_ids.clear();
        self.liked_set.clear();
        self.disliked.clear();
        self.menu = None;
        self.notice = None;
        self.covers.clear();
        self.playlists.clear();
        self.playlist = None;
        self.playlist_tracks.clear();
        self.requested.borrow_mut().clear();
        self.likes_from = 0;
        self.album = None;
        self.artist = None;
        self.artist_loading = None;
        self.preloaded = None;
        self.list_offset = 0.0;
        self.retry = None;
        self.unshuffled = None;
        self.queue_open = false;
        self.quality_menu = None;
        self.codec = None;
        self.uid = None;
        self.wave = None;
        self.wave_pending = None;
        self.queue_title = None;
        self.pins.clear();
        self.wave_queue = false;
        self.current = None;
        self.active = false;
        self.starting = false;
    }

    /// Обложки карточек и плейлистов из результатов поиска и коллекции.
    fn fetch_card_covers<'a>(&self, albums: impl Iterator<Item = &'a Card>, artists: impl Iterator<Item = &'a Card>, playlists: &[Playlist]) -> Task<Message> {
        let albums = albums.filter_map(|c| Some((format!("{ALBUM_COVER}{}", c.id), c.cover.clone()?)));
        let artists = artists.filter_map(|c| Some((format!("{ARTIST_COVER}{}", c.id), c.cover.clone()?)));
        let lists = playlists.iter().filter_map(|p| Some((playlist_cover_key(p), p.cover.clone()?)));
        self.fetch_cover_jobs(albums.chain(artists).chain(lists).collect(), "300x300")
    }

    fn search_run(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let text = self.query.trim().to_string();
        self.search_id += 1;
        if text.is_empty() {
            self.searching = false;
            self.searched = false;
            self.results.clear();
            self.found = Found::default();
            return Task::none();
        }
        self.searching = true;
        self.error = None;
        let id = self.search_id;
        let tab = self.search_tab;
        let size = if tab == SearchTab::All { SEARCH_ALL } else { SEARCH_TAB };
        Task::perform(background(move || api::search_mixed(&text, tab.types(), size, &token)), move |r| Message::Found(id, r))
    }

    /// Экран волны показывает обложку крупно.
    fn fetch_wave_covers(&self, tracks: &[Track]) -> Task<Message> {
        self.fetch_cover_jobs(tracks.iter().filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect(), "300x300")
    }

    fn fetch_cover_jobs(&self, jobs: Vec<(String, String)>, size: &'static str) -> Task<Message> {
        let mut requested = self.requested.borrow_mut();
        let jobs: Vec<(String, String)> = jobs.into_iter().filter(|(id, _)| !self.covers.contains_key(id) && requested.insert(id.clone())).collect();
        if jobs.is_empty() {
            return Task::none();
        }
        let (tx, rx) = mpsc::unbounded();
        let jobs = Arc::new(Mutex::new(jobs));
        for _ in 0..COVER_WORKERS {
            let (tx, jobs) = (tx.clone(), jobs.clone());
            std::thread::spawn(move || loop {
                let Some((id, uri)) = jobs.lock().unwrap().pop() else { break };
                if let Some(bytes) = api::cover(&uri, size) {
                    if tx.unbounded_send((id, bytes)).is_err() {
                        break;
                    }
                }
            });
        }
        Task::stream(rx).map(|(id, bytes)| Message::Cover(id, bytes))
    }

    /// Обложки остаются только у того, что на экране, рядом с ним и рядом в очереди.
    fn prune_covers(&mut self) {
        let range = self.visible();
        let from = range.start.saturating_sub(COVERS_AROUND);
        let around = self.list(self.section).iter().skip(from).take(range.end + COVERS_AROUND - from);
        let index = self.current.unwrap_or(0);
        let queue = self.queue.iter().skip(index.saturating_sub(QUEUE_COVERS_AROUND)).take(QUEUE_COVERS_AROUND * 2 + 1);
        let panel = self.queue_rows();
        let panel = self.queue.iter().skip(panel.start).take(panel.len());
        let keep: HashSet<String> = around.chain(queue).chain(panel).map(|t| t.id.clone()).collect();
        let mut cards: HashSet<String> = HashSet::new();
        let page = self.artist.as_ref();
        cards.extend(self.liked_albums.iter().chain(&self.found.albums).chain(page.into_iter().flat_map(|a| a.albums.iter().chain(&a.also))).map(|c| format!("{ALBUM_COVER}{}", c.id)));
        cards.extend(self.liked_artists.iter().chain(&self.found.artists).chain(page.into_iter().flat_map(|a| &a.similar)).map(|c| format!("{ARTIST_COVER}{}", c.id)));
        cards.extend(self.album.iter().map(|a| format!("{ALBUM_COVER}{}", a.id)));
        cards.extend(page.iter().map(|a| format!("{ARTIST_COVER}{}", a.id)));
        self.covers.retain(|id, _| id.starts_with(PLAYLIST_COVER) || id.starts_with(PIN_COVER_PREFIX) || cards.contains(id) || keep.contains(id));
        self.requested.borrow_mut().retain(|id| self.covers.contains_key(id));
    }

    /// Волне важно знать, что с треком стало. Ошибка здесь музыке не мешает.
    fn feedback(&self, kind: &'static str, track: Option<&Track>, played: Option<f32>) {
        let (Some(session), Some(token)) = (self.wave.clone(), self.token()) else { return };
        let track = track.cloned();
        std::thread::spawn(move || {
            let _ = api::wave_feedback(&session, kind, track.as_ref(), played, &token);
        });
    }

    fn current_track(&self) -> Option<&Track> {
        self.queue.get(self.current?)
    }

    fn play(&mut self, index: usize) -> Task<Message> {
        let (Some(player), Some(token), Some(track)) = (self.player.clone(), self.token(), self.queue.get(index).cloned()) else {
            return Task::none();
        };
        if let Some(session) = self.wave_pending.take() {
            self.wave = Some(session);
            self.feedback("radioStarted", None, None);
        }
        self.retry = None;
        self.codec = None;
        let generation = player.stop();
        self.play_id = generation;
        self.current = Some(index);
        self.active = false;
        self.starting = true;
        self.position = Duration::ZERO;
        self.seeking = None;
        self.error = None;
        self.feedback("trackStarted", Some(&track), None);
        let id = track.id.clone();
        let quality = self.quality;
        let start = Task::perform(
            background(move || {
                if let Some(started) = player.play_prepared(generation, &preload_key(&id, quality)) {
                    return started;
                }
                let info = api::file_info(&id, quality.code(), &token)?;
                player.play(generation, &info)?;
                Ok(info.codec)
            }),
            move |result| Message::Started(generation, result),
        );
        Task::batch([start, self.wave_refill()])
    }

    /// Незадолго до конца трека начинает качать следующий. Ошибка не важна: при запуске
    /// файл просто запросится обычным путём.
    fn preload_next(&mut self) {
        let (Some(track), Some(player), Some(token)) = (self.current_track(), self.player.clone(), self.token()) else { return };
        if track.duration.saturating_sub(self.position) > PRELOAD_BEFORE || (self.repeat == Repeat::One && !self.wave_queue) {
            return;
        }
        let Some(next) = self.next_index().and_then(|i| self.queue.get(i)) else { return };
        let (id, quality) = (next.id.clone(), self.quality);
        let key = preload_key(&id, quality);
        if self.preloaded.as_ref() == Some(&key) {
            return;
        }
        self.preloaded = Some(key.clone());
        std::thread::spawn(move || {
            if let Ok(info) = api::file_info(&id, quality.code(), &token) {
                let _ = player.preload(&key, &info);
            }
        });
    }

    /// Перед концом очереди просит у волны следующую пачку.
    fn wave_refill(&mut self) -> Task<Message> {
        let (Some(session), Some(token), Some(index)) = (self.wave.clone(), self.token(), self.current) else { return Task::none() };
        if self.wave_fetching || self.queue.len() - index > WAVE_MARGIN + 1 {
            return Task::none();
        }
        self.wave_fetching = true;
        let start = self.queue.len().saturating_sub(WAVE_HISTORY);
        let shown: Vec<String> = self.queue[start..].iter().map(Track::full_id).collect();
        Task::perform(background(move || api::wave_more(&session, &shown, &token)), Message::WaveMore)
    }

    fn liked_pages_load(&self) -> Task<Message> {
        let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
        Task::perform(background(move || api::liked_pages(&uid, &token)), Message::LikedPages)
    }

    fn remember(&mut self) {
        self.history.push(Place { section: self.section, playlist: self.playlist.clone(), artist: self.artist.clone() });
    }

    fn open_list(&mut self, playlist: Playlist) -> Task<Message> {
        self.remember();
        self.load_list(playlist)
    }

    fn load_list(&mut self, playlist: Playlist) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.section = Section::Playlists;
        self.error = None;
        self.playlist = Some(playlist.clone());
        self.playlist_tracks.clear();
        self.album = None;
        let kind = playlist.kind.clone();
        if playlist.source == Source::Album {
            return Task::perform(background(move || api::album(&playlist.kind, &token)), move |r| Message::AlbumLoaded(kind.clone(), r));
        }
        Task::perform(background(move || api::playlist_tracks(&playlist, &token)), move |r| Message::PlaylistLoaded(kind.clone(), r))
    }

    fn wave_load(&mut self, seeds: Vec<String>, autoplay: bool) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.wave_loading = true;
        self.error = None;
        Task::perform(background(move || api::wave_new(&seeds, &token)), move |r| Message::WaveStarted(autoplay, r))
    }

    /// Показывает уведомление и убирает его через несколько секунд.
    fn notify(&mut self, text: String) -> Task<Message> {
        let id = self.notice.as_ref().map_or(1, |(id, _)| id + 1);
        self.notice = Some((id, text));
        Task::perform(
            background(|| std::thread::sleep(NOTICE_TIME)),
            move |()| Message::NoticeClear(id),
        )
    }

    fn likes_more(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let from = self.likes_from;
        if self.likes_loading || from >= self.liked_ids.len() || self.uid.is_none() {
            return Task::none();
        }
        self.likes_loading = true;
        let page = self.liked_ids[from..(from + LIKES_PAGE).min(self.liked_ids.len())].to_vec();
        let size = page.len();
        Task::perform(background(move || api::tracks_by_ids(&page, &token)), move |r| Message::LikesPage(size, r))
    }

    fn apply_like(&mut self, track: &Track, liked: bool) {
        if liked {
            self.liked_set.insert(track.id.clone());
            self.liked_ids.insert(0, track.id.clone());
            // Пока список не загружали, трек придёт вместе с первой страницей.
            if !self.liked.is_empty() {
                self.liked.insert(0, track.clone());
                self.likes_from += 1;
            }
        } else {
            self.liked_set.remove(&track.id);
            self.liked_ids.retain(|id| *id != track.id);
            if self.liked.iter().any(|t| t.id == track.id) {
                self.likes_from = self.likes_from.saturating_sub(1);
            }
            self.liked.retain(|t| t.id != track.id);
        }
    }

    fn list(&self, section: Section) -> &[Track] {
        match section {
            Section::Wave => &self.queue,
            Section::Likes | Section::Collection => &self.liked,
            Section::Playlists => &self.playlist_tracks,
            Section::Artist => self.artist.as_ref().map_or(&[], |a| a.popular.as_slice()),
            Section::Albums | Section::Artists => &[],
            Section::Search => &self.results,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let place = self.list_place();
        let task = self.handle(message);
        // Другой список открывается с начала.
        let snap = if place == self.list_place() {
            Task::none()
        } else {
            self.list_offset = 0.0;
            operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START)
        };
        // Лайки догружаются, когда до конца загруженного остаётся немного.
        let more = if self.section == Section::Likes && self.visible().end + ROW_MARGIN >= self.liked.len() && !matches!(self.retry, Some(Message::LikesMore)) {
            self.likes_more()
        } else {
            Task::none()
        };
        if self.covers.len() > COVERS_LIMIT {
            self.prune_covers();
        }
        let covers = self.visible_covers();
        let track = self.current_track().cloned();
        let playing = self.active && !self.player.as_ref().is_none_or(Player::is_paused);
        let position = self.player.as_ref().filter(|_| self.active).map_or(Duration::ZERO, Player::position);
        if let Some(media) = &mut self.media {
            media.sync(track.as_ref(), playing, position);
        }
        Task::batch([task, snap, more, covers])
    }

    fn list_place(&self) -> (Section, Option<String>, Option<String>) {
        (self.section, self.playlist.as_ref().map(|p| p.kind.clone()), self.artist_loading.clone().or(self.artist.as_ref().map(|a| a.id.clone())))
    }

    fn playlists_load(&mut self) -> Task<Message> {
        let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
        if self.playlists_loading {
            return Task::none();
        }
        self.playlists_loading = true;
        Task::perform(background(move || api::playlists(&uid, &token)), Message::PlaylistsLoaded)
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoginOpen => {
                if matches!(self.screen, Screen::Login(true)) {
                    return Task::none();
                }
                self.screen = Screen::Login(true);
                return Task::perform(background(login_window), |_| Message::LoginFinished);
            }
            Message::LoginFinished => {
                if let Some(token) = api::load_token() {
                    self.error = None;
                    self.screen = Screen::Main(token.clone());
                    return load_account(token);
                }
                self.screen = Screen::Login(false);
            }
            Message::Logout => {
                self.reset();
                return Task::done(Message::LoginOpen);
            }
            Message::Account(Ok((profile, ids, disliked))) => {
                self.disliked = disliked.into_iter().collect();
                self.uid = Some(profile.uid.clone());
                self.profile = Some(profile);
                self.liked_set = ids.iter().cloned().collect();
                self.liked_ids = ids;
                let pins = self.token().map_or(Task::none(), |token| Task::perform(background(move || api::pins(&token)), Message::PinsLoaded));
                let pins = Task::batch([pins, self.liked_pages_load()]);
                let page = match self.preview.clone() {
                    Some(Preview::Wave | Preview::WaveSeek) => self.wave_load(vec![api::MY_WAVE.to_string()], false),
                    Some(Preview::Likes | Preview::LikesEnd | Preview::Menu | Preview::Panels) => self.update(Message::Open(Section::Likes)),
                    Some(Preview::Collection) => self.update(Message::Open(Section::Collection)),
                    Some(Preview::Playlists | Preview::Playlist | Preview::PlaylistHome) => self.update(Message::Open(Section::Playlists)),
                    Some(Preview::Album(id)) => {
                        self.preview = None;
                        self.open_list(Playlist::album(&id, ""))
                    }
                    Some(Preview::Artist(id)) => {
                        self.preview = None;
                        self.update(Message::ArtistOpen(id))
                    }
                    Some(Preview::Search(query)) => {
                        self.section = Section::Search;
                        self.query = query;
                        self.update(Message::Search)
                    }
                    None if self.section == Section::Likes => self.likes_more(),
                    // Главная сразу показывает трек и управление, как в оригинале.
                    None if self.queue.is_empty() => self.wave_load(vec![api::MY_WAVE.to_string()], false),
                    None => Task::none(),
                };
                return Task::batch([pins, page]);
            }
            Message::PinsLoaded(Ok(pins)) => {
                self.pins = pins;
                let jobs = self.pins.iter().enumerate().filter_map(|(i, p)| Some((format!("{PIN_COVER_PREFIX}{i}"), p.cover.clone()?))).collect();
                return self.fetch_cover_jobs(jobs, "100x100");
            }
            // Закрепления не должны мешать музыке.
            Message::PinsLoaded(Err(_)) => {}
            Message::PinOpen(index) => match self.pins.get(index).map(|p| p.target.clone()) {
                Some(PinTarget::List(list)) => {
                    let favorites = list.source == Source::Playlist && list.kind == "3" && self.uid.as_deref() == Some(list.owner.as_str());
                    if favorites {
                        return self.update(Message::Open(Section::Likes));
                    }
                    return self.open_list(list);
                }
                Some(PinTarget::Wave(seeds)) => {
                    if seeds.len() == 1 && seeds[0] == api::MY_WAVE {
                        return self.update(Message::Open(Section::Wave));
                    }
                    return self.update(Message::WaveStartSeeds(seeds));
                }
                None => {}
            },
            Message::Account(Err(e)) => return self.fail(e),
            Message::Back => {
                if let Some(place) = self.history.pop() {
                    self.section = place.section;
                    self.artist = place.artist;
                    self.artist_loading = None;
                    if place.section == Section::Playlists {
                        if let Some(playlist) = place.playlist {
                            return self.load_list(playlist);
                        }
                        self.playlist = None;
                        self.album = None;
                        self.playlist_tracks.clear();
                    }
                    self.prune_covers();
                }
            }
            Message::BarHover(hover) => self.bar_hover = hover,
            Message::Open(section) => {
                if self.section != section {
                    self.remember();
                }
                self.section = section;
                self.error = None;
                self.retry = None;
                if section == Section::Playlists {
                    self.playlist = None;
                }
                return match section {
                    Section::Collection => {
                        let likes = if self.liked.is_empty() { self.likes_more() } else { Task::none() };
                        let lists = if self.playlists.is_empty() { self.playlists_load() } else { Task::none() };
                        Task::batch([likes, lists])
                    }
                    Section::Likes if self.liked.is_empty() => self.likes_more(),
                    Section::Playlists if self.playlists.is_empty() => self.playlists_load(),
                    Section::Search => operation::focus(Id::new(SEARCH_INPUT)),
                    _ => Task::none(),
                };
            }
            Message::Query(query) => {
                self.query = query;
                // Новая буква отменяет поиск, который ещё ждёт паузы.
                self.search_id += 1;
                let id = self.search_id;
                return Task::perform(background(|| std::thread::sleep(SEARCH_DELAY)), move |()| Message::SearchTyped(id));
            }
            Message::SearchTyped(id) => {
                if id == self.search_id {
                    return self.search_run();
                }
            }
            Message::Search => return self.search_run(),
            Message::SearchTabSet(tab) => {
                if self.search_tab != tab {
                    self.search_tab = tab;
                    self.list_offset = 0.0;
                    let snap = operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START);
                    return Task::batch([snap, self.search_run()]);
                }
            }
            Message::ListOpen(playlist) => return self.open_list(playlist),
            Message::LikedPages(Ok((albums, artists))) => {
                self.liked_albums = albums;
                self.liked_artists = artists;
                return self.fetch_card_covers(self.liked_albums.iter(), self.liked_artists.iter(), &[]);
            }
            // Коллекция покажется без этих блоков; музыке это не мешает.
            Message::LikedPages(Err(_)) => {}
            Message::AlbumLike => {
                let (Some(uid), Some(token), Some(album)) = (self.uid.clone(), self.token(), self.album.clone()) else { return Task::none() };
                let liked = !self.liked_albums.iter().any(|c| c.id == album.id);
                if liked {
                    let subtitle = album.year.map_or_else(|| "Альбом".to_string(), |year| year.to_string());
                    self.liked_albums.insert(0, Card { id: album.id.clone(), title: album.title.clone(), subtitle, cover: album.cover.clone() });
                } else {
                    self.liked_albums.retain(|c| c.id != album.id);
                }
                return Task::perform(background(move || api::set_album_liked(&uid, &album.id, liked, &token)), Message::PageLikeSaved);
            }
            Message::ArtistLike => {
                let (Some(uid), Some(token), Some(artist)) = (self.uid.clone(), self.token(), self.artist.clone()) else { return Task::none() };
                let liked = !self.liked_artists.iter().any(|c| c.id == artist.id);
                if liked {
                    self.liked_artists.insert(0, Card { id: artist.id.clone(), title: artist.name.clone(), subtitle: "Исполнитель".to_string(), cover: artist.cover.clone() });
                } else {
                    self.liked_artists.retain(|c| c.id != artist.id);
                }
                return Task::perform(background(move || api::set_artist_liked(&uid, &artist.id, liked, &token)), Message::PageLikeSaved);
            }
            Message::PageLikeSaved(Ok(())) => {}
            Message::PageLikeSaved(Err(e)) => {
                // Яндекс не сохранил: показываем то, что у него на самом деле.
                let reload = self.liked_pages_load();
                return Task::batch([reload, self.fail(e)]);
            }
            Message::Found(id, result) => {
                if id != self.search_id {
                    return Task::none();
                }
                self.searching = false;
                match result {
                    Ok(found) => {
                        self.results = found.tracks.clone();
                        self.found = found;
                        self.searched = true;
                        self.prune_covers();
                        if matches!(self.preview.take(), Some(Preview::Search(_))) {
                            self.queue = self.results.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        return self.fetch_card_covers(self.found.albums.iter(), self.found.artists.iter(), &self.found.playlists);
                    }
                    Err(e) => return self.fail_retry(e, Message::Search),
                }
            }
            Message::LikesMore => return self.likes_more(),
            Message::Scrolled(offset) => self.list_offset = offset,
            Message::Retry => {
                if let Some(message) = self.retry.take() {
                    self.error = None;
                    return self.update(message);
                }
            }
            Message::ListReload => {
                if let Some(playlist) = self.playlist.clone() {
                    return self.load_list(playlist);
                }
            }
            Message::AlbumOpen(id, title) => return self.open_list(Playlist::album(&id, &title)),
            Message::AlbumLoaded(kind, result) => match result {
                Ok((album, tracks)) => {
                    if self.playlist.as_ref().is_none_or(|p| p.kind != kind) {
                        return Task::none();
                    }
                    if let Some(playlist) = &mut self.playlist {
                        playlist.title = album.title.clone();
                    }
                    let cover = album.cover.clone().map(|uri| (format!("{ALBUM_COVER}{}", album.id), uri)).into_iter().collect();
                    self.album = Some(album);
                    let cover = self.fetch_cover_jobs(cover, "300x300");
                    return Task::batch([cover, self.handle(Message::PlaylistLoaded(kind, Ok(tracks)))]);
                }
                Err(e) => return self.handle(Message::PlaylistLoaded(kind, Err(e))),
            },
            Message::ArtistOpen(id) => {
                self.remember();
                self.section = Section::Artist;
                self.artist = None;
                return self.handle(Message::ArtistLoad(id));
            }
            Message::ArtistLoad(id) => {
                let Some(token) = self.token() else { return Task::none() };
                self.error = None;
                self.artist_loading = Some(id.clone());
                let wanted = id.clone();
                return Task::perform(background(move || api::artist(&id, &token)), move |r| Message::ArtistLoaded(wanted.clone(), r));
            }
            Message::ArtistLoaded(id, result) => {
                // Пока грузился, могли уйти на другую страницу.
                if self.artist_loading.as_ref() != Some(&id) {
                    return Task::none();
                }
                self.artist_loading = None;
                match result {
                    Ok(artist) => {
                        let albums = artist.albums.iter().chain(&artist.also).filter_map(|c| Some((format!("{ALBUM_COVER}{}", c.id), c.cover.clone()?)));
                        let artists = artist.similar.iter().filter_map(|c| Some((format!("{ARTIST_COVER}{}", c.id), c.cover.clone()?)));
                        let main = artist.cover.clone().map(|uri| (format!("{ARTIST_COVER}{}", artist.id), uri));
                        let jobs = main.into_iter().chain(artists).chain(albums).collect();
                        self.artist = Some(artist);
                        return self.fetch_cover_jobs(jobs, "300x300");
                    }
                    Err(e) => return self.fail_retry(e, Message::ArtistLoad(id)),
                }
            }
            Message::ArtistTracks => {
                if let Some(artist) = &self.artist {
                    let list = Playlist::artist(&artist.id, &artist.name);
                    return self.open_list(list);
                }
            }
            Message::ToggleMute => {
                self.muted = !self.muted;
                self.apply_volume();
                self.save_settings();
            }
            Message::RepeatNext => {
                self.repeat = self.repeat.next();
                self.save_settings();
            }
            Message::ShuffleToggle => {
                self.shuffle = !self.shuffle;
                self.save_settings();
                if self.shuffle {
                    if let Some(index) = self.current {
                        self.unshuffled = Some(self.queue.clone());
                        shuffled(&mut self.queue, index, random_seed());
                        self.current = Some(0);
                    }
                } else if let Some(original) = self.unshuffled.take() {
                    let id = self.current_track().map(|t| t.id.clone());
                    self.current = original.iter().position(|t| Some(&t.id) == id.as_ref()).or(self.current.filter(|i| *i < original.len()));
                    self.queue = original;
                }
            }
            Message::LikesPage(size, result) => {
                self.likes_loading = false;
                match result {
                    Ok(page) => {
                        self.likes_from += size;
                        self.liked.extend(page);
                        if matches!(self.preview, Some(Preview::Collection)) {
                            self.preview = None;
                        }
                        if matches!(self.preview, Some(Preview::Menu)) {
                            self.preview = None;
                            self.menu = self.liked.first().cloned().map(|t| (t, 700.0, 200.0));
                            self.seeking = Some(60.0);
                            self.queue = self.liked.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        if matches!(self.preview, Some(Preview::Panels)) {
                            self.preview = None;
                            self.queue = self.liked.clone();
                            self.current = Some(20).filter(|i| *i < self.queue.len());
                            self.quality_menu = Some((1000.0, 700.0));
                            return self.update(Message::QueueToggle);
                        }
                        if matches!(self.preview, Some(Preview::LikesEnd)) {
                            self.preview = None;
                            return operation::snap_to(Id::new(LIST), scrollable::RelativeOffset { x: 0.0, y: 1.0 });
                        }
                        if matches!(self.preview.take(), Some(Preview::Likes)) {
                            self.queue = self.liked.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                    }
                    Err(e) => return self.fail_retry(e, Message::LikesMore),
                }
            }
            Message::Like(track) => {
                self.menu = None;
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let liked = !self.liked_set.contains(&track.id);
                self.apply_like(&track, liked);
                if self.queue.iter().any(|t| t.id == track.id) {
                    self.feedback(if liked { "like" } else { "unlike" }, Some(&track), None);
                }
                let id = track.id.clone();
                return Task::perform(background(move || api::set_liked(&uid, &id, liked, &token)), move |r| Message::LikeSaved(track.clone(), liked, r));
            }
            Message::LikeSaved(_, _, Ok(())) => {}
            Message::LikeSaved(track, liked, Err(e)) => {
                // Яндекс не сохранил: возвращаем как было.
                self.apply_like(&track, !liked);
                return self.fail(e);
            }
            Message::WaveStart => return self.update(Message::WaveStartSeeds(vec![api::MY_WAVE.to_string()])),
            Message::WaveStartSeeds(seeds) => {
                if self.section != Section::Wave {
                    self.remember();
                    self.section = Section::Wave;
                }
                return self.wave_load(seeds, true);
            }
            Message::WaveStarted(autoplay, result) => {
                self.wave_loading = false;
                match result {
                    // Пока грузилась фоновая волна, пользователь мог включить свой список.
                    Ok(_) if !autoplay && self.current.is_some() && !matches!(self.preview, Some(Preview::Wave | Preview::WaveSeek)) => {}
                    Ok(wave) => {
                        if let Some(p) = &self.player {
                            p.stop();
                        }
                        self.active = false;
                        self.starting = false;
                        self.wave = None;
                        self.wave_pending = Some(wave.session);
                        self.queue_title = None;
                        self.queue = wave.tracks;
                        self.wave_queue = true;
                        self.unshuffled = None;
                        self.wave_fetching = false;
                        self.current = Some(0).filter(|_| !self.queue.is_empty());
                        self.prune_covers();
                        let covers = self.fetch_wave_covers(&self.queue);
                        if matches!(self.preview, Some(Preview::WaveSeek)) {
                            self.seeking = Some(5.0);
                        }
                        if matches!(self.preview, Some(Preview::Wave | Preview::WaveSeek)) {
                            self.preview = None;
                        }
                        if autoplay {
                            return Task::batch([covers, self.play(0)]);
                        }
                        return covers;
                    }
                    Err(e) => return self.fail_retry(e, Message::WaveStart),
                }
            }
            Message::WaveMore(result) => {
                self.wave_fetching = false;
                match result {
                    Ok(tracks) if self.wave.is_some() => {
                        let known: HashSet<String> = self.queue.iter().map(|t| t.id.clone()).collect();
                        let fresh: Vec<Track> = tracks.into_iter().filter(|t| !known.contains(&t.id)).collect();
                        let task = self.fetch_wave_covers(&fresh);
                        self.queue.extend(fresh);
                        return task;
                    }
                    Ok(_) => {}
                    Err(e) => return self.fail(e),
                }
            }
            Message::PlaylistsLoaded(result) => {
                self.playlists_loading = false;
                match result {
                    Ok(playlists) => {
                        self.playlists = playlists;
                        let jobs = self.playlists.iter().filter_map(|p| Some((playlist_cover_key(p), p.cover.clone()?))).collect();
                        let covers = self.fetch_cover_jobs(jobs, "200x200");
                        if matches!(self.preview, Some(Preview::Playlists)) {
                            self.preview = None;
                        }
                        if matches!(self.preview, Some(Preview::Playlist | Preview::PlaylistHome)) {
                            return Task::batch([covers, self.update(Message::PlaylistOpen(0))]);
                        }
                        return covers;
                    }
                    Err(e) => return self.fail_retry(e, Message::Open(self.section)),
                }
            }
            Message::PlaylistOpen(index) => {
                if let Some(playlist) = self.playlists.get(index).cloned() {
                    return self.open_list(playlist);
                }
            }
            Message::PlaylistLoaded(kind, result) => {
                // Пока грузился, могли открыть другой.
                if self.playlist.as_ref().is_none_or(|p| p.kind != kind) {
                    return Task::none();
                }
                match result {
                    Ok(tracks) => {
                        self.playlist_tracks = tracks;
                        if let Some(playlist) = &mut self.playlist {
                            playlist.count = self.playlist_tracks.len() as u64;
                        }
                        let preview = self.preview.take();
                        if matches!(preview, Some(Preview::Playlist | Preview::PlaylistHome)) {
                            self.queue = self.playlist_tracks.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        if matches!(preview, Some(Preview::PlaylistHome)) {
                            self.queue_title = self.playlist.as_ref().map(|p| p.title.clone());
                            self.section = Section::Wave;
                        }
                    }
                    Err(e) => return self.fail_retry(e, Message::ListReload),
                }
            }
            Message::MenuOpen(track) => {
                let (x, y) = *CURSOR.lock().unwrap();
                self.menu = Some((track, x, y));
            }
            Message::MenuClose => {
                self.menu = None;
                self.quality_menu = None;
            }
            Message::QualityOpen => self.quality_menu = Some(*CURSOR.lock().unwrap()),
            Message::QualitySet(quality) => {
                self.quality = quality;
                self.quality_menu = None;
                self.save_settings();
            }
            Message::QueueToggle => {
                self.queue_open = !self.queue_open;
                if self.queue_open {
                    self.queue_offset = (self.current.unwrap_or(0) as f32 * ROW - QUEUE_ABOVE).max(0.0);
                    return operation::scroll_to(Id::new(QUEUE_LIST), scrollable::AbsoluteOffset { x: 0.0, y: self.queue_offset });
                }
            }
            Message::QueueScrolled(offset) => self.queue_offset = offset,
            Message::QueuePlay(index) => {
                self.skip_feedback();
                return self.play(index);
            }
            Message::QueueRemove(index) => {
                if index < self.queue.len() && self.current != Some(index) {
                    let track = self.queue.remove(index);
                    if self.current.is_some_and(|i| index < i) {
                        self.current = self.current.map(|i| i - 1);
                    }
                    if let Some(original) = &mut self.unshuffled {
                        if let Some(at) = original.iter().position(|t| t.id == track.id) {
                            original.remove(at);
                        }
                    }
                }
            }
            Message::Dislike(track) => {
                self.menu = None;
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let disliked = !self.disliked.contains(&track.id);
                if disliked {
                    self.disliked.insert(track.id.clone());
                } else {
                    self.disliked.remove(&track.id);
                }
                if self.queue.iter().any(|t| t.id == track.id) {
                    self.feedback(if disliked { "dislike" } else { "undislike" }, Some(&track), None);
                }
                let text = if disliked { "Трек больше не будет появляться в ваших рекомендациях" } else { "Теперь трек сможет появляться в ваших рекомендациях" };
                let notice = self.notify(text.to_string());
                let id = track.id.clone();
                let saved = id.clone();
                let save = Task::perform(background(move || api::set_disliked(&uid, &id, disliked, &token)), move |r| Message::DislikeSaved(saved.clone(), disliked, r));
                // Как в оригинале: дизлайк играющего трека сразу переключает на следующий.
                let next = if disliked && self.current_track().is_some_and(|t| t.id == track.id) {
                    self.current.filter(|i| i + 1 < self.queue.len()).map_or(Task::none(), |i| self.play(i + 1))
                } else {
                    Task::none()
                };
                return Task::batch([notice, save, next]);
            }
            Message::DislikeSaved(_, _, Ok(())) => {}
            Message::DislikeSaved(id, disliked, Err(e)) => {
                if disliked {
                    self.disliked.remove(&id);
                } else {
                    self.disliked.insert(id);
                }
                return self.fail(e);
            }
            Message::PlayNext(track) => {
                self.menu = None;
                let notice = self.notify(format!("Трек {} добавлен в начало очереди", track.title));
                let at = self.current.map_or(0, |i| i + 1);
                if let Some(original) = &mut self.unshuffled {
                    original.push(track.clone());
                }
                self.queue.insert(at.min(self.queue.len()), track);
                let start = if self.current.is_none() { self.play(0) } else { Task::none() };
                return Task::batch([notice, start]);
            }
            Message::PlayLast(track) => {
                self.menu = None;
                let notice = self.notify(format!("Трек {} добавлен в конец очереди", track.title));
                if let Some(original) = &mut self.unshuffled {
                    original.push(track.clone());
                }
                self.queue.push(track);
                let start = if self.current.is_none() { self.play(0) } else { Task::none() };
                return Task::batch([notice, start]);
            }
            Message::TrackWave(track) => {
                self.menu = None;
                return self.update(Message::WaveStartSeeds(vec![format!("track:{}", track.id)]));
            }
            Message::OpenAlbum(track) => {
                self.menu = None;
                if let Some(album) = &track.album {
                    return self.open_list(Playlist::album(album, &track.album_title));
                }
            }
            Message::OpenArtist(track) => {
                self.menu = None;
                if let Some(artist) = track.artist_id.clone() {
                    return self.update(Message::ArtistOpen(artist));
                }
            }
            Message::CopyLink(track) => {
                self.menu = None;
                let notice = self.notify("Ссылка на трек скопирована".to_string());
                return Task::batch([iced::clipboard::write(track.share_link()), notice]);
            }
            Message::NoticeClear(id) => {
                if self.notice.as_ref().is_some_and(|(current, _)| *current == id) {
                    self.notice = None;
                }
            }
            Message::WindowReady(hwnd) => {
                if let Some(hwnd) = hwnd {
                    black_caption(hwnd);
                }
                self.media = hwnd.map(Media::new);
            }
            Message::Media(event) => {
                let playing = self.active && !self.player.as_ref().is_none_or(Player::is_paused);
                match event {
                    media::Event::Play if !playing => return self.update(Message::Toggle),
                    media::Event::Pause if playing => return self.update(Message::Toggle),
                    media::Event::Play | media::Event::Pause => {}
                    media::Event::Toggle => return self.update(Message::Toggle),
                    media::Event::Next => return self.update(Message::Next),
                    media::Event::Previous => return self.update(Message::Previous),
                    media::Event::Seek(position) => {
                        self.seeking = Some(position.as_secs_f32());
                        return self.update(Message::SeekDone);
                    }
                }
            }
            Message::Cover(id, bytes) => {
                self.covers.insert(id, image::Handle::from_bytes(bytes));
            }
            Message::Play(section, index) => {
                if section == Section::Wave {
                    // Выбрали трек в очереди волны: сессия продолжается.
                    self.skip_feedback();
                } else {
                    self.queue = self.list(section).to_vec();
                    self.wave = None;
                    self.wave_pending = None;
                    self.wave_queue = false;
                    self.queue_title = Some(match section {
                        Section::Playlists => self.playlist.as_ref().map_or_else(|| "Плейлист".to_string(), |p| p.title.clone()),
                        Section::Search => "Поиск".to_string(),
                        Section::Artist => self.artist.as_ref().map_or_else(String::new, |a| a.name.clone()),
                        _ => "Мне нравится".to_string(),
                    });
                    self.unshuffled = None;
                    if self.shuffle && index < self.queue.len() {
                        self.unshuffled = Some(self.queue.clone());
                        shuffled(&mut self.queue, index, random_seed());
                        return self.play(0);
                    }
                }
                return self.play(index);
            }
            Message::Started(generation, result) => {
                // Пока трек запускался, могли включить другой.
                if generation != self.play_id {
                    return Task::none();
                }
                self.starting = false;
                match result {
                    Ok(codec) => {
                        self.codec = Some(codec);
                        self.active = true;
                        self.failures = 0;
                    }
                    Err(e) => {
                        let task = self.fail(e);
                        self.failures += 1;
                        if let (Screen::Main(_), true, Some(i)) = (&self.screen, self.failures < MAX_FAILURES, self.current.filter(|i| i + 1 < self.queue.len())) {
                            // Трек мог быть недоступен: ошибку показываем, а музыка идёт дальше.
                            let error = self.error.clone();
                            let next = self.play(i + 1);
                            self.error = error;
                            return next;
                        }
                        return task;
                    }
                }
            }
            Message::Toggle => {
                // Трек доиграл: кнопка запускает его заново.
                if let (false, false, Some(index)) = (self.active, self.starting, self.current) {
                    return self.play(index);
                }
                if let Some(p) = &self.player {
                    p.toggle();
                }
            }
            Message::Next => {
                if let Some(next) = self.next_index() {
                    self.skip_feedback();
                    return self.play(next);
                }
            }
            Message::Previous => {
                if self.active && (self.position >= PREVIOUS_RESTART || self.current == Some(0)) {
                    self.seeking = Some(0.0);
                    return self.update(Message::SeekDone);
                }
                if let Some(i) = self.current.filter(|i| *i > 0) {
                    self.skip_feedback();
                    return self.play(i - 1);
                }
            }
            Message::Tick => {
                self.preload_next();
                if let (true, Some(p)) = (self.active, &self.player) {
                    self.position = p.position();
                    if p.is_empty() {
                        self.active = false;
                        if let Some(track) = self.current_track().cloned() {
                            self.feedback("trackFinished", Some(&track), Some(track.duration.as_secs_f32()));
                        }
                        if let (Repeat::One, false, Some(index)) = (self.repeat, self.wave_queue, self.current) {
                            return self.play(index);
                        }
                        if let Some(next) = self.next_index() {
                            return self.play(next);
                        }
                    }
                }
            }
            Message::Seek(value) => self.seeking = Some(value),
            Message::SeekDone => {
                if let (Some(value), Some(p)) = (self.seeking.take(), self.player.clone()) {
                    let position = Duration::from_secs_f32(value.max(0.0));
                    self.position = position;
                    return Task::perform(background(move || p.seek(position)), Message::Seeked);
                }
            }
            Message::Seeked(Err(e)) => self.error = Some(e),
            Message::Seeked(Ok(())) => {}
            Message::Volume(volume) => {
                self.volume = volume;
                self.muted = false;
                self.apply_volume();
                self.save_settings();
            }
        }
        Task::none()
    }

    fn skip_feedback(&self) {
        if let (true, Some(track), Some(p)) = (self.active, self.current_track(), &self.player) {
            self.feedback("skip", Some(track), Some(p.position().as_secs_f32()));
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        match &self.screen {
            Screen::Main(_) if self.active => time::every(TICK).map(|_| Message::Tick),
            _ => Subscription::none(),
        }
        .pipe_media()
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Login(opened) => self.view_login(*opened),
            Screen::Main(_) => self.view_main(),
        }
    }

    fn view_login(&self, opened: bool) -> Element<'_, Message> {
        let mut content = column![text("Tune").size(32).font(ui::font_headline())].spacing(14).align_x(Alignment::Center);
        if opened {
            content = content.push(text("Войдите в аккаунт Яндекса в открывшемся окне").color(ui::MUTED));
        } else {
            content = content
                .push(text("Для музыки нужен аккаунт Яндекса").color(ui::MUTED))
                .push(button(text("Войти через Яндекс").font(ui::font_headline())).padding([10, 20]).style(ui::accent).on_press(Message::LoginOpen));
        }
        if let Some(error) = &self.error {
            content = content.push(text(error).color(ui::DANGER));
        }
        container(content).center(Length::Fill).style(ui::root).into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let page: Element<'_, Message> = match self.section {
            Section::Wave => self.view_wave(),
            Section::Collection => self.view_collection(),
            Section::Likes => self.view_likes(),
            Section::Playlists => self.view_playlists(),
            Section::Artist => self.view_artist(),
            Section::Albums => self.view_card_page("Альбомы", &self.liked_albums, ALBUM_COVER, false, "Здесь появятся альбомы, которые вам понравились"),
            Section::Artists => self.view_card_page("Исполнители", &self.liked_artists, ARTIST_COVER, true, "Здесь появятся исполнители, которые вам понравились"),
            Section::Search => self.view_search(),
        };
        let mut page = column![page].spacing(12).height(Length::Fill);
        if let Some(error) = &self.error {
            let mut line = row![text(error).color(ui::DANGER)].spacing(12).align_y(Alignment::Center);
            if self.retry.is_some() {
                line = line.push(button(text("Повторить").color(ui::TEXT)).padding([6, 14]).style(ui::secondary).on_press(Message::Retry));
            }
            page = page.push(line);
        }
        // Моя волна в оригинале занимает всё окно без панели и нижнего плеера.
        let main: Element<'_, Message> = if self.section == Section::Wave {
            container(page).width(Length::Fill).height(Length::Fill).padding(Padding::ZERO.right(12).bottom(12)).into()
        } else {
            let mut main = column![container(page).padding(24).width(Length::Fill).height(Length::Fill).style(ui::content)].spacing(12);
            if let Some(bar) = self.view_bar() {
                main = main.push(bar);
            }
            container(main).padding(Padding::ZERO.right(12).bottom(12)).width(Length::Fill).height(Length::Fill).into()
        };
        let mut columns = row![self.view_sidebar(), main];
        if self.queue_open {
            columns = columns.push(container(self.view_queue()).padding(Padding::ZERO.right(12).bottom(12)));
        }
        let base = container(columns).width(Length::Fill).height(Length::Fill).style(ui::root);
        let mut layers = stack![base];
        if let Some((track, x, y)) = &self.menu {
            layers = layers.push(mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill)).on_press(Message::MenuClose).on_right_press(Message::MenuClose));
            layers = layers.push(self.view_menu(track, *x, *y));
        }
        if let Some((x, y)) = self.quality_menu {
            layers = layers.push(mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill)).on_press(Message::MenuClose).on_right_press(Message::MenuClose));
            layers = layers.push(self.view_quality(x, y));
        }
        if let Some((_, notice)) = &self.notice {
            layers = layers.push(
                container(container(text(notice).color(ui::TEXT)).padding([12, 18]).style(ui::popover))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Start)
                    .padding(Padding::new(0.0).top(16)),
            );
        }
        layers.into()
    }

    /// Контекстное меню трека: те же пункты и названия, что в оригинале.
    fn view_menu<'a>(&self, track: &'a Track, x: f32, y: f32) -> Element<'a, Message> {
        let item = |icon, label: &'a str, message: Message| {
            button(row![ui::icon(icon, 20.0, ui::MUTED), text(label).color(ui::TEXT)].spacing(12).align_y(Alignment::Center))
                .width(Length::Fill)
                .height(MENU_ITEM)
                .padding([9, 10])
                .style(ui::menu_item)
                .on_press(message)
        };
        let liked = self.liked_set.contains(&track.id);
        let disliked = self.disliked.contains(&track.id);
        let mut items = column![
            item(if liked { ui::HEART } else { ui::HEART_OUTLINE }, if liked { "Убрать из «Мне нравится»" } else { "Нравится" }, Message::Like(track.clone())),
            item(ui::DISLIKE, if disliked { "Убрать дизлайк" } else { "Не нравится" }, Message::Dislike(track.clone())),
            item(ui::WAVE, "Моя волна по треку", Message::TrackWave(track.clone())),
            item(ui::QUEUE_NEXT, "Играть следующим", Message::PlayNext(track.clone())),
            item(ui::QUEUE_LAST, "Добавить в конец очереди", Message::PlayLast(track.clone())),
        ];
        let mut count = 5.0;
        if track.album.is_some() {
            items = items.push(item(ui::ALBUM, "Перейти к альбому", Message::OpenAlbum(track.clone())));
            count += 1.0;
        }
        if track.artist_id.is_some() {
            items = items.push(item(ui::ARTIST, "Перейти к исполнителю", Message::OpenArtist(track.clone())));
            count += 1.0;
        }
        items = items.push(item(ui::LINK, "Скопировать ссылку", Message::CopyLink(track.clone())));
        count += 1.0;
        popup(items.into(), x, y, MENU_WIDTH, count * MENU_ITEM)
    }

    /// Выбор качества. Новое качество действует со следующего трека.
    fn view_quality(&self, x: f32, y: f32) -> Element<'_, Message> {
        let mut items = column![container(text("Качество звука").font(ui::font_medium()).color(ui::TEXT)).padding([6, 10])];
        for quality in Quality::ALL {
            let mark: Element<'_, Message> = if quality == self.quality { ui::icon(ui::CHECK, 20.0, ui::ACCENT) } else { Space::new().width(20).into() };
            items = items.push(
                button(row![mark, text(quality.label()).color(ui::TEXT)].spacing(12).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .height(MENU_ITEM)
                    .padding([9, 10])
                    .style(ui::menu_item)
                    .on_press(Message::QualitySet(quality)),
            );
        }
        if let Some(codec) = &self.codec {
            items = items.push(container(text(format!("Сейчас играет: {}", codec_name(codec))).size(13).color(ui::MUTED)).padding([6, 10]));
        }
        popup(items.into(), x, y, QUALITY_WIDTH, 5.0 * MENU_ITEM)
    }

    /// Панель очереди: как и длинные списки, строит только видимые строки.
    fn view_queue(&self) -> Element<'_, Message> {
        let header = row![
            text("Очередь").size(24).font(ui::font_headline()).color(ui::TEXT),
            text(format!("{} {}", self.queue.len(), tracks_word(self.queue.len()))).color(ui::MUTED),
            Space::new().width(Length::Fill),
            button(ui::icon(ui::CLOSE, 20.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(Message::QueueToggle),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let range = self.queue_rows();
        let mut list = column![Space::new().height(range.start as f32 * ROW)].padding(Padding::ZERO.right(SCROLLBAR));
        for index in range.clone() {
            let track = &self.queue[index];
            let current = self.current == Some(index);
            let mut line = row![
                self.cover(track, COVER),
                column![
                    text(&track.title).font(ui::font_medium()).color(if current { ui::ACCENT } else { ui::WHITE }).wrapping(Wrapping::None),
                    text(&track.artists).color(ui::MUTED).wrapping(Wrapping::None),
                ]
                .width(Length::Fill)
                .clip(true),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            if !current {
                line = line.push(button(ui::icon(ui::CLOSE, 16.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(Message::QueueRemove(index)));
            }
            list = list.push(mouse_area(button(line).width(Length::Fill).height(ROW).padding([8, 8]).style(ui::row).on_press(Message::QueuePlay(index))).on_right_press(Message::MenuOpen(track.clone())));
        }
        list = list.push(Space::new().height((self.queue.len() - range.end) as f32 * ROW));
        let list = scrollable(list).id(QUEUE_LIST).on_scroll(|viewport| Message::QueueScrolled(viewport.absolute_offset().y)).height(Length::Fill);
        container(column![header, list].spacing(12)).width(QUEUE_WIDTH).height(Length::Fill).padding(16).style(ui::content).into()
    }

    /// Качество и очередь: кнопки рядом с громкостью.
    fn view_tools<'a>(&self, size: f32) -> Element<'a, Message> {
        let tool = |icon, on: bool, message| button(ui::icon(icon, size, if on { ui::ACCENT } else { ui::MUTED })).padding(4).style(ui::ghost).on_press(message);
        row![tool(ui::TUNE, self.quality == Quality::Lossless, Message::QualityOpen), tool(ui::QUEUE, self.queue_open, Message::QueueToggle)].spacing(6).align_y(Alignment::Center).into()
    }

    fn more_button<'a>(&self, track: &Track, size: f32) -> Element<'a, Message> {
        button(container(ui::icon(ui::MORE, size, ui::MUTED)).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::MenuOpen(track.clone())).into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let opened = match self.section {
            Section::Likes | Section::Playlists | Section::Albums | Section::Artists => Section::Collection,
            section => section,
        };
        let item = |icon, label, section| {
            let tint = if opened == section { ui::TEXT } else { ui::MUTED };
            button(row![ui::icon(icon, 24.0, tint), text(label).color(tint)].spacing(12).align_y(Alignment::Center))
                .width(Length::Fill)
                .padding(Padding::new(12.0).right(24))
                .style(ui::nav)
                .on_press(Message::Open(section))
        };
        let mut account = column![text(self.profile.as_ref().map_or("", |p| p.name.as_str())).size(14).font(ui::font_medium()).color(ui::TEXT).wrapping(Wrapping::None)].clip(true).width(Length::Fill);
        if self.profile.as_ref().is_some_and(|p| p.plus) {
            account = account.push(container(text("Плюс").size(12).font(ui::font_medium())).padding([1, 8]).style(ui::plus));
        }
        let first = self.profile.as_ref().and_then(|p| p.name.chars().next()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let profile = row![
            container(text(first).size(18).font(ui::font_medium())).center(42).style(ui::avatar),
            account,
            button(ui::icon(ui::LOGOUT, 18.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(Message::Logout),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        container(
            column![
                container(text("Tune").size(26).font(ui::font_headline()).color(ui::ACCENT)).padding(Padding::new(0.0).left(12)),
                Space::new().height(8),
                column![
                    item(ui::SEARCH, "Поиск", Section::Search),
                    item(ui::WAVE, "Моя волна", Section::Wave),
                    item(ui::HEART_OUTLINE, "Коллекция", Section::Collection),
                ]
                .spacing(4),
                Space::new().height(16),
                self.view_pins(),
                Space::new().height(Length::Fill),
                profile,
            ]
            .spacing(4),
        )
        .width(SIDEBAR)
        .height(Length::Fill)
        .padding(Padding::new(12.0).top(4))
        .into()
    }

    /// Закреплённое пользователем в аккаунте.
    fn view_pins(&self) -> Element<'_, Message> {
        self.pins
            .iter()
            .enumerate()
            .fold(column![].spacing(4), |list, (index, pin)| {
                let radius = if pin.round { 20.0 } else { 6.0 };
                let cover: Element<'_, Message> = match self.covers.get(&format!("{PIN_COVER_PREFIX}{index}")) {
                    Some(handle) => image(handle.clone()).width(40).height(40).border_radius(radius).into(),
                    None => container(Space::new()).width(40).height(40).style(if pin.round { ui::avatar } else { ui::tile }).into(),
                };
                let line = row![
                    cover,
                    column![
                        text(&pin.title).size(13).font(ui::font_medium()).color(ui::TEXT).wrapping(Wrapping::None),
                        text(pin.subtitle).size(12).color(ui::MUTED).wrapping(Wrapping::None),
                    ]
                    .width(Length::Fill)
                    .clip(true),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                list.push(button(line).width(Length::Fill).padding(6).style(ui::row).on_press(Message::PinOpen(index)))
            })
            .into()
    }

    /// Заголовок блока со стрелкой, как на страницах оригинала.
    fn block_title<'a>(&self, title: &'a str, section: Section) -> Element<'a, Message> {
        button(row![text(title).size(24).font(ui::font_headline()).color(ui::TEXT), ui::icon(ui::CHEVRON, 24.0, ui::MUTED)].spacing(6).align_y(Alignment::Center))
            .padding(0)
            .style(button::text)
            .on_press(Message::Open(section))
            .into()
    }

    /// Главная: играющий трек с управлением, откуда бы он ни играл.
    fn view_wave(&self) -> Element<'_, Message> {
        let Some(track) = self.current_track() else {
            if self.wave_loading || self.uid.is_none() {
                return placeholder("Настраиваем волну…", None);
            }
            return container(
                column![
                    text("Моя волна").size(56).font(ui::font_headline()).color(ui::WHITE),
                    Space::new().height(16),
                    button(container(ui::icon(ui::PLAY, 36.0, ui::ON_ACCENT)).center(64)).padding(8).style(ui::round).on_press(Message::WaveStart),
                ]
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into();
        };
        let side = |icon, message| button(ui::icon(icon, 32.0, ui::TEXT)).padding(8).style(ui::ghost).on_press(message);
        let circle = |icon, message| button(container(ui::icon(icon, 22.0, ui::TEXT)).center(40)).padding(8).style(ui::secondary).on_press(message);
        let liked = self.liked_set.contains(&track.id);
        let name = track.artists.split(',').next().unwrap_or_default().to_string();
        let total = track.duration.as_secs_f32().max(1.0);
        let shown = self.seeking.unwrap_or(self.position.as_secs_f32()).min(total);
        let title = container(text(&track.title).size(18).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)).center(Length::Fill).padding([0, 24]).clip(true);
        let progress = canvas(seek::Seek { value: shown, total, radius: 28.0, base: ui::CONTROL, fill: ui::CONTROL_HOVER }).width(Length::Fill).height(56);
        let pill = container(stack![progress, title]).width(PILL_WIDTH).height(56);
        let mut top = column![].spacing(4).align_x(Alignment::Center);
        match &self.queue_title {
            Some(source) => {
                let back = row![text(source).size(20).font(ui::font_headline()).color(ui::ACCENT), ui::icon(ui::CLOSE, 22.0, ui::ACCENT)].spacing(8).align_y(Alignment::Center);
                top = top.push(button(back).padding([6, 14]).style(ui::source_button).on_press(Message::WaveStart));
            }
            None => top = top.push(text("Моя волна").size(20).font(ui::font_headline()).color(ui::ACCENT)),
        }
        let time = if self.starting { "Загрузка…".to_string() } else { format!("{} / {}", clock(Duration::from_secs_f32(shown)), clock(track.duration)) };
        container(
            column![
                top,
                Space::new().height(12),
                text(name).size(64).font(ui::font_headline()).color(ui::WHITE).wrapping(Wrapping::None),
                Space::new().height(16),
                self.cover(track, WAVE_COVER),
                Space::new().height(16),
                row![pill, circle(if liked { ui::HEART } else { ui::HEART_OUTLINE }, Message::Like(track.clone())), circle(ui::MORE, Message::MenuOpen(track.clone()))].spacing(8).align_y(Alignment::Center),
                Space::new().height(8),
                text(time).size(13).color(ui::MUTED),
                Space::new().height(12),
                row![
                    side(ui::PREVIOUS, Message::Previous),
                    button(container(ui::icon(if self.is_playing() { ui::PAUSE } else { ui::PLAY }, 36.0, ui::ON_ACCENT)).center(64)).padding(8).style(ui::round).on_press(Message::Toggle),
                    side(ui::NEXT, Message::Next),
                ]
                .spacing(28)
                .align_y(Alignment::Center),
                Space::new().height(16),
                row![self.view_tools(22.0), self.view_volume(22.0, Some(160.0))].spacing(10).align_y(Alignment::Center),
            ]
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into()
    }

    fn view_volume<'a>(&self, icon: f32, width: Option<f32>) -> Element<'a, Message> {
        let shown = if self.muted { 0.0 } else { self.volume };
        let mut line = row![button(ui::icon(if self.muted { ui::VOLUME_OFF } else { ui::VOLUME }, icon, ui::MUTED)).padding(4).style(ui::ghost).on_press(Message::ToggleMute)].spacing(6).align_y(Alignment::Center);
        if let Some(width) = width {
            line = line.push(slider(0.0..=1.0, shown, Message::Volume).step(0.01).width(width).style(ui::volume));
        }
        line.into()
    }

    fn view_collection(&self) -> Element<'_, Message> {
        let count = match self.liked_ids.len() {
            0 => String::new(),
            n => format!("{n} {}", tracks_word(n)),
        };
        let tile = container(ui::icon(ui::HEART, 34.0, ui::WHITE)).center(58).style(ui::tile);
        let likes = button(
            row![
                tile,
                column![
                    row![text("Мне нравится").size(24).font(ui::font_headline()).color(ui::TEXT), ui::icon(ui::CHEVRON, 24.0, ui::MUTED)].spacing(6).align_y(Alignment::Center),
                    text(count).color(ui::MUTED),
                ],
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding(0)
        .style(button::text)
        .on_press(Message::Open(Section::Likes));

        // Первые восемь треков в две колонки.
        let column_of = |range: std::ops::Range<usize>| {
            range.fold(column![].spacing(0).width(Length::FillPortion(1)), |list, index| match self.liked.get(index) {
                Some(track) => list.push(self.view_track(Section::Likes, index, track)),
                None => list,
            })
        };
        let mut page = column![
            text("Коллекция").size(36).font(ui::font_headline()).color(ui::TEXT),
            Space::new().height(16),
            likes,
            row![column_of(0..4), column_of(4..8)].spacing(24),
        ]
        .spacing(16)
        .padding(Padding::ZERO.right(SCROLLBAR));
        if !self.playlists.is_empty() {
            page = page.push(Space::new().height(8)).push(self.block_title("Плейлисты", Section::Playlists)).push(self.view_playlist_cards(&self.playlists));
        }
        if !self.liked_albums.is_empty() {
            let shown = &self.liked_albums[..self.liked_albums.len().min(COLLECTION_CARDS)];
            page = page.push(Space::new().height(8)).push(self.block_title("Альбомы", Section::Albums)).push(self.view_cards(shown, ALBUM_COVER, false, open_album));
        }
        if !self.liked_artists.is_empty() {
            let shown = &self.liked_artists[..self.liked_artists.len().min(COLLECTION_CARDS)];
            page = page.push(Space::new().height(8)).push(self.block_title("Исполнители", Section::Artists)).push(self.view_cards(shown, ARTIST_COVER, true, open_artist));
        }
        scrollable(page).height(Length::Fill).into()
    }

    fn view_card_page<'a>(&'a self, title: &'a str, cards: &'a [Card], prefix: &str, round: bool, empty: &'a str) -> Element<'a, Message> {
        let back = button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back);
        let head = row![back, text(title).size(36).font(ui::font_headline()).color(ui::TEXT)].spacing(12).align_y(Alignment::Center);
        let center: Element<'a, Message> = if cards.is_empty() {
            placeholder("Пока пусто", Some(empty))
        } else {
            let open = if round { open_artist } else { open_album };
            scrollable(container(self.view_cards(cards, prefix, round, open)).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).height(Length::Fill).into()
        };
        column![head, center].spacing(16).height(Length::Fill).into()
    }

    fn view_playlist_cards<'a>(&self, playlists: &'a [Playlist]) -> Element<'a, Message> {
        let cards = playlists.iter().fold(row![].spacing(16), |cards, playlist| {
            let cover: Element<'a, Message> = match self.covers.get(&playlist_cover_key(playlist)) {
                Some(handle) => image(handle.clone()).width(CARD).height(CARD).border_radius(8).into(),
                None => container(ui::icon(ui::PLAY, 48.0, ui::MUTED)).center(CARD).style(ui::tile).into(),
            };
            let card = column![
                cover,
                text(&playlist.title).font(ui::font_medium()).color(ui::TEXT).wrapping(Wrapping::None),
                text(format!("{} {}", playlist.count, tracks_word(playlist.count as usize))).size(13).color(ui::MUTED),
            ]
            .spacing(6)
            .width(CARD)
            .clip(true);
            cards.push(button(card).padding(0).style(button::text).on_press(Message::ListOpen(playlist.clone())))
        });
        cards.wrap().into()
    }

    fn view_likes(&self) -> Element<'_, Message> {
        let count = match self.liked_ids.len() {
            0 => String::new(),
            n => format!("{n} {}", tracks_word(n)),
        };
        let title = row![
            button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back),
            text("Мне нравится").size(36).font(ui::font_headline()).color(ui::TEXT),
            text(count).color(ui::MUTED),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let center: Element<'_, Message> = if !self.liked.is_empty() {
            self.view_list(Section::Likes, None)
        } else if self.likes_loading || self.uid.is_none() {
            skeleton_list()
        } else {
            placeholder("Пока пусто", Some("Нажмите на сердце у трека, и он появится здесь"))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_playlists(&self) -> Element<'_, Message> {
        let back = button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back);
        if let Some(playlist) = &self.playlist {
            if playlist.source == Source::Album {
                return self.view_album(playlist);
            }
            let title = row![
                back,
                text(&playlist.title).size(36).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None),
                text(if self.playlist_tracks.is_empty() { String::new() } else { format!("{} {}", playlist.count, tracks_word(playlist.count as usize)) }).color(ui::MUTED),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            let center: Element<'_, Message> = if !self.playlist_tracks.is_empty() {
                self.view_list(Section::Playlists, None)
            } else if playlist.count == 0 {
                placeholder("В этом плейлисте пока нет треков", None)
            } else {
                skeleton_list()
            };
            return column![title, center].spacing(16).height(Length::Fill).into();
        }
        let title = row![back, text("Плейлисты").size(36).font(ui::font_headline()).color(ui::TEXT)].spacing(12).align_y(Alignment::Center);
        let center: Element<'_, Message> = if !self.playlists.is_empty() {
            scrollable(container(self.view_playlist_cards(&self.playlists)).padding(Padding::ZERO.right(SCROLLBAR))).height(Length::Fill).into()
        } else if self.playlists_loading || self.uid.is_none() {
            skeleton_cards()
        } else {
            placeholder("Плейлистов пока нет", Some("Созданные вами плейлисты появятся здесь"))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_search(&self) -> Element<'_, Message> {
        let input = text_input("Трек или исполнитель", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .padding([11, 20])
            .size(15)
            .style(ui::input);
        let mut page = column![input].spacing(16).height(Length::Fill);
        if !self.searched && !self.searching {
            return page.push(placeholder("Что послушаем?", Some("Начните вводить название трека, альбома или исполнителя"))).into();
        }
        let tabs = SearchTab::ALL.into_iter().fold(row![].spacing(8), |tabs, tab| {
            tabs.push(button(text(tab.label())).padding([7, 16]).style(ui::chip(tab == self.search_tab)).on_press(Message::SearchTabSet(tab)))
        });
        page = page.push(tabs);
        if let Some(corrected) = &self.found.corrected {
            page = page.push(text(format!("Показаны результаты по запросу «{corrected}»")).color(ui::MUTED));
        }
        let found = &self.found;
        let nothing = found.tracks.is_empty() && found.albums.is_empty() && found.artists.is_empty() && found.playlists.is_empty();
        let center: Element<'_, Message> = if self.searching {
            skeleton_list()
        } else if nothing {
            placeholder("Ничего не нашлось", Some("Попробуйте написать иначе"))
        } else if self.search_tab == SearchTab::Tracks {
            self.view_list(Section::Search, None)
        } else {
            let block = |title| text(title).size(24).font(ui::font_headline()).color(ui::TEXT);
            let mut blocks = column![].spacing(12).padding(Padding::ZERO.right(SCROLLBAR));
            if !found.artists.is_empty() {
                blocks = blocks.push(block("Исполнители")).push(self.view_cards(&found.artists, ARTIST_COVER, true, open_artist)).push(Space::new().height(8));
            }
            if !found.tracks.is_empty() {
                let shown = found.tracks.iter().take(SEARCH_ALL_TRACKS).enumerate();
                blocks = blocks.push(block("Треки")).push(shown.fold(column![], |list, (index, track)| list.push(self.view_track(Section::Search, index, track)))).push(Space::new().height(8));
            }
            if !found.albums.is_empty() {
                blocks = blocks.push(block("Альбомы")).push(self.view_cards(&found.albums, ALBUM_COVER, false, open_album)).push(Space::new().height(8));
            }
            if !found.playlists.is_empty() {
                blocks = blocks.push(block("Плейлисты")).push(self.view_playlist_cards(&found.playlists));
            }
            scrollable(blocks).id(LIST).height(Length::Fill).into()
        };
        page.push(center).into()
    }

    /// Шапка альбома и исполнителя: обложка, название, подписи и кнопки.
    fn page_header<'a>(&self, kicker: &'a str, title: String, cover_key: String, round: bool, about: String, actions: iced::widget::Row<'a, Message>) -> Element<'a, Message> {
        let radius = if round { PAGE_COVER / 2.0 } else { 8.0 };
        let cover: Element<'a, Message> = match self.covers.get(&cover_key) {
            Some(handle) => image(handle.clone()).width(PAGE_COVER).height(PAGE_COVER).border_radius(radius).into(),
            None => container(Space::new()).width(PAGE_COVER).height(PAGE_COVER).style(if round { ui::avatar } else { ui::tile }).into(),
        };
        let back = button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back);
        let info = column![
            text(kicker).size(14).color(ui::MUTED),
            text(title).size(40).font(ui::font_headline()).color(ui::WHITE).wrapping(Wrapping::None),
            text(about).color(ui::MUTED).wrapping(Wrapping::None),
            Space::new().height(8),
            actions,
        ]
        .spacing(6)
        .width(Length::Fill)
        .clip(true);
        container(row![container(back).height(PAGE_COVER), cover, container(info).center_y(PAGE_COVER)].spacing(16)).height(PAGE_HEADER).width(Length::Fill).into()
    }

    /// Кнопки шапки: запуск списка и волна по нему.
    fn page_actions<'a>(&self, section: Section, seed: String) -> iced::widget::Row<'a, Message> {
        let play = button(row![ui::icon(ui::PLAY, 22.0, ui::ON_ACCENT), text("Слушать").font(ui::font_medium()).color(ui::ON_ACCENT)].spacing(6).align_y(Alignment::Center))
            .padding([8, 18])
            .style(ui::accent)
            .on_press_maybe((!self.list(section).is_empty()).then_some(Message::Play(section, 0)));
        let wave = button(row![ui::icon(ui::WAVE, 20.0, ui::TEXT), text("Моя волна").color(ui::TEXT)].spacing(6).align_y(Alignment::Center))
            .padding([8, 18])
            .style(ui::secondary)
            .on_press(Message::WaveStartSeeds(vec![seed]));
        row![play, wave].spacing(8).align_y(Alignment::Center)
    }

    fn page_like<'a>(&self, liked: bool, message: Message) -> Element<'a, Message> {
        let icon = if liked { ui::HEART } else { ui::HEART_OUTLINE };
        button(container(ui::icon(icon, 22.0, ui::TEXT)).center(22)).padding(9).style(ui::secondary).on_press(message).into()
    }

    fn view_album<'a>(&'a self, playlist: &'a Playlist) -> Element<'a, Message> {
        let Some(album) = &self.album else {
            let back = button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back);
            return column![back, skeleton_list()].spacing(16).height(Length::Fill).into();
        };
        let count = self.playlist_tracks.len();
        let mut about = vec![album.artists.clone()];
        about.extend(album.year.map(|year| year.to_string()));
        about.push(format!("{count} {}", tracks_word(count)));
        let mut actions = self.page_actions(Section::Playlists, format!("album:{}", album.id));
        if let Some(artist) = &album.artist_id {
            actions = actions.push(button(text("К исполнителю").color(ui::TEXT)).padding([8, 18]).style(ui::secondary).on_press(Message::ArtistOpen(artist.clone())));
        }
        let liked = self.liked_albums.iter().any(|c| c.id == album.id);
        actions = actions.push(self.page_like(liked, Message::AlbumLike));
        let header = self.page_header("Альбом", playlist.title.clone(), format!("{ALBUM_COVER}{}", album.id), false, about.join(" · "), actions);
        self.view_list(Section::Playlists, Some(header))
    }

    fn view_cards<'a>(&self, cards: &'a [Card], prefix: &str, round: bool, open: fn(&Card) -> Message) -> Element<'a, Message> {
        cards
            .iter()
            .fold(row![].spacing(16), |line, card| {
                let cover: Element<'a, Message> = match self.covers.get(&format!("{prefix}{}", card.id)) {
                    Some(handle) => image(handle.clone()).width(CARD).height(CARD).border_radius(if round { CARD / 2.0 } else { 8.0 }).into(),
                    None => container(Space::new()).width(CARD).height(CARD).style(if round { ui::avatar } else { ui::tile }).into(),
                };
                let body = column![cover, text(&card.title).font(ui::font_medium()).color(ui::TEXT).wrapping(Wrapping::None), text(&card.subtitle).size(13).color(ui::MUTED)].spacing(6).width(CARD).clip(true);
                line.push(button(body).padding(0).style(button::text).on_press(open(card)))
            })
            .wrap()
            .into()
    }

    fn view_artist(&self) -> Element<'_, Message> {
        let Some(artist) = &self.artist else {
            let back = button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back);
            return column![back, skeleton_list()].spacing(16).height(Length::Fill).into();
        };
        let about = if artist.listeners > 0 { format!("Слушателей в месяц: {}", grouped(artist.listeners)) } else { String::new() };
        let actions = self.page_actions(Section::Artist, format!("artist:{}", artist.id)).push(button(text("Все треки").color(ui::TEXT)).padding([8, 18]).style(ui::secondary).on_press(Message::ArtistTracks));
        let liked = self.liked_artists.iter().any(|c| c.id == artist.id);
        let actions = actions.push(self.page_like(liked, Message::ArtistLike));
        let block = |title| text(title).size(24).font(ui::font_headline()).color(ui::TEXT);
        let mut page = column![self.page_header("Исполнитель", artist.name.clone(), format!("{ARTIST_COVER}{}", artist.id), true, about, actions)].spacing(12).padding(Padding::ZERO.right(SCROLLBAR));
        if !artist.popular.is_empty() {
            page = page.push(block("Популярные треки"));
            page = page.push(artist.popular.iter().enumerate().fold(column![], |list, (index, track)| list.push(self.view_track(Section::Artist, index, track))));
        }
        let albums = |cards: &'_ [Card]| !cards.is_empty();
        if albums(&artist.albums) {
            page = page.push(Space::new().height(8)).push(block("Альбомы")).push(self.view_cards(&artist.albums, ALBUM_COVER, false, open_album));
        }
        if albums(&artist.also) {
            page = page.push(Space::new().height(8)).push(block("Сборники")).push(self.view_cards(&artist.also, ALBUM_COVER, false, open_album));
        }
        if albums(&artist.similar) {
            page = page.push(Space::new().height(8)).push(block("Похожие исполнители")).push(self.view_cards(&artist.similar, ARTIST_COVER, true, open_artist));
        }
        scrollable(page).id(LIST).height(Length::Fill).into()
    }

    /// Строятся только видимые строки: остальные заменяет пустое место той же высоты.
    fn view_list<'a>(&'a self, section: Section, header: Option<Element<'a, Message>>) -> Element<'a, Message> {
        let total = self.row_count(section);
        let range = self.visible();
        let tracks = self.list(section);
        let mut list = column![].padding(Padding::ZERO.right(SCROLLBAR));
        if let Some(header) = header {
            list = list.push(header);
        }
        list = list.push(Space::new().height(range.start as f32 * ROW));
        for index in range.clone() {
            list = list.push(match tracks.get(index) {
                Some(track) => self.view_track(section, index, track),
                None => skeleton_row(),
            });
        }
        list = list.push(Space::new().height((total - range.end) as f32 * ROW));
        scrollable(list).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).height(Length::Fill).into()
    }

    fn view_track<'a>(&'a self, section: Section, index: usize, track: &'a Track) -> Element<'a, Message> {
        let current = self.current_track().is_some_and(|t| t.id == track.id);
        let cover: Element<'_, Message> = if current {
            let icon = if self.is_playing() { ui::PAUSE } else { ui::PLAY };
            stack![self.cover(track, COVER), container(ui::icon(icon, 22.0, ui::WHITE)).center(COVER).style(ui::overlay)].into()
        } else {
            self.cover(track, COVER)
        };
        let line = row![
            cover,
            column![
                text(&track.title).font(ui::font_medium()).color(ui::WHITE).wrapping(Wrapping::None),
                artist_link(track, &track.artists),
            ]
            .width(Length::Fill)
            .clip(true),
            self.like_button(track, 20.0),
            text(clock(track.duration)).color(ui::MUTED),
            self.more_button(track, 18.0),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        mouse_area(button(line).width(Length::Fill).height(ROW).padding([8, 8]).style(ui::row).on_press(Message::Play(section, index))).on_right_press(Message::MenuOpen(track.clone())).into()
    }

    fn like_button<'a>(&self, track: &Track, size: f32) -> Element<'a, Message> {
        let liked = self.liked_set.contains(&track.id);
        let icon = if liked { ui::icon(ui::HEART, size, ui::MUTED) } else { ui::icon(ui::HEART_OUTLINE, size, ui::MUTED) };
        button(container(icon).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::Like(track.clone())).into()
    }

    fn cover<'a>(&'a self, track: &Track, size: f32) -> Element<'a, Message> {
        match self.covers.get(&track.id) {
            Some(handle) => image(handle.clone()).width(size).height(size).border_radius(6).into(),
            None => container(Space::new()).width(size).height(size).style(ui::tile).into(),
        }
    }

    fn is_playing(&self) -> bool {
        self.starting || (self.active && !self.player.as_ref().is_none_or(Player::is_paused))
    }

    fn view_bar(&self) -> Option<Element<'_, Message>> {
        let track = self.current_track()?;
        let total = track.duration.as_secs_f32().max(1.0);
        let shown = self.seeking.unwrap_or(self.position.as_secs_f32()).min(total);
        let status = if self.starting { "Загрузка…" } else { track.artists.as_str() };

        let side = |icon, message| button(ui::icon(icon, 26.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(message);
        let toggle = button(container(ui::icon(if self.is_playing() { ui::PAUSE } else { ui::PLAY }, 26.0, ui::ON_ACCENT)).center(26)).padding(7).style(ui::round).on_press(Message::Toggle);

        let about = row![
            self.cover(track, BAR_COVER),
            column![
                text(&track.title).font(ui::font_medium()).color(ui::WHITE).wrapping(Wrapping::None),
                artist_link(track, status),
            ]
            .width(Length::Fill)
            .clip(true),
            self.like_button(track, 22.0),
            self.more_button(track, 20.0),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .width(if self.queue_open { Length::Fill } else { Length::FillPortion(3) });

        // В волне порядок задаёт она сама, поэтому этих кнопок там нет.
        let mode = |icon, on: bool, message| button(ui::icon(icon, 22.0, if on { ui::ACCENT } else { ui::MUTED })).padding(8).style(ui::ghost).on_press(message);
        let mut buttons = row![].spacing(14).align_y(Alignment::Center);
        if !self.wave_queue {
            buttons = buttons.push(mode(ui::SHUFFLE, self.shuffle, Message::ShuffleToggle));
        }
        buttons = buttons.push(side(ui::PREVIOUS, Message::Previous)).push(toggle).push(side(ui::NEXT, Message::Next));
        if !self.wave_queue {
            let icon = if self.repeat == Repeat::One { ui::REPEAT_ONE } else { ui::REPEAT };
            buttons = buttons.push(mode(icon, self.repeat != Repeat::Off, Message::RepeatNext));
        }
        // С открытой очередью места мало: название важнее пустых полей.
        let (transport_width, right_width) = if self.queue_open { (Length::Shrink, Length::Shrink) } else { (Length::FillPortion(4), Length::FillPortion(3)) };
        let transport = container(buttons).center_x(transport_width);

        let mut right = row![].spacing(8).align_y(Alignment::Center);
        if self.bar_hover {
            right = right.push(text(format!("{} / {}", clock(Duration::from_secs_f32(shown)), clock(track.duration))).size(12).color(ui::MUTED)).push(Space::new().width(8));
        }
        let volume = container(right.push(self.view_tools(22.0)).push(self.view_volume(24.0, (!self.queue_open).then_some(90.0))))
            .align_right(right_width);

        let seek = canvas(seek::Seek { value: shown, total, radius: 12.0, base: iced::Color::TRANSPARENT, fill: ui::CONTROL }).width(Length::Fill).height(BAR_HEIGHT);
        let controls = container(row![about, transport, volume].spacing(16).align_y(Alignment::Center)).padding([8, 8]).center_y(BAR_HEIGHT).width(Length::Fill);
        Some(
            mouse_area(container(stack![seek, controls]).width(Length::Fill).height(BAR_HEIGHT).style(ui::player).clip(true))
                .on_enter(Message::BarHover(true))
                .on_exit(Message::BarHover(false))
                .into(),
        )
    }
}

/// Всплывающая панель у курсора. Не выходит за край окна.
fn popup<'a>(content: Element<'a, Message>, x: f32, y: f32, width: f32, height: f32) -> Element<'a, Message> {
    let (window_width, window_height) = *WINDOW.lock().unwrap();
    let x = x.min(window_width - width - 8.0).max(8.0);
    let y = y.min(window_height - height - 24.0).max(8.0);
    container(container(content).width(width).padding(8).style(ui::popover))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::new(0.0).left(x).top(y))
        .into()
}

fn open_album(card: &Card) -> Message {
    Message::AlbumOpen(card.id.clone(), card.title.clone())
}

fn open_artist(card: &Card) -> Message {
    Message::ArtistOpen(card.id.clone())
}

/// У разных владельцев номера плейлистов совпадают, поэтому в ключе есть и владелец.
fn playlist_cover_key(playlist: &Playlist) -> String {
    format!("{PLAYLIST_COVER}{}:{}", playlist.owner, playlist.kind)
}

fn preload_key(id: &str, quality: Quality) -> String {
    format!("{id}:{}", quality.code())
}

/// 4733421 → «4 733 421».
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// Имя исполнителя, по которому можно перейти на его страницу.
fn artist_link<'a>(track: &'a Track, label: &'a str) -> Element<'a, Message> {
    let line = text(label).color(ui::MUTED).wrapping(Wrapping::None);
    match &track.artist_id {
        Some(id) => button(line).padding(0).style(button::text).on_press(Message::ArtistOpen(id.clone())).into(),
        None => line.into(),
    }
}

fn codec_name(codec: &str) -> String {
    match codec {
        "flac" | "flac-mp4" => "FLAC".to_string(),
        "mp3" => "MP3".to_string(),
        "aac" | "aac-mp4" => "AAC".to_string(),
        "he-aac" | "he-aac-mp4" => "HE-AAC".to_string(),
        other => other.to_uppercase(),
    }
}

/// Заглушка строки, пока трек не загружен.
fn skeleton_row<'a>() -> Element<'a, Message> {
    let bar = |width: f32, height: f32| container(Space::new()).width(width).height(height).style(ui::tile);
    container(row![bar(COVER, COVER), column![bar(180.0, 12.0), bar(110.0, 10.0)].spacing(8)].spacing(12).align_y(Alignment::Center))
        .width(Length::Fill)
        .height(ROW)
        .padding([8, 8])
        .into()
}

fn skeleton_list<'a>() -> Element<'a, Message> {
    (0..SKELETON_ROWS).fold(column![].padding(Padding::ZERO.right(SCROLLBAR)), |list, _| list.push(skeleton_row())).height(Length::Fill).into()
}

fn skeleton_cards<'a>() -> Element<'a, Message> {
    (0..SKELETON_ROWS).fold(row![].spacing(16), |cards, _| cards.push(container(Space::new()).width(CARD).height(CARD).style(ui::tile))).wrap().into()
}

fn placeholder<'a>(title: &'a str, hint: Option<&'a str>) -> Element<'a, Message> {
    let mut content = column![].spacing(6).align_x(Alignment::Center);
    match hint {
        Some(hint) => content = content.push(text(title).size(24).font(ui::font_headline()).color(ui::TEXT)).push(text(hint).color(ui::MUTED)),
        None => content = content.push(text(title).color(ui::MUTED)),
    }
    container(content).center(Length::Fill).into()
}

/// Окно нужно медиапанели. Обработчик идёт в потоке окна.
fn window_handle() -> Task<Message> {
    window::latest()
        .then(|id| match id {
            Some(id) => window::run(id, |w| match w.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                _ => None,
            }),
            None => Task::done(None),
        })
        .map(Message::WindowReady)
}

/// Заголовок окна чёрный, как в оригинале. Рамка остаётся системной: работают привязка
/// к краям экрана и изменение размера.
fn black_caption(hwnd: isize) {
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR};
    let black: u32 = 0;
    unsafe {
        DwmSetWindowAttribute(hwnd as _, DWMWA_CAPTION_COLOR as u32, std::ptr::from_ref(&black).cast(), 4);
    }
}

trait PipeMedia {
    fn pipe_media(self) -> Self;
}

impl PipeMedia for Subscription<Message> {
    fn pipe_media(self) -> Self {
        let back = event::listen_with(|event, _, _| match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Back)) => Some(Message::Back),
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                *CURSOR.lock().unwrap() = (position.x, position.y);
                None
            }
            Event::Window(window::Event::Resized(size)) => {
                *WINDOW.lock().unwrap() = (size.width, size.height);
                None
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Escape), .. }) => Some(Message::MenuClose),
            _ => None,
        });
        Subscription::batch([self, back, Subscription::run(media::events).map(Message::Media)])
    }
}

/// Запускает окно входа отдельным процессом и ждёт, пока его закроют.
fn login_window() -> bool {
    std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).arg("--login").status())
        .is_ok_and(|status| status.success())
}

fn load_account(token: String) -> Task<Message> {
    Task::perform(
        background(move || {
            let profile = api::profile(&token)?;
            let ids = api::liked_ids(&profile.uid, &token)?;
            let disliked = api::disliked_ids(&profile.uid, &token)?;
            Ok((profile, ids, disliked))
        }),
        Message::Account,
    )
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn tracks_word(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=14) => "треков",
        (1, _) => "трек",
        (2..=4, _) => "трека",
        _ => "треков",
    }
}

/// Проверка без окна: поиск, ссылка на файл и несколько секунд звука.
fn smoke(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let tracks = api::search(query, &token)?;
    println!("Найдено: {}", tracks.len());
    let track = tracks.first().ok_or("Ничего не найдено")?;
    println!("Трек: {} — {} ({})", track.artists, track.title, clock(track.duration));
    let info = api::file_info(&track.id, Quality::Normal.code(), &token)?;
    println!("Файл: {}, зашифрован: {}", info.codec, info.key.is_some());
    let player = Player::new()?;
    player.set_volume(0.05);
    let generation = player.stop();
    player.play(generation, &info)?;
    std::thread::sleep(Duration::from_secs(4));
    println!("Позиция через 4 с: {:?}", player.position());
    player.seek(Duration::from_secs(30))?;
    std::thread::sleep(Duration::from_secs(2));
    println!("Позиция после перемотки на 30 с: {:?}", player.position());
    Ok(())
}

/// Проверка без окна и без звука: каждое качество запускается и перематывается.
fn smoke_quality(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let tracks = api::search(query, &token)?;
    let track = tracks.first().ok_or("Ничего не найдено")?;
    println!("Трек: {} — {}", track.artists, track.title);
    let player = Player::new()?;
    player.set_volume(0.0);
    for quality in Quality::ALL {
        let info = api::file_info(&track.id, quality.code(), &token)?;
        let generation = player.stop();
        player.play(generation, &info)?;
        std::thread::sleep(Duration::from_secs(3));
        let played = player.position();
        player.seek(Duration::from_secs(30))?;
        std::thread::sleep(Duration::from_secs(1));
        println!("{}: {}, позиция {played:?}, после перемотки {:?}", quality.code(), info.codec, player.position());
    }
    let info = api::file_info(&track.id, Quality::Normal.code(), &token)?;
    player.preload("проверка", &info)?;
    let generation = player.stop();
    let started = std::time::Instant::now();
    player.play_prepared(generation, "проверка").ok_or("Подготовленный файл потерялся")??;
    println!("Запуск подготовленного файла: {:?}", started.elapsed());
    std::thread::sleep(Duration::from_secs(2));
    println!("Позиция: {:?}", player.position());
    Ok(())
}

/// Проверка без окна: лайки, волна и её обратная связь. Лайк ставится и сразу снимается,
/// список «Мне нравится» остаётся прежним; из обратной связи волне уходит только запуск.
fn smoke_library(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let uid = api::profile(&token)?.uid;
    let ids = api::liked_ids(&uid, &token)?;
    println!("Лайков: {}", ids.len());
    let page = api::tracks_by_ids(&ids[..ids.len().min(LIKES_PAGE)], &token)?;
    println!("Загружено сведений: {}, порядок совпал: {}", page.len(), page.iter().zip(&ids).all(|(t, id)| t.id == *id));
    let wave = api::wave_new(&[api::MY_WAVE.to_string()], &token)?;
    println!("Волна: треков в первой пачке {}", wave.tracks.len());
    println!("Обратная связь: {:?}", api::wave_feedback(&wave.session, "radioStarted", None, None, &token));
    let shown: Vec<String> = wave.tracks.iter().map(Track::full_id).collect();
    println!("Следующая пачка: {:?}", api::wave_more(&wave.session, &shown, &token).map(|t| t.len()));
    let track = api::search(query, &token)?.into_iter().find(|t| !ids.contains(&t.id)).ok_or("Нет трека без лайка")?;
    api::set_liked(&uid, &track.id, true, &token)?;
    let added = api::liked_ids(&uid, &token)?.contains(&track.id);
    api::set_liked(&uid, &track.id, false, &token)?;
    let after = api::liked_ids(&uid, &token)?;
    println!("Лайк появился: {added}, снялся: {}, список как был: {}", !after.contains(&track.id), after == ids);
    let (albums, artists) = api::liked_pages(&uid, &token)?;
    println!("Альбомов в коллекции: {}, исполнителей: {}", albums.len(), artists.len());
    let found = api::search_mixed(query, SearchTab::All.types(), SEARCH_ALL, &token)?;
    println!("Смешанный поиск: треков {}, альбомов {}, исполнителей {}, плейлистов {}", found.tracks.len(), found.albums.len(), found.artists.len(), found.playlists.len());
    if let Some(album) = found.albums.iter().find(|c| !albums.iter().any(|a| a.id == c.id)) {
        api::set_album_liked(&uid, &album.id, true, &token)?;
        let added = api::liked_pages(&uid, &token)?.0.iter().any(|a| a.id == album.id);
        api::set_album_liked(&uid, &album.id, false, &token)?;
        let after = api::liked_pages(&uid, &token)?.0;
        println!("Альбом добавился: {added}, убрался: {}, порядок прежний: {}", !after.iter().any(|a| a.id == album.id), after.iter().map(|a| &a.id).eq(albums.iter().map(|a| &a.id)));
    }
    if let Some(artist) = found.artists.iter().find(|c| !artists.iter().any(|a| a.id == c.id)) {
        api::set_artist_liked(&uid, &artist.id, true, &token)?;
        let added = api::liked_pages(&uid, &token)?.1.iter().any(|a| a.id == artist.id);
        api::set_artist_liked(&uid, &artist.id, false, &token)?;
        let after = api::liked_pages(&uid, &token)?.1;
        println!("Исполнитель добавился: {added}, убрался: {}, порядок прежний: {}", !after.iter().any(|a| a.id == artist.id), after.iter().map(|a| &a.id).eq(artists.iter().map(|a| &a.id)));
    }
    let pins = api::pins(&token)?;
    println!("Закреплено: {}", pins.iter().map(|p| p.title.as_str()).collect::<Vec<_>>().join(", "));
    let lists = api::playlists(&uid, &token)?;
    println!("Плейлистов: {}", lists.len());
    if let Some(list) = lists.iter().find(|p| p.count > 0) {
        println!("В плейлисте заявлено {}, загружено {}", list.count, api::playlist_tracks(list, &token)?.len());
    }
    let before = api::disliked_ids(&uid, &token)?;
    api::set_disliked(&uid, &track.id, true, &token)?;
    let added = api::disliked_ids(&uid, &token)?.contains(&track.id);
    api::set_disliked(&uid, &track.id, false, &token)?;
    let after = api::disliked_ids(&uid, &token)?;
    println!("Дизлайк появился: {added}, снялся: {}, список как был: {}", !after.contains(&track.id), after == before);
    Ok(())
}

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|flag| flag == "--login") {
        login::run();
    }
    if let [_, flag, path] = args.as_slice() {
        if flag == "--api" {
            match api::load_token().ok_or_else(|| "Сначала войдите".to_string()).and_then(|token| api::raw(path, &token)) {
                Ok(json) => println!("{json}"),
                Err(e) => println!("Ошибка: {e}"),
            }
            return Ok(());
        }
    }
    // DirectX 12 занимает меньше всего памяти из графических API. Нет видеокарты — iced сам
    // перейдёт на отрисовку процессором.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }
    let preview = match args.as_slice() {
        [_, flag, query] if flag == "--smoke" || flag == "--smoke-library" || flag == "--smoke-quality" => {
            let result = match flag.as_str() {
                "--smoke" => smoke(query),
                "--smoke-library" => smoke_library(query),
                _ => smoke_quality(query),
            };
            if let Err(e) = result {
                println!("Ошибка: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        [_, flag, what] if flag == "--preview" => Some(match what.as_str() {
            "wave" => Preview::Wave,
            "wave-seek" => Preview::WaveSeek,
            "likes" => Preview::Likes,
            "likes-end" => Preview::LikesEnd,
            "panels" => Preview::Panels,
            "menu" => Preview::Menu,
            "collection" => Preview::Collection,
            "playlists" => Preview::Playlists,
            "playlist" => Preview::Playlist,
            "playlist-home" => Preview::PlaylistHome,
            what if what.starts_with("album:") => Preview::Album(what[6..].to_string()),
            what if what.starts_with("artist:") => Preview::Artist(what[7..].to_string()),
            query => Preview::Search(query.to_string()),
        }),
        _ => None,
    };
    let yandex_fonts = fonts::load();
    ui::YANDEX_FONTS.store(!yandex_fonts.is_empty(), Ordering::Relaxed);
    let mut app = iced::application(move || App::new(preview.clone()), App::update, App::view);
    for font in yandex_fonts {
        app = app.font(font);
    }
    app.title("Tune")
        .theme(ui::theme())
        .default_font(ui::font())
        .subscription(App::subscription)
        .window(window::Settings { size: Size::new(1280.0, 820.0), min_size: Some(Size::new(900.0, 600.0)), ..window::Settings::default() })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_digits() {
        assert_eq!(grouped(4733421), "4 733 421");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1 000");
    }

    #[test]
    fn names_codecs() {
        assert_eq!(codec_name("flac-mp4"), "FLAC");
        assert_eq!(codec_name("he-aac-mp4"), "HE-AAC");
    }

    #[test]
    fn builds_only_rows_near_the_screen() {
        assert_eq!(visible_rows(0.0, 580.0, 1000), 0..16);
        assert_eq!(visible_rows(5800.0, 580.0, 1000), 94..116);
        assert_eq!(visible_rows(58000.0, 580.0, 1000), 994..1000);
        assert_eq!(visible_rows(0.0, 580.0, 3), 0..3);
        // Список стал короче, чем текущая прокрутка.
        assert_eq!(visible_rows(58000.0, 580.0, 3), 3..3);
    }

    #[test]
    fn shuffle_keeps_tracks_and_puts_chosen_first() {
        let mut items: Vec<u32> = (0..50).collect();
        shuffled(&mut items, 17, 12345);
        assert_eq!(items[0], 17);
        assert_ne!(items[1..10], [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<u32>>());
        let mut one = [5];
        shuffled(&mut one, 0, 1);
        assert_eq!(one, [5]);
        shuffled::<u32>(&mut [], 0, 1);
    }
}
