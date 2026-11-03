#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod anim;
mod fonts;
mod glow;
mod login;
mod media;
mod player;
mod seek;
mod session;
mod crossfade;
mod fit;
mod offline;
use fit::{fit, fit_row};
mod pick;
mod equalizer;
mod settings;
mod ui;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::ops::Range;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::futures::channel::{mpsc, oneshot};
use iced::widget::text::Wrapping;
use iced::keyboard;
use iced::widget::{button, canvas, column, container, image, mouse_area, operation, pick_list, row, scrollable, stack, text, text_input, vertical_slider, Id, Space};
use anim::{hover_line, reveal, smooth, Fade};
use glow::{glow, shine};
use iced::{event, mouse, time, window, Alignment, Event, Element, Length, Padding, Size, Subscription, Task};

use api::{Album, AlbumKind, Artist, Card, Found, Home, HomeBlock, HomeItem, Pin, PinRef, PinTarget, Playlist, Profile, Source, Track, Wave, NEED_LOGIN};
use media::Media;
use player::Player;
use raw_window_handle::RawWindowHandle;
use settings::{Quality, Repeat, Settings};

const TICK: Duration = Duration::from_millis(500);
const COVER: f32 = 40.0;
const BAR_COVER: f32 = 52.0;
const BAR_HEIGHT: f32 = 68.0;
/// Полоска перемотки: высота и ширина подписей времени.
const SEEK_HEIGHT: f32 = 20.0;
const SEEK_TIME: f32 = 40.0;
/// Кнопки трекбара: ширина с полями и расстояние между ними.
const BAR_MODE: f32 = 38.0;
const BAR_SIDE: f32 = 38.0;
const BAR_TOGGLE: f32 = 40.0;
const BAR_GAP: f32 = 14.0;
/// Сколько места нужно названию рядом с обложкой. Меньше — оно уезжает в строку сверху.
const BAR_TITLE_ROOM: f32 = 150.0;
/// Верхняя строка трекбара с названием: высота, размер текста и время выезда.
const BAR_TOP_LINE: f32 = 24.0;
const BAR_TOP_SIZE: f32 = 14.0;
/// Громкость в верхней строке узкого трекбара: значок и ползунок.
const BAR_TOP_VOLUME_ICON: f32 = 16.0;
const BAR_TOP_SLIDER: f32 = 90.0;
const BAR_TOP_FADE: f32 = 0.2;
const BAR_TOOL: f32 = 30.0;
const BAR_VOLUME_ICON: f32 = 32.0;
const BAR_SLIDER_MIN: f32 = 30.0;
const BAR_SLIDER_MAX: f32 = 110.0;
const CARD: f32 = 190.0;
/// Волна: что занято по высоте кроме обложки, пределы обложки и ширина подписей под ней.
const WAVE_COVER_MIN: f32 = 140.0;
const WAVE_COLUMN: f32 = 560.0;
/// Обложка играющего трека в волне крупнее, чем в списках.
const WAVE_BIG: &str = "wave-big:";
/// За сколько секунд проявляется загруженная обложка.
const COVER_FADE: f32 = 0.25;
const MENU_WIDTH: f32 = 270.0;
const MENU_ITEM: f32 = 40.0;
const NOTICE_TIME: Duration = Duration::from_secs(4);
/// Сколько держится уведомление об удалении с кнопкой отмены.
const UNDO_TIME: Duration = Duration::from_secs(5);

/// Где курсор и какого размера окно. Меняются постоянно, поэтому не идут через сообщения:
/// иначе интерфейс перестраивался бы от каждого движения мыши.
static CURSOR: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
static WINDOW: Mutex<(f32, f32)> = Mutex::new((1280.0, 820.0));
const PIN_COVER_PREFIX: &str = "pin:";
const AVATAR_COVER: &str = "avatar";
const AVATAR_SIZE: f32 = 42.0;
const ACCOUNT_AVATAR: f32 = 72.0;
const ACCOUNT_LINK: &str = "https://id.yandex.ru/";
const PLUS_LINK: &str = "https://plus.yandex.ru/my";
const SIDEBAR: f32 = 224.0;
const LIKE: f32 = 32.0;
/// Постоянное место под полосу прокрутки: колонка времени не прыгает на коротких списках.
const SCROLLBAR: f32 = 14.0;
const SEARCH_INPUT: &str = "search";
const FILTER_INPUT: &str = "filter";
const FILTER_WIDTH: f32 = 240.0;
const FILTER_WIDTH_SMALL: f32 = 180.0;
/// Уже этого заголовку нужно больше места, и поле поиска становится короче.
const FILTER_WIDE: f32 = 800.0;
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
/// Ширина окна настроек волны.
const TUNE_WIDTH: f32 = 560.0;
/// Страница текста: размер строк и как быстро она догоняет звучащую строку.
const LYRICS_SIZE: f32 = 26.0;
const LYRICS_SPEED: f32 = 5.0;
/// За сколько секунд текст возвращается к звучащей строке, если его прокрутили.
const LYRICS_RETURN: f32 = 0.7;
/// На сколько увеличена звучащая строка и как гаснут остальные с каждой строкой расстояния.
const LYRICS_GROW: f32 = 0.08;
/// Насколько яркой становится строка под курсором.
const LYRICS_HOVER: f32 = 0.85;
const LYRICS_DIM_NEAR: f32 = 0.5;
const LYRICS_DIM_STEP: f32 = 0.08;
const LYRICS_DIM_FAR: f32 = 0.16;
const LYRICS_CHROME: f32 = 300.0;
/// Как часто запоминать место в треке.
const SESSION_SAVE: Duration = Duration::from_secs(10);
/// Шаг перемотки и громкости с клавиатуры.
const SEEK_STEP: f32 = 5.0;
/// Сколько других альбомов исполнителя показать под альбомом.
const ALBUM_OTHERS: usize = 8;
const VOLUME_STEP: f32 = 0.05;
const DIALOG_INPUT: &str = "dialog";
const DIALOG_WIDTH: f32 = 400.0;
/// Сколько окна остаётся вокруг сведений о треке.
const INFO_MARGIN: f32 = 80.0;
/// Сколько последних треков показать в истории.
const HISTORY_LIMIT: usize = 200;
/// Сколько плейлистов видно в меню трека без прокрутки.
const MENU_LISTS: usize = 8;
/// Плейлист «Мне нравится»: его нельзя ни менять, ни удалять как обычный.
const FAVORITES_KIND: &str = "3";
/// Сколько исполнителей проверить, когда страницу ищем по имени.
const ARTIST_SEARCH: u32 = 5;
/// Длительность появления страницы и выезда очереди, в секундах.
const PAGE_FADE: f32 = 0.26;
/// На сколько точек новая страница сдвинута в начале появления.
const PAGE_SHIFT: f32 = 22.0;
const UNAVAILABLE: &str = "Трек недоступен для прослушивания";
/// Подписи и обложка недоступного трека.
const FAINT: iced::Color = iced::Color { r: 1.0, g: 1.0, b: 1.0, a: 0.3 };
const UNAVAILABLE_COVER: f32 = 0.35;
/// Как часто во время звучания уходит промежуточный отчёт о прослушивании.
const LISTEN_REPORT: f32 = 30.0;
/// За сколько секунд проявляется содержимое другой вкладки.
const AREA_FADE: f32 = 0.2;
/// Смена трека в Моей волне.
const TRACK_FADE: f32 = 0.3;
/// Появление и исчезновение меню, окон и уведомлений, в секундах.
const POPUP_OPEN: f32 = 0.16;
const POPUP_CLOSE: f32 = 0.1;
const DIALOG_OPEN: f32 = 0.2;
/// Плеер на всё окно: место под шапку, название, перемотку и кнопки; предел обложки; насколько темнеет цвет фона.
const FULL_CHROME: f32 = 330.0;
/// Высота ползунков эквалайзера.
const EQ_HEIGHT: f32 = 150.0;
/// Эквалайзер из трекбара: ширина и высота окна, высота ползунков в нём.
const EQ_POPUP_WIDTH: f32 = 520.0;
const EQ_POPUP_HEIGHT: f32 = 336.0;
/// Насколько выше точки клика по кнопке кончается окно эквалайзера, чтобы не закрывать трекбар.
const EQ_POPUP_LIFT: f32 = 76.0;
const EQ_POPUP_SLIDER: f32 = 120.0;
const FULL_COVER_MAX: f32 = 560.0;
const FULL_TINT: f32 = 0.55;
/// Как быстро фон перетекает в цвет обложки нового трека: чем больше, тем короче переход.
const FULL_TINT_SPEED: f32 = 5.0;
const DIALOG_CLOSE: f32 = 0.13;
/// За сколько секунд плеер на всё окно выезжает и уезжает.
const FULL_OPEN: f32 = 0.32;
const FULL_CLOSE: f32 = 0.24;
const NOTICE_OPEN: f32 = 0.22;
const NOTICE_CLOSE: f32 = 0.18;
/// Сколько времени самое большее учитывает один кадр: первый кадр страницы бывает долгим.
const FRAME_LIMIT: f32 = 0.05;
/// Самая длинная подпись волны.
const WAVE_LABEL: usize = 56;
/// Заголовок страницы волны: пределы размера, ширина буквы в долях размера и поля по бокам.
const WAVE_TITLE_MAX: f32 = 56.0;
const WAVE_TITLE_MIN: f32 = 32.0;
const WAVE_TITLE_CHAR: f32 = 0.62;
const WAVE_TITLE_GAP: f32 = 80.0;
const WAVE_CARD: &str = "wave:";
/// Сколько треков показать в блоке главной.
const HOME_TRACKS: usize = 5;
/// Сколько места занимает карточка вместе с отступами.
const CARD_SLOT: f32 = CARD + 20.0;
/// Ширина содержимого, на которой нижний плеер отказывается от лишнего.
/// Уже этой ширины в строке трека нет места для длительности.
const TRACK_TIME_WIDTH: f32 = 520.0;
/// Место под значок «E» после названия: сам значок и отступ.
const EXPLICIT_RESERVE: f32 = 22.0;
/// Как часто без сети проверять, не вернулась ли она.
const ONLINE_CHECK: Duration = Duration::from_secs(10);
/// Место под «123 трека» после названия плейлиста.
const PLAYLIST_COUNT_RESERVE: f32 = 110.0;
/// Ширина колонки времени: у музыки «03:25», у выпусков «осталось 1:47:20».
const TIME_WIDTH: f32 = 72.0;
const TIME_WIDTH_EPISODE: f32 = 150.0;
/// От этой ширины коллекция показывает треки в две колонки.
const COLLECTION_COLUMNS: f32 = 860.0;
const PAGE_COVER_SMALL: f32 = 120.0;
const PAGE_COVER_WIDTH: f32 = 640.0;
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
/// Выпуски и книги: шаг кнопок перемотки, как часто место уходит Яндексу, и границы,
/// внутри которых выпуск продолжается с места, а не с начала.
const PODCAST_STEP: f32 = 15.0;
const PROGRESS_SAVE: Duration = Duration::from_secs(30);
const PROGRESS_MIN: f32 = 5.0;
const PROGRESS_TAIL: f32 = 10.0;
/// Высота шапки страницы очереди: заголовок, играющий трек и «Далее в очереди».
/// Сколько карточек показываем в «Похожем».
const SIMILAR_LIMIT: usize = 12;
const QUEUE_HEADER: f32 = 200.0;
/// За сколько секунд очередь доезжает до начала, когда включили трек ниже.
const QUEUE_SCROLL: f32 = 0.45;
/// Подпись источника начинается под заголовком, правее кнопки «назад».
const QUEUE_SOURCE_INSET: f32 = 48.0;
const QUEUE_SOURCE_SIZE: f32 = 14.0;
const QUEUE_SOURCE_LABEL: usize = 60;
const QUALITY_WIDTH: f32 = 250.0;
const SKELETON_ROWS: usize = 10;
const SKELETON_BLOCKS: usize = 3;
/// Высота строк настоящей страницы, чтобы после загрузки ничего не сдвигалось.
const SKELETON_HEAD: f32 = 32.0;
const SKELETON_TITLE: f32 = 21.0;
const SKELETON_SUBTITLE: f32 = 17.0;
const SKELETON_TAB: f32 = 35.0;
const SKELETON_TABS: [f32; 2] = [89.0, 87.0];
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Section {
    Home,
    Wave,
    /// Книги и подкасты: такая же страница из блоков, как главная.
    Books,
    /// Детский раздел, тоже страница из блоков.
    Kids,
    /// Скачанные треки: играют и без интернета.
    Downloads,
    Collection,
    Likes,
    Playlists,
    Artist,
    Albums,
    Artists,
    Queue,
    History,
    Dislikes,
    Search,
    Lyrics,
    Settings,
    /// Все релизы исполнителя или сборники с ним.
    Discography,
    /// Плейлисты микса.
    Tag,
}

/// Список, в котором перетаскивают трек.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DragList {
    Queue,
    Playlist,
}

/// Идёт перетаскивание: подписка ловит отпускание кнопки где угодно в окне.
static DRAGGING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Удаление, которое ещё можно отменить: в окне элемента уже нет, на сервер оно уйдёт
/// по истечении уведомления. Место нужно, чтобы вернуть элемент туда же.
#[derive(Clone, Debug)]
enum Pending {
    Playlist(Playlist, usize),
    Track(Playlist, Track, usize),
}

/// Окно поверх страницы.
/// Откуда взята очередь: подпись на её странице и переход обратно.
#[derive(Clone, Debug, Default)]
enum QueueFrom {
    /// Подборка с главной или очередь, восстановленная после запуска.
    #[default]
    Unknown,
    List(Playlist),
    Artist(String),
    Likes,
    History,
    Dislikes,
    Search,
    Downloads,
}

#[derive(Clone, Debug, PartialEq)]
enum Dialog {
    Create,
    Rename(Playlist),
    Describe(Playlist),
    Delete(Playlist),
    /// Убрать трек из своего плейлиста.
    RemoveTrack(Track),
    WaveTune,
    TrackInfo(Track),
    Logout,
    Account,
    /// Удалить всё скачанное с компьютера.
    ClearDownloads,
    /// Скачать все треки «Мне нравится».
    DownloadLikes,
}

/// Вкладка поиска.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    Dislikes,
    History,
    Dialog,
    Home,
    /// Вторая вкладка главной.
    HomeTrends,
    Wave,
    /// Главная с перемоткой в самом начале трека.
    WaveSeek,
    /// Волна по исполнителю.
    WaveSeeded,
    WaveTune,
    Lyrics,
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
    /// Исполнитель, прокрученный до описания.
    ArtistAbout(String),
    /// Микс по тегу.
    Tag(String),
    /// Пустой поиск с недавними запросами; в файл они не пишутся.
    SearchHistory,
    /// Поиск сам перебирает вкладки: номер следующей.
    SearchTabs(usize),
    /// Поиск набирает запрос по буквам и пишет в stderr, где фокус.
    SearchFocus,
    /// Лайки прокручиваются, открывается главная и снова лайки: в stderr — где прокрутка.
    ScrollReturn,
    /// Восстановленная очередь и плеер на всё окно.
    Full,
    EqBar,
    PageMenu,
    Books,
    Downloads,
    Kids,
    /// Восстановленная очередь на странице волны, без звука.
    WaveRestored,
    /// В лайках набирается фильтр по буквам: в stderr — остался ли фокус.
    FilterFocus,
    /// Все релизы исполнителя.
    Discography(String),
    /// Первый плейлист в очереди, открыта главная.
    PlaylistHome,
    /// Открытое меню первого трека.
    Menu,
    Search(String),
    /// Поиск внутри лайков.
    Filter(String),
    /// Сведения о первом треке лайков.
    TrackInfo,
    Account,
    LogoutAsk,
    /// Вкладка настроек.
    Settings,
    /// Очередь, сохранённая при прошлом выходе.
    Restored,
    PinMenu,
    /// Нажатие на закреп под этим номером, без звука.
    PinPlay(usize),
    /// Плейлист по владельцу и номеру или альбом, прокрученные до «Похожего».
    SimilarOf(Playlist),
    /// Очередь, прокрученная вниз, в которой включается трек ниже.
    QueueJump,
}

struct App {
    screen: Screen,
    section: Section,
    /// Откуда пришли: кнопка «назад» и боковая кнопка мыши.
    history: Vec<Place>,
    profile: Option<Profile>,
    query: String,
    results: Vec<Track>,
    home: Home,
    home_tab: usize,
    home_loading: bool,
    /// Чья страница из блоков сейчас загружена: главная или книги и подкасты.
    home_page: Section,
    /// Последняя открытая вкладка главной и книг с подкастами.
    landing_tabs: [usize; 3],
    /// Меню «…» страницы плейлиста, альбома или исполнителя открыто в этой точке.
    page_menu: Option<(f32, f32)>,
    page_menu_fade: Fade<(f32, f32)>,
    /// Удаление, которое пока можно отменить, и номер его уведомления.
    pending: Option<(u64, Pending)>,
    /// Окно приложения: к нему привязываются системные окна.
    hwnd: Option<isize>,
    /// Перетаскивание: список, откуда взят трек, и над какой строкой он сейчас (ниже ли середины).
    drag: Option<(DragList, usize, Option<(usize, bool)>)>,
    /// Где остановились в выпусках и книгах за этот запуск: свежее ответа сервера.
    progress: HashMap<String, f32>,
    /// Когда место в выпуске последний раз ушло Яндексу.
    progress_at: Option<Instant>,
    /// С какой карточки показан блок главной.
    home_pages: HashMap<usize, usize>,
    /// Выбранная вкладка в блоках главной, у которых они есть.
    home_block_tabs: HashMap<usize, usize>,
    found: Found,
    search_tab: SearchTab,
    /// Что уже искали: для вкладок и подсказки.
    searched: bool,
    /// Уже полученные вкладки текущего запроса: возврат на них не ждёт сети.
    search_cache: HashMap<(String, SearchTab), Found>,
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
    /// В меню трека открыт список его исполнителей.
    menu_artists: bool,
    menu_lists: bool,
    dialog: Option<Dialog>,
    dialog_text: String,
    history_tracks: Vec<Track>,
    history_loading: bool,
    /// Скачанные треки, их номера и сами треки для списка; новые первыми.
    downloads: Vec<offline::Saved>,
    download_ids: HashSet<String>,
    download_tracks: Vec<Track>,
    /// Что ждёт скачивания, что качается сейчас и итог текущей пачки: скачано и не вышло.
    download_queue: VecDeque<Track>,
    download_now: Option<String>,
    download_done: usize,
    download_failed: usize,
    /// Сколько треков в текущей пачке всего: для «Скачиваем 3 из 20».
    download_batch: usize,
    /// Нет сети: вместо ошибок спокойная плашка, играет только скачанное, связь проверяется сама.
    offline: bool,
    online_checking: bool,
    /// Сведения о треке для окна: номер трека и строки. Пока идёт загрузка, строк нет.
    track_info: Option<(String, api::TrackDetails)>,
    /// Поиск внутри открытого плейлиста или лайков.
    filter: String,
    filter_tracks: Vec<Track>,
    filter_loading: bool,
    filter_id: u64,
    disliked_list: Vec<Track>,
    liked_lists: Vec<Playlist>,
    /// Описание и приватность открытого плейлиста.
    list_meta: Option<(String, String, String)>,
    notice: Option<(u64, String, bool)>,
    liked: Vec<Track>,
    likes_loading: bool,
    pins: Vec<Pin>,
    /// Меню закрепа: его номер и точка клика.
    pin_menu: Option<(usize, f32, f32)>,
    pin_fade: Fade<(usize, f32, f32)>,
    /// Последний нажатый закреп: ответ на прошлое нажатие уже не нужен.
    pin_wanted: u64,
    /// Сколько запросов на закрепление ещё идёт: пока они идут, старому списку верить нельзя.
    pin_saving: u32,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    /// Открытый плейлист и его треки; показывается только начало списка.
    playlist: Option<Playlist>,
    playlist_tracks: Vec<Track>,
    /// Шапка открытого альбома.
    album: Option<Album>,
    /// Все выпуски подкаста по порядку сервера; на странице — выбранный сезон в выбранном порядке.
    episodes: Vec<Track>,
    /// Выпуски сначала старые; по умолчанию сначала новые.
    episodes_old_first: bool,
    /// Выбранный сезон подкаста, с нуля; `None` — все сезоны.
    season: Option<usize>,
    artist: Option<Artist>,
    /// Какого исполнителя сейчас загружаем.
    artist_loading: Option<String>,
    /// Какой трек качается заранее.
    preloaded: Option<String>,
    covers: HashMap<String, image::Handle>,
    /// Только что загруженные обложки и насколько они проявились.
    cover_fades: HashMap<iced::advanced::image::Id, f32>,
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
    /// Эквалайзер: включён ли, пресет, предусиление и полосы.
    eq: equalizer::EqSettings,
    /// Плавные переходы между треками.
    crossfade: bool,
    /// Показывать пункт «Детям» в боковой панели.
    kids: bool,
    /// Следующий запуск — переход: где затихает играющий трек.
    cross_out: Option<(f32, f32)>,
    /// Что играет: смена раздела и новый поиск не прерывают музыку.
    queue: Vec<Track>,
    /// Сессия волны, если очередь — это волна.
    wave: Option<String>,
    /// Волна загружена, но ещё не запущена: сессия начнётся с первым треком.
    wave_pending: Option<String>,
    /// Откуда играет очередь, если это не волна.
    queue_title: Option<String>,
    queue_from: QueueFrom,
    /// Похожее на открытый альбом или плейлист.
    similar: Vec<HomeItem>,
    /// Превью: после загрузки похожего перейти в конец страницы.
    preview_end: bool,
    /// Плавный подъём очереди: откуда едем и как далеко прошли, от 0 до 1.
    queue_scroll: Option<(f32, f32)>,
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
    /// Кодек и битрейт играющего файла.
    sound: Option<api::Sound>,
    /// Трек перезапускается в другом качестве; внутри — был ли он на паузе.
    restart: Option<bool>,
    /// После перемотки на прошлое место трек надо пустить.
    unhold: bool,
    /// Страницы, с которых ушли кнопкой «назад».
    forward: Vec<Place>,
    /// Выбор качества открыт в этой точке.
    quality_menu: Option<(f32, f32)>,
    /// Эквалайзер из трекбара открыт в этой точке.
    eq_menu: Option<(f32, f32)>,
    /// Что ещё закрывает новую страницу, от 1 до 0.
    page_veil: f32,
    /// Текущее прослушивание для отчётов `plays`.
    listen: Option<api::Listen>,
    /// Где была прокрутка на каждой странице.
    scroll_places: HashMap<(Section, Option<String>, Option<String>), f32>,
    /// Прокрутка ждёт, пока страница загрузится.
    scroll_wait: Option<f32>,
    listen_tick: Option<Instant>,
    /// После скольких секунд звучания уйдёт следующий промежуточный отчёт.
    listen_next: f32,
    /// Почему сменится трек: доиграл, пропущен, назад.
    change_reason: Option<&'static str>,
    /// То же для области под вкладками поиска и главной.
    area_veil: f32,
    /// Откуда въезжает страница.
    page_shift: (f32, f32),
    /// Насколько выехала верхняя строка трекбара, от 0 до 1.
    bar_top: f32,
    /// Что ещё закрывает новый трек Моей волны, от 1 до 0.
    track_veil: f32,
    shown_track: Option<String>,
    menu_fade: Fade<(Track, f32, f32, bool, bool)>,
    quality_fade: Fade<(f32, f32)>,
    eq_fade: Fade<(f32, f32)>,
    dialog_fade: Fade<Dialog>,
    /// Плеер на всё окно: открыт ли и как далеко выехал.
    player_full: bool,
    full_fade: Fade<()>,
    /// Средний цвет крупной обложки трека: им окрашен фон плеера на всё окно.
    cover_tints: HashMap<String, iced::Color>,
    /// Цвет фона плеера на всё окно сейчас: догоняет цвет обложки играющего трека.
    full_tint: iced::Color,
    notice_fade: Fade<(String, bool)>,
    /// От чего запущена волна; пусто у обычной Моей волны.
    wave_title: Option<String>,
    /// Выбранные настройки волны и зёрна, к которым они добавляются.
    wave_tuning: [usize; 4],
    wave_base: Vec<String>,
    /// Текст: для какого трека он загружен, что загружается, куда прокручена страница.
    lyrics: Option<(String, Option<api::Lyrics>)>,
    lyrics_loading: Option<String>,
    lyrics_line: Option<usize>,
    /// Текст только что открыт: на звучащую строку встаём сразу, без перемотки сверху.
    lyrics_fresh: bool,
    /// Где фокус: номер строки, дробный посреди перехода. До первой строки это -1.
    lyrics_shown: f32,
    lyrics_target: f32,
    /// Где текст стоит сейчас, от 0 до 1.
    lyrics_view: f32,
    /// Насколько текст ушёл от звучащей строки, и как далеко прошёл возврат, от 0 до 1.
    lyrics_gap: f32,
    lyrics_back: f32,
    /// Место в треке после перезапуска: действует, пока трек не запустили.
    resume: Option<Duration>,
    /// Куда перемотать, как только трек запустится.
    resume_seek: Option<Duration>,
    /// Какая очередь уже на диске и когда запоминали место.
    session_key: Option<u64>,
    session_at: Option<Instant>,
    /// «Меньше ИИ»; пока не знаем ответа сервера, переключатель не работает.
    ai_less: Option<bool>,
    discography: Vec<Card>,
    /// Открытый микс: тег, название и его плейлисты карточками главной.
    tag: Option<(String, String)>,
    tag_lists: Vec<HomeItem>,
    tag_loading: bool,
    searches: Vec<String>,
    /// Другие альбомы исполнителя открытого альбома.
    album_others: Vec<Card>,
    discography_total: u64,
    discography_also: bool,
    discography_by_year: bool,
    /// Какой запрос релизов ждём: старые ответы отбрасываются.
    discography_key: Option<String>,
    /// Какую волну ждём: опоздавший ответ о прежней не должен её заменить.
    wave_wanted: Vec<String>,
    last_frame: Option<Instant>,
    error: Option<String>,
    preview: Option<Preview>,
}

#[derive(Clone, Debug)]
enum Message {
    LoginOpen,
    LoginFinished,
    Logout,
    Account(Result<(Profile, Vec<String>, Vec<String>), String>),
    AccountRetry,
    Back,
    Open(Section),
    Query(String),
    Search,
    Found(u64, SearchTab, String, Result<Found, String>),
    SearchTyped(u64),
    MenuLyrics,
    TrackInfoLoaded(String, Result<api::TrackDetails, String>),
    /// Включить похожий трек из окна сведений.
    TrackInfoPlay(usize),
    Filter(String),
    FilterTyped(u64),
    FilterLoaded(u64, Result<Vec<Track>, String>),
    SearchTabSet(SearchTab),
    ListOpen(Playlist),
    HomeLoaded(Section, usize, Result<Home, String>),
    HomeTab(usize),
    HomePage(usize, bool),
    /// Вкладка внутри блока главной.
    HomeBlockTab(usize, usize),
    HomePlay(usize, usize),
    HomeOpen(HomeItem),
    LikedPages(Result<(Vec<Card>, Vec<Card>), String>),
    AlbumLike,
    /// Порядок выпусков (сначала старые) и выбор сезона на странице подкаста.
    EpisodesOrder(bool),
    SeasonPick(Option<usize>),
    /// Отметка «прослушано» у выпуска или главы и у подкаста или книги целиком.
    EpisodeFinished(Track, bool),
    AlbumFinished(bool),
    Marked(Result<(), String>),
    ArtistLike,
    PageLikeSaved(Result<(), String>),
    LikesMore,
    LikesPage(usize, Result<Vec<Track>, String>),
    Scrolled(f32),
    LyricsScrolled(f32, f32),
    Retry,
    ListReload,
    AlbumOpen(String, String),
    AlbumLoaded(String, Result<(Album, Vec<Track>), String>),
    ArtistOpen(String),
    ArtistByName(String),
    ArtistFound(String, Result<Found, String>),
    ArtistLoad(String),
    ArtistLoaded(String, Result<Artist, String>),
    ArtistTracks,
    ToggleMute,
    RepeatNext,
    ShuffleToggle,
    QualityOpen,
    QualitySet(Quality),
    QueueToggle,
    QueueSource,
    PreviewQueueDown,
    PreviewArtistEnd,
    PreviewSearchTab(usize),
    PreviewType(usize),
    PreviewScroll(usize),
    PreviewFilter(usize),
    PreviewFocused(usize, bool),
    Frame(Instant),
    QueuePlay(usize),
    QueueRemove(usize),
    Like(Track),
    LikeSaved(Track, bool, Result<(), String>),
    WaveStart,
    /// Какая волна играла в прошлый раз.
    WaveLast(Result<(Vec<String>, Option<String>), String>),
    /// Обратно к Моей волне без настроек.
    WaveReset,
    /// Группа настроек и выбранный вариант.
    WaveTune(usize, usize),
    WaveTuneReset,
    LyricsToggle,
    LyricsLoaded(String, Result<Option<api::Lyrics>, String>),
    LyricsSeek(f32),
    /// Зёрна волны и подпись, откуда она запущена.
    WaveStartSeeds(Vec<String>, Option<String>),
    WaveStarted(bool, Vec<String>, Option<String>, Result<Wave, String>),
    WaveMore(Result<Vec<Track>, String>),
    Cover(String, image::Handle),
    PlaylistsLoaded(Result<Vec<Playlist>, String>),
    PlaylistOpen(usize),
    PinOpen(usize),
    PinPage(usize),
    PinMenu(usize),
    PinTracks(u64, Playlist, String, Result<Vec<Track>, String>),
    OpenLink(&'static str),
    /// Ссылка из ответа Яндекса, например официальная страница исполнителя.
    OpenUrl(String),
    /// Все релизы исполнителя; `true` — сборники.
    DiscographyOpen(bool),
    /// Сортировка по дате выхода или по популярности.
    DiscographySort(bool),
    DiscographyMore,
    TagLoaded(String, Result<(Option<String>, Vec<HomeItem>), String>),
    /// Повторить недавний запрос.
    SearchAgain(String),
    SearchHistoryClear,
    DiscographyLoaded(String, Result<api::Releases, String>),
    /// Другие альбомы исполнителя для страницы альбома.
    AlbumOthers(String, Result<api::Releases, String>),
    /// Окно закрывают: сначала надо запомнить, что играло.
    Quit,
    /// Новая сессия для волны, восстановленной после запуска.
    WaveResumed(Vec<String>, Result<api::Wave, String>),
    AiLessLoaded(Result<bool, String>),
    AiLessSet(bool),
    AiLessSaved(Result<(), String>),
    PinsLoaded(Result<Vec<Pin>, String>),
    PlaylistLoaded(String, Result<Vec<Track>, String>),
    SimilarLoaded(String, Result<Vec<HomeItem>, String>),
    LinkTrack(Result<Vec<Track>, String>),
    LinkPlaylist(Result<Playlist, String>),
    MenuOpen(Track),
    MenuClose,
    MenuArtists,
    MenuLists,
    DialogOpen(Dialog),
    DialogInput(String),
    DialogConfirm,
    PlaylistCreated(Result<Playlist, String>),
    PlaylistRenamed(String, String, Result<(), String>),
    PlaylistDeleted(String, Result<(), String>),
    TrackAdd(Track, Playlist),
    TrackAdded(String, String, Result<(), String>),
    TrackRemove(Track),
    TrackRemoved(Result<(), String>),
    /// Нажали на недоступный трек.
    Unavailable,
    /// Эквалайзер: открыть из трекбара, включить, выбрать пресет, сдвинуть полосу или предусиление, отпустить ползунок.
    EqOpen,
    /// Перетаскивание трека: взяли за ручку, курсор над строкой (номер и высота в ней), отпустили.
    /// Своя обложка плейлиста: выбрать картинку и загрузить; ответ — адрес новой обложки,
    /// `None` в нём — окно выбора закрыли.
    CoverPick(Playlist),
    /// Меню «…» страницы: открыть и выбрать пункт (меню закрывается, пункт выполняется).
    PageMenuOpen,
    PageMenuPick(Box<Message>),
    /// Отменить удаление; время на отмену вышло.
    Undo,
    UndoExpire(u64),
    CoverUploaded(Playlist, Result<Option<Option<String>>, String>),
    DragStart(DragList, usize),
    DragOver(usize, f32),
    DragEnd,
    TrackMoved(Result<(), String>),
    CrossfadeToggle,
    /// Проверка связи без сети и её итог.
    OnlineCheck,
    OnlineChecked(Result<(), String>),
    /// Скачать треки, итог одного трека, убрать скачанные по номерам, убрать всё.
    Download(Vec<Track>),
    Downloaded(String, Result<offline::Saved, String>),
    DownloadRemove(Vec<String>),
    DownloadsClear,
    /// Не качать то, что ещё в очереди скачивания: текущий трек докачается.
    DownloadStop,
    /// «Мне нравится» целиком: сначала сведения обо всех треках, потом скачивание.
    DownloadLikes,
    LikesForDownload(Result<Vec<Track>, String>),
    KidsToggle,
    EqToggle,
    EqPreset(usize),
    EqBand(usize, f32),
    EqPreamp(f32),
    EqSave,
    /// Плеер на всё окно: открыть или закрыть.
    FullToggle,

    /// Ответ, какие треки очереди сейчас можно слушать.
    Availability(Result<Vec<(String, bool)>, String>),
    HistoryLoaded(Result<Vec<Track>, String>),
    DislikesLoaded(Result<Vec<Track>, String>),
    LikedLists(Result<Vec<Playlist>, String>),
    ListLike(Playlist),
    ListMeta(String, Result<(String, String), String>),
    Visibility(Playlist),
    Pin(PinRef),
    PinSaved(Result<(), String>),
    CopyText(String),
    SimpleSaved(Result<(), String>),
    Dislike(Track),
    DislikeSaved(String, bool, Result<(), String>),
    PlayNext(Track),
    PlayLast(Track),
    TrackWave(Track),
    OpenAlbum(Track),
    CopyLink(Track),
    NoticeClear(u64),
    WindowReady(Option<isize>),
    WindowSize(Size),
    Media(media::Event),
    Play(Section, usize),
    Started(u64, Result<api::Sound, String>),
    Toggle,
    Next,
    Previous,
    Tick,
    Seek(f32),
    SeekDone,
    /// Горячие клавиши: перемотка и громкость на шаг, оценка играющего трека.
    SeekBy(f32),
    VolumeBy(f32),
    LikeCurrent,
    DislikeCurrent,
    Forward,
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
        let downloads = offline::load();
        let token = api::load_token();
        let task = match &token {
            Some(token) => load_account(token.clone()),
            None => Task::done(Message::LoginOpen),
        };
        let app = Self {
            screen: token.map_or(Screen::Login(false), Screen::Main),
            section: Section::Wave,
            history: Vec::new(),
            profile: None,
            query: String::new(),
            results: Vec::new(),
            home: Home::default(),
            home_tab: 0,
            home_loading: false,
            home_page: Section::Home,
            landing_tabs: [0; 3],
            progress: HashMap::new(),
            progress_at: None,
            drag: None,
            hwnd: None,
            page_menu: None,
            page_menu_fade: Fade::new(POPUP_OPEN, POPUP_CLOSE),
            pending: None,
            home_pages: HashMap::new(),
            home_block_tabs: HashMap::new(),
            found: Found::default(),
            search_tab: SearchTab::All,
            searched: false,
            search_cache: HashMap::new(),
            liked_albums: Vec::new(),
            liked_artists: Vec::new(),
            searching: false,
            search_id: 0,
            uid: None,
            liked_ids: Vec::new(),
            liked_set: HashSet::new(),
            disliked: HashSet::new(),
            menu: None,
            menu_artists: false,
            menu_lists: false,
            dialog: None,
            dialog_text: String::new(),
            history_tracks: Vec::new(),
            history_loading: false,
            download_ids: downloads.iter().map(|saved| saved.track.id.clone()).collect(),
            download_tracks: downloads.iter().map(|saved| saved.track.clone()).collect(),
            downloads,
            download_queue: VecDeque::new(),
            download_now: None,
            download_done: 0,
            download_failed: 0,
            download_batch: 0,
            offline: false,
            online_checking: false,
            track_info: None,
            filter: String::new(),
            filter_tracks: Vec::new(),
            filter_loading: false,
            filter_id: 0,
            disliked_list: Vec::new(),
            liked_lists: Vec::new(),
            list_meta: None,
            notice: None,
            liked: Vec::new(),
            likes_loading: false,
            pins: Vec::new(),
            pin_menu: None,
            pin_fade: Fade::new(POPUP_OPEN, POPUP_CLOSE),
            pin_wanted: 0,
            pin_saving: 0,
            playlists: Vec::new(),
            playlists_loading: false,
            playlist: None,
            playlist_tracks: Vec::new(),
            album: None,
            episodes: Vec::new(),
            episodes_old_first: false,
            season: None,
            artist: None,
            artist_loading: None,
            preloaded: None,
            covers: HashMap::new(),
            cover_fades: HashMap::new(),
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
            queue_from: QueueFrom::Unknown,
            similar: Vec::new(),
            preview_end: false,
            queue_scroll: None,
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
            eq: saved.eq,
            crossfade: saved.crossfade,
            kids: saved.kids,
            cross_out: None,
            sound: None,
            restart: None,
            unhold: false,
            forward: Vec::new(),
            quality_menu: None,
            eq_menu: None,
            page_veil: 0.0,
            listen: None,
            scroll_places: HashMap::new(),
            scroll_wait: None,
            listen_tick: None,
            listen_next: LISTEN_REPORT,
            change_reason: None,
            area_veil: 0.0,
            page_shift: (0.0, 0.0),
            bar_top: 0.0,
            track_veil: 0.0,
            shown_track: None,
            menu_fade: Fade::new(POPUP_OPEN, POPUP_CLOSE),
            quality_fade: Fade::new(POPUP_OPEN, POPUP_CLOSE),
            eq_fade: Fade::new(POPUP_OPEN, POPUP_CLOSE),
            dialog_fade: Fade::new(DIALOG_OPEN, DIALOG_CLOSE),
            player_full: false,
            full_fade: Fade::new(FULL_OPEN, FULL_CLOSE),
            cover_tints: HashMap::new(),
            full_tint: ui::TILE,
            notice_fade: Fade::new(NOTICE_OPEN, NOTICE_CLOSE),
            wave_title: None,
            wave_tuning: saved.wave,
            wave_base: vec![api::MY_WAVE.to_string()],
            lyrics: None,
            lyrics_loading: None,
            lyrics_line: None,
            lyrics_fresh: true,
            lyrics_shown: -1.0,
            lyrics_target: -1.0,
            lyrics_view: 0.0,
            lyrics_gap: 0.0,
            lyrics_back: 1.0,
            resume: None,
            resume_seek: None,
            session_key: None,
            session_at: None,
            ai_less: None,
            discography: Vec::new(),
            tag: None,
            tag_lists: Vec::new(),
            tag_loading: false,
            searches: session::searches(),
            album_others: Vec::new(),
            discography_total: 0,
            discography_also: false,
            discography_by_year: false,
            discography_key: None,
            wave_wanted: Vec::new(),
            last_frame: None,
            error,
            preview,
        };
        app.apply_volume();
        if let Some(player) = &app.player {
            player.set_eq(app.eq);
        }
        (app, Task::batch([task, window_handle(), window::latest().then(|id| id.map_or(Task::none(), window::size)).map(Message::WindowSize)]))
    }

    fn animating(&self) -> bool {
        self.pin_fade.busy() || self.queue_scroll.is_some() || (self.section == Section::Lyrics && self.lyrics_back < 1.0) || self.page_veil > 0.0 || self.area_veil > 0.0 || self.track_veil > 0.0 || self.bar_top != self.bar_top_target() || (self.section == Section::Lyrics && self.lyrics_shown != self.lyrics_target) || !self.cover_fades.is_empty() || self.menu_fade.busy() || self.quality_fade.busy() || self.eq_fade.busy() || self.page_menu_fade.busy() || self.dialog_fade.busy() || self.full_fade.busy() || (self.full_fade.item.is_some() && !same_color(self.full_tint, self.full_target())) || self.notice_fade.busy()
    }

    /// Сколько места занимает основная часть окна.
    fn main_width(&self) -> f32 {
        let (width, _) = *WINDOW.lock().unwrap();
        width - SIDEBAR - 12.0
    }

    /// Свой обычный плейлист: его можно менять.
    fn is_favorites(&self, list: &Playlist) -> bool {
        list.source == Source::Playlist && list.kind == FAVORITES_KIND && self.uid.as_deref() == Some(list.owner.as_str())
    }

    fn queue_from_list(&self, list: Playlist) -> QueueFrom {
        if self.is_favorites(&list) {
            QueueFrom::Likes
        } else if list.source == Source::Artist {
            QueueFrom::Artist(list.kind)
        } else {
            QueueFrom::List(list)
        }
    }

    /// Что показать из ответа сервера: без повторов.
    fn pins_shown(&self, pins: Vec<Pin>) -> Vec<Pin> {
        let mut seen = HashSet::new();
        pins.into_iter().filter(|pin| seen.insert(pin.key())).collect()
    }

    /// Сразу показывает новый закреп по тому, что уже известно о странице.
    fn pin_show(&mut self, target: &PinRef) -> Task<Message> {
        let pin = match target {
            PinRef::Wave(seeds) => {
                let title = self.wave_title.clone().unwrap_or_else(|| "Волна".to_string());
                Some(Pin { target: PinTarget::Wave(seeds.clone()), title, subtitle: "Волна".to_string(), cover: None, round: true, color: None })
            }
            PinRef::Album(id) => self.album.as_ref().filter(|album| album.id == *id).map(|album| {
                let list = Playlist { cover: album.cover.clone(), ..Playlist::album(id, &album.title) };
                Pin { target: PinTarget::List(list), title: album.title.clone(), subtitle: "Альбом".to_string(), cover: album.cover.clone(), round: false, color: None }
            }),
            PinRef::Artist(id) => self.artist.as_ref().filter(|artist| artist.id == *id).map(|artist| {
                let list = Playlist { cover: artist.cover.clone(), ..Playlist::artist(id, &artist.name) };
                Pin { target: PinTarget::List(list), title: artist.name.clone(), subtitle: "Исполнитель".to_string(), cover: artist.cover.clone(), round: true, color: None }
            }),
            // «Мне нравится» открыто отдельной страницей, а не как плейлист.
            PinRef::Playlist(owner, kind) if kind == FAVORITES_KIND => {
                let list = Playlist { source: Source::Playlist, owner: owner.clone(), kind: kind.clone(), title: "Мне нравится".to_string(), count: 0, cover: None };
                Some(Pin { title: list.title.clone(), subtitle: "Плейлист".to_string(), cover: None, target: PinTarget::List(list), round: false, color: None })
            }
            PinRef::Playlist(..) => self.playlist.clone().map(|list| Pin { title: list.title.clone(), subtitle: "Плейлист".to_string(), cover: list.cover.clone(), target: PinTarget::List(list), round: false, color: None }),
        };
        let Some(pin) = pin else { return Task::none() };
        let job = pin.cover.clone().map(|cover| (format!("{PIN_COVER_PREFIX}{}", pin.key()), cover));
        self.pins.insert(0, pin);
        self.fetch_cover_jobs(job.into_iter().collect(), "100x100")
    }

    /// Сейчас играет то, что на этом закрепе.
    fn pin_active(&self, pin: &Pin) -> bool {
        match &pin.target {
            PinTarget::Wave(seeds) => self.wave_queue && self.wave_base == *seeds,
            PinTarget::List(_) if self.wave_queue => false,
            PinTarget::List(list) => match (&self.queue_from, self.queue_from_list(list.clone())) {
                (QueueFrom::Likes, QueueFrom::Likes) => true,
                (QueueFrom::Artist(a), QueueFrom::Artist(b)) => *a == b,
                (QueueFrom::List(a), QueueFrom::List(b)) => a.source == b.source && a.kind == b.kind && a.owner == b.owner,
                _ => false,
            },
        }
    }

    fn is_own(&self, list: &Playlist) -> bool {
        list.source == Source::Playlist && list.kind != FAVORITES_KIND && self.uid.as_deref() == Some(list.owner.as_str())
    }

    /// Передаёт эквалайзер плееру. Во время перетаскивания ползунка файл не пишется на каждый шаг.
    fn eq_apply(&self, save: bool) {
        if let Some(player) = &self.player {
            player.set_eq(self.eq);
        }
        if save {
            self.save_settings();
        }
    }

    fn apply_volume(&self) {
        if let Some(p) = &self.player {
            p.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    fn save_settings(&self) {
        settings::save(Settings { volume: self.volume, muted: self.muted, repeat: self.repeat, shuffle: self.shuffle, quality: self.quality, wave: self.wave_tuning, eq: self.eq, crossfade: self.crossfade, kids: self.kids });
    }

    /// Куда идти дальше кнопкой или по концу трека. В волне повтора нет: она бесконечна.
    /// Недоступные треки пропускаются.
    fn next_index(&self) -> Option<usize> {
        let index = self.current?;
        let open = |i: &usize| self.queue.get(*i).is_some_and(|track| !track.unavailable);
        if let Some(next) = (index + 1..self.queue.len()).find(open) {
            return Some(next);
        }
        (self.repeat == Repeat::All && !self.wave_queue).then(|| (0..=index).find(open)).flatten()
    }

    fn row_count(&self, section: Section) -> usize {
        match section {
            // Ещё не загруженные лайки занимают своё место: полоса прокрутки не прыгает.
            Section::Likes if !self.filtering() => self.liked.len() + self.liked_ids.len().saturating_sub(self.likes_from),
            _ => self.list(section).len(),
        }
    }

    /// Какие строки открытого раздела сейчас строятся.
    fn visible(&self) -> Range<usize> {
        match self.section {
            Section::Wave => 0..0,
            Section::Collection => 0..self.liked.len().min(8),
            Section::Artist => 0..self.list(Section::Artist).len(),
            Section::Queue => visible_rows(self.list_offset - QUEUE_HEADER, WINDOW.lock().unwrap().1, self.row_count(Section::Queue)),
            Section::Search if self.search_tab == SearchTab::All => 0..self.results.len().min(SEARCH_ALL_TRACKS),
            Section::Playlists if self.playlist.as_ref().is_some_and(|p| p.source == Source::Album) => {
                visible_rows(self.list_offset - PAGE_HEADER, WINDOW.lock().unwrap().1, self.row_count(Section::Playlists))
            }
            section => visible_rows(self.list_offset, WINDOW.lock().unwrap().1, self.row_count(section)),
        }
    }

    /// Сколько карточек помещается в строку главной.
    fn cards_in_row(&self) -> usize {
        (((self.main_width() - 48.0 - SCROLLBAR) / CARD_SLOT) as usize).max(1)
    }

    fn home_shown(&self, index: usize, block: &HomeBlock) -> Range<usize> {
        let from = self.home_pages.get(&index).copied().unwrap_or(0).min(block.items.len());
        from..(from + self.cards_in_row()).min(block.items.len())
    }

    /// Обложки главной: только показанных сейчас карточек.
    fn home_covers(&self) -> Task<Message> {
        if !is_landing(self.section) {
            return Task::none();
        }
        let shown = self.home.blocks.iter().enumerate().flat_map(|(index, block)| block.items[self.home_shown(index, block)].iter());
        let cards = shown.filter_map(|item| Some((home_item_key(item), home_item_cover(item)?)));
        let tracks = self.home.blocks.iter().flat_map(|block| block.tracks.iter().take(HOME_TRACKS)).filter_map(|t| Some((t.id.clone(), t.cover.clone()?)));
        Task::batch([self.fetch_cover_jobs(cards.collect(), "200x200"), self.fetch_cover_jobs(tracks.collect(), "100x100")])
    }

    fn visible_covers(&self) -> Task<Message> {
        let range = self.visible();
        let jobs = self.list(self.section).iter().skip(range.start).take(range.len()).filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect();
        let list = self.fetch_cover_jobs(jobs, "100x100");
        let similar = if self.section == Section::Playlists { self.similar.iter().filter_map(|item| Some((home_item_key(item), home_item_cover(item)?))).collect() } else { Vec::new() };
        let similar = self.fetch_cover_jobs(similar, "200x200");
        let big = self.current_track().filter(|_| self.player_full).and_then(|t| Some((format!("{WAVE_BIG}{}", t.id), t.cover.clone()?)));
        let big = self.fetch_cover_jobs(big.into_iter().collect(), "600x600");
        // Играющий трек показан и в плеере, и крупно на главной.
        let current = self.current_track().and_then(|t| Some((t.id.clone(), t.cover.clone()?))).into_iter().collect();
        Task::batch([list, similar, self.home_covers(), self.fetch_cover_jobs(current, "300x300"), big])
    }

    fn token(&self) -> Option<String> {
        match &self.screen {
            Screen::Main(token) => Some(token.clone()),
            Screen::Login(_) => None,
        }
    }

    fn fail_retry(&mut self, error: String, retry: Message) -> Task<Message> {
        let task = self.fail(error);
        // Без сети повтор запомнен и сработает сам, когда связь вернётся.
        if self.error.is_some() || self.offline {
            self.retry = Some(retry);
        }
        task
    }

    fn fail(&mut self, error: String) -> Task<Message> {
        eprintln!("ошибка: {error}");
        self.retry = None;
        if error == NEED_LOGIN {
            self.reset();
            self.error = Some("Вход устарел, войдите ещё раз".into());
            return Task::done(Message::LoginOpen);
        }
        if api::network_error(&error) {
            self.offline = true;
            self.error = None;
            return Task::none();
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
        self.forward.clear();
        self.profile = None;
        self.results.clear();
        self.history_tracks.clear();
        self.disliked_list.clear();
        self.liked_lists.clear();
        self.list_meta = None;
        self.dialog = None;
        self.home = Home::default();
        self.home_pages.clear();
        self.home_block_tabs.clear();
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
        self.quality_menu = None;
        self.eq_menu = None;
        self.page_menu = None;
        self.pending = None;
        self.sound = None;
        self.uid = None;
        self.wave = None;
        self.wave_pending = None;
        self.queue_title = None;
        self.queue_from = QueueFrom::Unknown;
        self.wave_title = None;
        self.resume = None;
        self.resume_seek = None;
        self.session_key = None;
        session::clear();
        self.searches.clear();
        session::save_searches(Vec::new());
        self.ai_less = None;
        self.pins.clear();
        self.pin_menu = None;
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

    /// Открывает страницу, на которую ведёт ссылка.
    fn link_open(&mut self, link: api::Link, token: String) -> Task<Message> {
        match link {
            api::Link::Album(id) => self.open_list(Playlist::album(&id, "")),
            api::Link::Artist(id) => self.handle(Message::ArtistOpen(id)),
            api::Link::Track(id) => Task::perform(background(move || api::tracks_by_ids(&[id], &token)), Message::LinkTrack),
            link => Task::perform(background(move || api::link_playlist(&link, &token)), Message::LinkPlaylist),
        }
    }

    fn search_run(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let text = self.query.trim().to_string();
        self.search_id += 1;
        if let Some(link) = api::parse_link(&text) {
            self.query.clear();
            self.searching = false;
            self.searched = false;
            return self.link_open(link, token);
        }
        if text.is_empty() {
            self.searching = false;
            self.searched = false;
            self.results.clear();
            self.found = Found::default();
            return Task::none();
        }
        if let Some(found) = self.search_cache.get(&(text.clone(), self.search_tab)).cloned() {
            self.searching = false;
            return self.search_show(found);
        }
        self.searching = true;
        self.error = None;
        let id = self.search_id;
        let tab = self.search_tab;
        let size = if tab == SearchTab::All { SEARCH_ALL } else { SEARCH_TAB };
        let asked = text.clone();
        Task::perform(background(move || api::search_mixed(&asked, tab.types(), size, &token)), move |r| Message::Found(id, tab, text.clone(), r))
    }

    fn search_show(&mut self, found: Found) -> Task<Message> {
        self.results = found.tracks.clone();
        self.found = found;
        self.searched = true;
        self.prune_covers();
        self.fetch_card_covers(self.found.albums.iter(), self.found.artists.iter(), &self.found.playlists)
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
                // Скачанный трек берёт обложку с диска; закреплённое и аватарка — из сети, а без
                // неё с диска, куда сохраняются при каждой загрузке.
                let bytes = if offline::is_track(&id) {
                    offline::cover(&id).or_else(|| api::cover(&uri, size))
                } else {
                    let kept = id.starts_with(PIN_COVER_PREFIX) || id == AVATAR_COVER;
                    match api::cover(&uri, size) {
                        Some(bytes) => {
                            if kept {
                                offline::keep_cover(&id, &bytes);
                            }
                            Some(bytes)
                        }
                        None if kept => offline::cover(&id),
                        None => None,
                    }
                };
                if let Some(handle) = bytes.and_then(|bytes| decoded(&bytes)) {
                    if tx.unbounded_send((id, handle)).is_err() {
                        break;
                    }
                }
            });
        }
        Task::stream(rx).map(|(id, handle)| Message::Cover(id, handle))
    }

    /// Обложки остаются только у того, что на экране, рядом с ним и рядом в очереди.
    fn prune_covers(&mut self) {
        let range = self.visible();
        let from = range.start.saturating_sub(COVERS_AROUND);
        let around = self.list(self.section).iter().skip(from).take(range.end + COVERS_AROUND - from);
        let index = self.current.unwrap_or(0);
        let queue = self.queue.iter().skip(index.saturating_sub(QUEUE_COVERS_AROUND)).take(QUEUE_COVERS_AROUND * 2 + 1);
        let home = self.home.blocks.iter().flat_map(|block| block.tracks.iter().take(HOME_TRACKS));
        // Вкладки поиска, где уже были, возвращаются с обложками, а не с пустыми карточками.
        let searched = self.search_cache.values().flat_map(|found| found.tracks.iter().take(SEARCH_ALL_TRACKS * 2));
        let keep: HashSet<String> = around.chain(queue).chain(home).chain(searched).map(|t| t.id.clone()).collect();
        let mut cards: HashSet<String> = HashSet::new();
        let page = self.artist.as_ref();
        cards.extend(self.liked_albums.iter().chain(&self.found.albums).chain(page.into_iter().flat_map(|a| a.albums.iter().chain(&a.also))).map(|c| format!("{ALBUM_COVER}{}", c.id)));
        cards.extend(self.liked_artists.iter().chain(&self.found.artists).chain(page.into_iter().flat_map(|a| &a.similar)).map(|c| format!("{ARTIST_COVER}{}", c.id)));
        cards.extend(self.home.blocks.iter().enumerate().flat_map(|(index, block)| block.items[self.home_shown(index, block)].iter()).map(home_item_key));
        cards.extend(self.album.iter().map(|a| format!("{ALBUM_COVER}{}", a.id)));
        let album_cards = self.album.iter().flat_map(|a| &a.versions).chain(&self.album_others).chain(&self.discography);
        cards.extend(album_cards.map(|c| format!("{ALBUM_COVER}{}", c.id)));
        cards.extend(self.similar.iter().chain(&self.tag_lists).map(home_item_key));
        for found in self.search_cache.values() {
            cards.extend(found.albums.iter().map(|c| format!("{ALBUM_COVER}{}", c.id)));
            cards.extend(found.artists.iter().map(|c| format!("{ARTIST_COVER}{}", c.id)));
        }
        cards.extend(self.current_track().map(|t| format!("{WAVE_BIG}{}", t.id)));
        cards.extend(page.iter().map(|a| format!("{ARTIST_COVER}{}", a.id)));
        self.covers.retain(|id, _| id == AVATAR_COVER || id.starts_with(PLAYLIST_COVER) || id.starts_with(PIN_COVER_PREFIX) || cards.contains(id) || keep.contains(id));
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

    /// Отчёт о прослушивании уходит в фоне; ошибка на музыку не влияет.
    /// Без сети отчёты откладываются на диск и уходят, когда связь вернётся.
    fn send_plays(&self, reports: Vec<serde_json::Value>) {
        if self.offline {
            offline::keep_plays(reports);
            return;
        }
        let Some(token) = self.token() else { return };
        std::thread::spawn(move || {
            if let Err(e) = api::plays(reports.clone(), &token) {
                if api::network_error(&e) {
                    offline::keep_plays(reports);
                }
            }
        });
    }

    /// Добавляет время звучания с прошлого замера.
    fn listen_count(&mut self) {
        let now = Instant::now();
        let playing = self.active && self.player.as_ref().is_some_and(|p| !p.is_paused());
        if let (Some(listen), Some(last), true) = (self.listen.as_mut(), self.listen_tick, playing) {
            listen.played += now.duration_since(last).as_secs_f32();
        }
        self.listen_tick = Some(now);
    }

    /// Закрывает прослушивание прошлого трека с причиной смены.
    fn listen_finish(&mut self) {
        self.listen_count();
        let reason = self.change_reason.take().unwrap_or("other_track");
        let Some(listen) = self.listen.take().filter(|listen| listen.started.is_some()) else { return };
        let position = if reason == "finish" { listen.length } else { self.player.as_ref().map_or(self.position, |p| p.position()).as_secs_f32() };
        self.send_plays(vec![listen.report(position, Some(reason), false)]);
    }

    /// Удаление с отменой: уведомление с кнопкой «Отменить», по истечении — запрос на сервер.
    /// Прошлое отложенное удаление при этом выполняется сразу.
    fn pending_start(&mut self, pending: Pending, text: String) -> Task<Message> {
        let earlier = self.pending_commit();
        let id = self.notice.as_ref().map_or(1, |(id, _, _)| id + 1);
        self.notice = Some((id, text, true));
        self.pending = Some((id, pending));
        let expire = Task::perform(background(|| std::thread::sleep(UNDO_TIME)), move |()| Message::UndoExpire(id));
        Task::batch([earlier, expire])
    }

    /// Отправляет отложенное удаление на сервер.
    fn pending_commit(&mut self) -> Task<Message> {
        let (Some((_, pending)), Some(uid), Some(token)) = (self.pending.take(), self.uid.clone(), self.token()) else { return Task::none() };
        match pending {
            Pending::Playlist(list, _) => {
                let kind = list.kind;
                let wanted = kind.clone();
                Task::perform(background(move || api::delete_playlist(&uid, &kind, &token)), move |r| Message::PlaylistDeleted(wanted.clone(), r))
            }
            Pending::Track(list, track, _) => Task::perform(background(move || api::remove_from_playlist(&uid, &list.kind, &track, &token)), Message::TrackRemoved),
        }
    }

    /// Пункты меню «…» открытой страницы: значок, подпись, что сделать и опасный ли пункт.
    fn page_menu_items(&self) -> Vec<(&'static str, String, Message, bool)> {
        let link_and_pin = |target: PinRef, link: String| {
            let pinned = self.pins.iter().any(|pin| target.matches(pin));
            vec![(ui::PIN, if pinned { "Открепить" } else { "Закрепить" }.to_string(), Message::Pin(target), false), (ui::LINK, "Скопировать ссылку".to_string(), Message::CopyText(link), false)]
        };
        match self.section {
            Section::Artist => self.artist.as_ref().map_or_else(Vec::new, |artist| link_and_pin(PinRef::Artist(artist.id.clone()), format!("https://music.yandex.ru/artist/{}", artist.id))),
            Section::Playlists => {
                let Some(list) = &self.playlist else { return Vec::new() };
                if list.source == Source::Album {
                    let mut items = link_and_pin(PinRef::Album(list.kind.clone()), format!("https://music.yandex.ru/album/{}", list.kind));
                    if let Some(album) = self.album.as_ref().filter(|album| album.kind != AlbumKind::Music) {
                        let label = if album.finished { "Отметить непрослушанным" } else { "Отметить прослушанным" };
                        items.push((ui::CHECK, label.to_string(), Message::AlbumFinished(!album.finished), false));
                    }
                    items.extend(self.download_item());
                    return items;
                }
                let mut items = link_and_pin(PinRef::Playlist(list.owner.clone(), list.kind.clone()), format!("https://music.yandex.ru/users/{}/playlists/{}", list.owner, list.kind));
                items.extend(self.download_item());
                if self.is_own(list) {
                    items.push((ui::EDIT, "Переименовать".to_string(), Message::DialogOpen(Dialog::Rename(list.clone())), false));
                    items.push((ui::NOTES, "Изменить описание".to_string(), Message::DialogOpen(Dialog::Describe(list.clone())), false));
                    items.push((ui::IMAGE, if list.cover.is_some() { "Изменить обложку" } else { "Добавить обложку" }.to_string(), Message::CoverPick(list.clone()), false));
                    if let Some(meta) = &self.list_meta {
                        let private = meta.2 == "private";
                        items.push((if private { ui::GLOBE } else { ui::LOCK }, if private { "Сделать публичным" } else { "Сделать приватным" }.to_string(), Message::Visibility(list.clone()), false));
                    }
                    items.push((ui::DELETE, "Удалить плейлист".to_string(), Message::DialogOpen(Dialog::Delete(list.clone())), true));
                }
                items
            }
            _ => Vec::new(),
        }
    }

    /// Кнопка «…» в шапке страницы.
    fn page_more<'a>(&self) -> Element<'a, Message> {
        shine(button(container(ui::icon(ui::MORE, 22.0, ui::TEXT)).center(22)).padding(9).style(ui::secondary).on_press(Message::PageMenuOpen), 24.0).into()
    }

    fn view_page_menu(&self, x: f32, y: f32) -> Element<'_, Message> {
        let (alpha, live) = (self.page_menu_fade.shown(), self.page_menu_fade.live);
        let items = self.page_menu_items();
        let count = items.len();
        let list = items.into_iter().fold(column![], |list, (icon, label, message, danger)| {
            let tint = if danger { ui::DANGER } else { ui::TEXT };
            let mark = if danger { ui::DANGER } else { ui::MUTED };
            list.push(glow(
                button(row![ui::icon(icon, 20.0, mark.scale_alpha(alpha)), text(label).color(tint.scale_alpha(alpha))].spacing(12).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .height(MENU_ITEM)
                    .padding([9, 10])
                    .style(ui::menu_item)
                    .on_press(Message::PageMenuPick(Box::new(message))),
                6.0,
            ))
        });
        popup(list.into(), x, y, MENU_WIDTH, count as f32 * MENU_ITEM, alpha, live)
    }

    /// Скачивает следующий трек из очереди скачивания. Качается по одному, чтобы не мешать звуку.
    fn download_next(&mut self) -> Task<Message> {
        if self.download_now.is_some() {
            return Task::none();
        }
        let (Some(token), Some(track)) = (self.token(), self.download_queue.pop_front()) else { return Task::none() };
        self.download_now = Some(track.id.clone());
        let (quality, id) = (self.quality.code(), track.id.clone());
        Task::perform(background(move || offline::download(&track, quality, &token)), move |result| Message::Downloaded(id.clone(), result))
    }

    /// Список скачанного поменялся: обновить номера, список для страницы и файл со списком.
    fn downloads_changed(&mut self) {
        self.download_ids = self.downloads.iter().map(|saved| saved.track.id.clone()).collect();
        self.download_tracks = self.downloads.iter().map(|saved| saved.track.clone()).collect();
        offline::save(&self.downloads);
    }

    /// Пункт меню страницы: скачать её треки или убрать их, если скачаны все.
    fn download_item(&self) -> Option<(&'static str, String, Message, bool)> {
        let tracks: Vec<&Track> = self.playlist_tracks.iter().filter(|track| !track.unavailable).collect();
        if tracks.is_empty() {
            return None;
        }
        if tracks.iter().all(|track| self.download_ids.contains(&track.id)) {
            let ids = tracks.iter().map(|track| track.id.clone()).collect();
            return Some((ui::DELETE, "Удалить из скачанного".to_string(), Message::DownloadRemove(ids), false));
        }
        Some((ui::DOWNLOAD, "Скачать".to_string(), Message::Download(tracks.into_iter().cloned().collect()), false))
    }

    /// У скачанного трека перед исполнителем значок: он играет и без интернета.
    fn download_mark<'a>(&self, track: &Track, names: Element<'a, Message>) -> Element<'a, Message> {
        if !self.download_ids.contains(&track.id) {
            return names;
        }
        row![ui::icon(ui::DOWNLOAD_DONE, 14.0, ui::ACCENT), names].spacing(4).align_y(Alignment::Center).into()
    }

    /// Переставляет трек: в очереди только в окне, в своём плейлисте ещё и на сервере.
    fn move_track(&mut self, list: DragList, from: usize, to: usize) -> Task<Message> {
        match list {
            DragList::Queue => {
                // Играющий трек и всё, что до него, не двигаются.
                let floor = self.current.map_or(0, |current| current + 1);
                if from < floor || to < floor || from >= self.queue.len() || to >= self.queue.len() {
                    return Task::none();
                }
                let track = self.queue.remove(from);
                self.queue.insert(to, track);
                self.save_session();
                Task::none()
            }
            DragList::Playlist => {
                let (Some(uid), Some(token), Some(playlist)) = (self.uid.clone(), self.token(), self.playlist.clone()) else { return Task::none() };
                if !self.is_own(&playlist) || from >= self.playlist_tracks.len() || to >= self.playlist_tracks.len() {
                    return Task::none();
                }
                let track = self.playlist_tracks.remove(from);
                self.playlist_tracks.insert(to, track.clone());
                Task::perform(background(move || api::move_in_playlist(&uid, &playlist.kind, &track, from, to, &token)), Message::TrackMoved)
            }
        }
    }

    /// За эту ручку строку перетаскивают.
    fn grip<'a>(&self, list: DragList, index: usize) -> Element<'a, Message> {
        mouse_area(container(ui::icon(ui::GRIP, 18.0, ui::MUTED)).padding(6)).on_press(Message::DragStart(list, index)).interaction(mouse::Interaction::Grab).into()
    }

    /// Строка во время перетаскивания: ловит курсор, взятая строка бледнеет, место, куда трек
    /// встанет, отмечено линией.
    fn drag_target<'a>(&self, row: Element<'a, Message>, list: DragList, index: usize) -> Element<'a, Message> {
        let Some((_, from, over)) = self.drag.filter(|(dragged, _, _)| *dragged == list) else { return row };
        let row: Element<'a, Message> = if index == from { reveal(row).veil(ui::CONTENT.scale_alpha(0.6)).into() } else { row };
        let mut layers = stack![mouse_area(row).on_move(move |point| Message::DragOver(index, point.y))];
        if let Some((_, below)) = over.filter(|(at, below)| *at == index && drop_index(from, *at, *below).is_some()) {
            let line = container(Space::new()).width(Length::Fill).height(2).style(|_| container::Style::default().background(ui::ACCENT));
            layers = layers.push(container(line).height(ROW).align_y(if below { Alignment::End } else { Alignment::Start }));
        }
        layers.into()
    }

    /// Отдаёт Яндексу место в играющем выпуске или книге и запоминает его до конца запуска.
    fn progress_save(&mut self, finished: bool) {
        let (Some(track), Some(token)) = (self.current_track().filter(|track| track.remember).cloned(), self.token()) else { return };
        if !self.active && !finished {
            return;
        }
        let length = track.duration.as_secs_f32();
        let place = if finished { length } else { self.player.as_ref().map_or(self.position, Player::position).as_secs_f32() };
        self.progress.insert(track.id.clone(), place);
        self.progress_at = Some(Instant::now());
        std::thread::spawn(move || {
            if let Err(e) = api::save_progress(&track.id, place, length, finished, &token) {
                eprintln!("место в выпуске не сохранилось: {e}");
            }
        });
    }

    /// Откуда играет трек: контекст, его номер и страница, как их называет официальный клиент.
    fn listen_new(&self, track: &Track) -> api::Listen {
        let screen = |page: &str| format!("desktop-{page}_screen-default");
        if self.wave_queue {
            let mut listen = api::Listen::new(track, screen("wave_landing"), "radio", Some(self.wave_base.join(",")).filter(|seeds| !seeds.is_empty()));
            listen.radio = self.wave.clone().map(|session| (session, track.batch.clone()));
            return listen;
        }
        let (page, context, item, playlist) = match &self.queue_from {
            QueueFrom::List(list) if list.source == Source::Album => ("album", "album", Some(list.kind.clone()), None),
            QueueFrom::List(list) if !list.owner.is_empty() => {
                let id = format!("{}:{}", list.owner, list.kind);
                ("playlist", "playlist", Some(id.clone()), Some(id))
            }
            QueueFrom::Artist(id) => ("artist", "artist", Some(id.clone()), None),
            QueueFrom::Search => ("search", "search", None, None),
            QueueFrom::Likes | QueueFrom::Dislikes => ("playlist", "other", None, None),
            QueueFrom::Downloads => ("downloads", "other", None, None),
            QueueFrom::History => ("history", "other", None, None),
            QueueFrom::List(_) | QueueFrom::Unknown => ("main", "other", None, None),
        };
        let mut listen = api::Listen::new(track, screen(page), context, item);
        listen.playlist = playlist;
        listen
    }

    /// Цвет обложки играющего трека: по крупной, а пока её нет — по уже загруженной мелкой.
    fn full_target(&self) -> iced::Color {
        let Some(track) = self.current_track() else { return ui::TILE };
        self.cover_tints.get(&track.id).copied().or_else(|| self.covers.get(&track.id).and_then(average_color)).unwrap_or(ui::TILE)
    }

    fn current_track(&self) -> Option<&Track> {
        self.queue.get(self.current?)
    }

    /// Возвращает очередь прошлого запуска. Музыка не включается сама.
    fn restore(&mut self, saved: session::Saved) -> Task<Message> {
        let ids: Vec<String> = saved.tracks.iter().map(|track| track.id.clone()).collect();
        let check = match self.token() {
            Some(token) => Task::perform(background(move || api::availability(&ids, &token)), Message::Availability),
            None => Task::none(),
        };
        Task::batch([check, self.restore_queue(saved)])
    }

    fn restore_queue(&mut self, saved: session::Saved) -> Task<Message> {
        self.session_key = Some(queue_key(&saved.tracks, &saved.title));
        self.queue = saved.tracks;
        self.queue_from = QueueFrom::Unknown;
        self.current = Some(saved.current);
        self.position = saved.position;
        self.resume = Some(saved.position);
        let Some(seeds) = saved.wave else {
            self.queue_title = Some(saved.title);
            self.wave_queue = false;
            return Task::none();
        };
        self.queue_title = None;
        self.wave_title = Some(saved.title).filter(|title| !title.is_empty());
        self.wave_queue = true;
        self.wave_base = seeds.clone();
        self.wave_wanted = seeds.clone();
        // Старая сессия на сервере могла закончиться, а новая нужна, чтобы волна шла дальше.
        let Some(token) = self.token() else { return Task::none() };
        let mut asked = seeds.clone();
        asked.extend(settings::wave_seeds(self.wave_tuning));
        Task::perform(background(move || api::wave_new(&asked, &token)), move |result| Message::WaveResumed(seeds.clone(), result))
    }

    /// Запоминает очередь и место, волну вместе с её зёрнами.
    fn save_session(&mut self) {
        self.session_store(false);
    }

    fn session_store(&mut self, wait: bool) {
        self.session_at = Some(Instant::now());
        let Some(current) = self.current else { return };
        let title = if self.wave_queue { self.wave_title.clone() } else { self.queue_title.clone() }.unwrap_or_default();
        let key = queue_key(&self.queue, &title);
        if self.session_key != Some(key) {
            self.session_key = Some(key);
            session::save_queue(self.queue.clone(), title, self.wave_queue.then(|| self.wave_base.clone()), wait);
        }
        session::save_place(current, self.position, wait);
    }

    fn play(&mut self, index: usize) -> Task<Message> {
        // На недоступный трек попадают перемешиванием или сменой очереди: играет следующий доступный.
        if self.queue.get(index).is_some_and(|track| track.unavailable) {
            return match (index + 1..self.queue.len()).find(|&i| !self.queue[i].unavailable) {
                Some(next) => self.play(next),
                None => self.notify(UNAVAILABLE.to_string()),
            };
        }
        // Без сети звучит только скачанное: остальное в очереди пропускается.
        if self.offline && self.queue.get(index).is_some_and(|track| !self.download_ids.contains(&track.id)) {
            let saved = (index + 1..self.queue.len()).chain(0..index).find(|&i| self.download_ids.contains(&self.queue[i].id));
            return match saved {
                Some(next) => {
                    let note = self.notify("Без интернета играет только скачанное".to_string());
                    Task::batch([note, self.play(next)])
                }
                None => self.notify("Нет подключения к интернету, а в очереди нет скачанных треков".to_string()),
            };
        }
        let (Some(player), Some(token), Some(track)) = (self.player.clone(), self.token(), self.queue.get(index).cloned()) else {
            return Task::none();
        };
        if let Some(session) = self.wave_pending.take() {
            self.wave = Some(session);
            self.feedback("radioStarted", None, None);
        }
        let restart = self.restart.take();
        // Место прошлого запуска относится только к тому же треку.
        let mut resume = self.resume.take().filter(|_| self.current == Some(index));
        // Выпуск подкаста и глава книги продолжаются там, где остановились, если не дослушаны.
        if resume.is_none() && restart.is_none() && track.remember {
            let place = self.progress.get(&track.id).copied().or(track.progress).unwrap_or(0.0);
            let end = track.duration.as_secs_f32();
            if place > PROGRESS_MIN && place < end - PROGRESS_TAIL {
                resume = Some(Duration::from_secs_f32(place));
            }
        }
        // Прошлый трек закрывается, пока плеер ещё знает его место.
        if restart.is_none() {
            self.progress_save(self.change_reason == Some("finish"));
            let again = self.change_reason == Some("finish") && self.listen.as_ref().is_some_and(|listen| listen.track == track.id);
            self.listen_finish();
            let mut listen = self.listen_new(&track);
            listen.restored = resume.is_some();
            listen.repeated = again;
            self.listen = Some(listen);
            self.listen_next = LISTEN_REPORT;
        }
        self.retry = None;
        self.sound = None;
        // Переход: играющий трек затихает сам, новый нарастает поверх него.
        let generation = match (self.cross_out.take(), track.fade) {
            (Some(out), Some(fade)) if restart.is_none() && resume.is_none() => player.crossfade(out, (fade.in_start, fade.in_stop)),
            _ => player.stop(),
        };
        self.play_id = generation;
        self.current = Some(index);
        self.active = false;
        self.starting = true;
        self.position = resume.unwrap_or(Duration::ZERO);
        self.resume_seek = resume.filter(|place| !place.is_zero());
        self.seeking = None;
        self.error = None;
        // Трек с прошлого места стартует на паузе, иначе на миг слышно его начало.
        player.hold_next(restart == Some(true) || self.resume_seek.is_some());
        self.unhold = self.resume_seek.is_some() && restart != Some(true);
        if restart.is_none() {
            self.feedback("trackStarted", Some(&track), None);
        }
        let id = track.id.clone();
        let quality = self.quality;
        let local = self.downloads.iter().find(|saved| saved.track.id == track.id).map(|saved| (saved.path(), saved.codec.clone(), saved.sound()));
        let start = Task::perform(
            background(move || {
                if let Some((path, codec, sound)) = local.filter(|(path, _, _)| path.exists()) {
                    player.play_file(generation, &path, &codec)?;
                    return Ok(sound);
                }
                if let Some(started) = player.play_prepared(generation, &preload_key(&id, quality)) {
                    return started;
                }
                let info = api::file_info(&id, quality.code(), &token)?;
                player.play(generation, &info)?;
                Ok(info.sound())
            }),
            move |result| Message::Started(generation, result),
        );
        Task::batch([start, self.wave_refill()])
    }

    /// Пора начинать переход к следующему треку: оба трека — музыка с точками перехода,
    /// трек играет и дошёл до начала затухания. Повтор одного трека переходом не идёт.
    fn cross_next(&self) -> Option<usize> {
        if !self.crossfade || (self.repeat == Repeat::One && !self.wave_queue) || self.starting || self.seeking.is_some() || self.resume_seek.is_some() {
            return None;
        }
        if self.player.as_ref().is_none_or(Player::is_paused) {
            return None;
        }
        let fade = self.current_track()?.fade?;
        let position = self.position.as_secs_f32();
        if position < fade.out_start || position >= fade.out_stop {
            return None;
        }
        let next = self.next_index().filter(|&next| Some(next) != self.current)?;
        self.queue.get(next)?.fade.map(|_| next)
    }

    /// Незадолго до конца трека начинает качать следующий. Ошибка не важна: при запуске
    /// файл просто запросится обычным путём.
    fn preload_next(&mut self) {
        let (Some(track), Some(player), Some(token)) = (self.current_track(), self.player.clone(), self.token()) else { return };
        if track.duration.saturating_sub(self.position) > PRELOAD_BEFORE || (self.repeat == Repeat::One && !self.wave_queue) {
            return;
        }
        let Some(next) = self.next_index().and_then(|i| self.queue.get(i)) else { return };
        // Скачанный следующий трек и так на диске.
        if self.download_ids.contains(&next.id) {
            return;
        }
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
        let (list_uid, list_token) = (uid.clone(), token.clone());
        let lists = Task::perform(background(move || api::liked_playlists(&list_uid, &list_token)), Message::LikedLists);
        Task::batch([lists, Task::perform(background(move || api::liked_pages(&uid, &token)), Message::LikedPages)])
    }

    /// Включает новый список вместо очереди.
    fn play_list(&mut self, tracks: Vec<Track>, title: String, index: usize) -> Task<Message> {
        self.queue = tracks;
        self.wave = None;
        self.wave_pending = None;
        self.wave_queue = false;
        self.queue_title = Some(title);
        self.unshuffled = None;
        if self.shuffle && index < self.queue.len() {
            self.unshuffled = Some(self.queue.clone());
            shuffled(&mut self.queue, index, random_seed());
            return self.play(0);
        }
        self.play(index)
    }

    fn home_load(&mut self, page: Section, tab: usize) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.home_page = page;
        self.home_tab = tab;
        self.landing_tabs[landing_index(page)] = tab;
        self.home_loading = true;
        self.home_pages.clear();
        self.home_block_tabs.clear();
        self.home.blocks.clear();
        self.error = None;
        let skeleton = landing_skeleton(page);
        Task::perform(background(move || api::landing(skeleton, tab, &token)), move |r| Message::HomeLoaded(page, tab, r))
    }

    fn here(&self) -> Place {
        Place { section: self.section, playlist: self.playlist.clone(), artist: self.artist.clone() }
    }

    fn remember(&mut self) {
        self.history.push(self.here());
        self.forward.clear();
    }

    fn discography_load(&mut self, page: u64) -> Task<Message> {
        let (Some(artist), Some(token)) = (self.artist.as_ref(), self.token()) else { return Task::none() };
        if page == 0 {
            self.discography.clear();
            self.discography_total = 0;
        }
        let (id, also, by_year) = (artist.id.clone(), self.discography_also, self.discography_by_year);
        let key = format!("{id}:{also}:{by_year}:{page}");
        self.discography_key = Some(key.clone());
        Task::perform(background(move || api::releases(&id, also, by_year, page, &token)), move |result| Message::DiscographyLoaded(key.clone(), result))
    }

    fn go(&mut self, place: Place) -> Task<Message> {
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
        if place.section == Section::Discography {
            return self.discography_load(0);
        }
        Task::none()
    }

    fn open_list(&mut self, playlist: Playlist) -> Task<Message> {
        self.remember();
        self.load_list(playlist)
    }

    fn similar_load(&self, list: &Playlist) -> Task<Message> {
        // У своих плейлистов похожего не бывает.
        let (Some(token), false) = (self.token(), self.is_own(list) || list.kind == FAVORITES_KIND) else { return Task::none() };
        let (list, kind) = (list.clone(), list.kind.clone());
        Task::perform(background(move || api::similar(&list, &token)), move |r| Message::SimilarLoaded(kind.clone(), r))
    }

    fn load_list(&mut self, playlist: Playlist) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.section = Section::Playlists;
        self.error = None;
        self.playlist = Some(playlist.clone());
        self.playlist_tracks.clear();
        self.album = None;
        self.album_others.clear();
        self.similar.clear();
        let similar = self.similar_load(&playlist);
        let kind = playlist.kind.clone();
        if playlist.source == Source::Album {
            return Task::batch([similar, Task::perform(background(move || api::album(&playlist.kind, &token)), move |r| Message::AlbumLoaded(kind.clone(), r))]);
        }
        self.list_meta = None;
        let meta = if playlist.source == Source::Playlist {
            let (owner, list_kind, meta_token) = (playlist.owner.clone(), kind.clone(), token.clone());
            let wanted = kind.clone();
            Task::perform(background(move || api::playlist_meta(&owner, &list_kind, &meta_token)), move |r| Message::ListMeta(wanted.clone(), r))
        } else {
            Task::none()
        };
        Task::batch([similar, meta, Task::perform(background(move || api::playlist_tracks(&playlist, &token)), move |r| Message::PlaylistLoaded(kind.clone(), r))])
    }

    fn wave_load(&mut self, seeds: Vec<String>, title: Option<String>, autoplay: bool) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.wave_loading = true;
        self.error = None;
        self.wave_wanted = seeds.clone();
        let mut asked = seeds.clone();
        asked.extend(settings::wave_seeds(self.wave_tuning));
        Task::perform(background(move || api::wave_new(&asked, &token)), move |r| Message::WaveStarted(autoplay, seeds.clone(), title.clone(), r))
    }

    /// Показывает уведомление и убирает его через несколько секунд.
    fn notify(&mut self, text: String) -> Task<Message> {
        let id = self.notice.as_ref().map_or(1, |(id, _, _)| id + 1);
        self.notice = Some((id, text, false));
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

    fn filtering(&self) -> bool {
        !self.filter.trim().is_empty()
    }

    fn filter_reset(&mut self) {
        self.filter.clear();
        self.filter_tracks.clear();
        self.filter_loading = false;
        self.filter_id += 1;
    }

    /// Где искать: в лайках или в открытом плейлисте.
    fn filter_place(&self) -> Option<(String, String)> {
        match self.section {
            Section::Likes => Some((self.uid.clone()?, FAVORITES_KIND.to_string())),
            Section::Playlists => self.playlist.as_ref().filter(|p| p.source == Source::Playlist).map(|p| (p.owner.clone(), p.kind.clone())),
            _ => None,
        }
    }

    fn list(&self, section: Section) -> &[Track] {
        match section {
            Section::Likes | Section::Playlists if self.filtering() => &self.filter_tracks,
            Section::Wave => &self.queue,
            Section::Likes | Section::Collection => &self.liked,
            Section::Playlists => &self.playlist_tracks,
            Section::Artist => self.artist.as_ref().map_or(&[], |a| a.popular.as_slice()),
            Section::Albums | Section::Artists | Section::Home | Section::Books | Section::Kids | Section::Discography | Section::Tag => &[],
            // На странице очереди список — то, что будет после играющего трека.
            Section::History => &self.history_tracks,
            Section::Dislikes => &self.disliked_list,
            Section::Downloads => &self.download_tracks,
            Section::Queue => self.current.and_then(|i| self.queue.get(i + 1..)).unwrap_or(&[]),
            Section::Lyrics | Section::Settings => &[],
            Section::Search => &self.results,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // Запрос запоминается, когда по нему что-то открыли или нажали Enter, а не на каждой букве.
        let chosen = matches!(message, Message::Search | Message::Play(Section::Search, _) | Message::ArtistOpen(_) | Message::AlbumOpen(..) | Message::ListOpen(_));
        if chosen && self.section == Section::Search && self.searched && api::parse_link(&self.query).is_none() {
            let list = session::remember_search(&self.searches, &self.query);
            if list != self.searches {
                self.searches = list.clone();
                session::save_searches(list);
            }
        }
        if let Message::Quit = message {
            // Удаление, которое ещё можно было отменить, при выходе всё же выполняется.
            if let (Some((_, pending)), Some(uid), Some(token)) = (self.pending.take(), self.uid.clone(), self.token()) {
                let _ = match pending {
                    Pending::Playlist(list, _) => api::delete_playlist(&uid, &list.kind, &token),
                    Pending::Track(list, track, _) => api::remove_from_playlist(&uid, &list.kind, &track, &token),
                };
            }
            self.session_store(true);
            return iced::exit();
        }
        if let Message::Frame(now) = message {
            if self.page_veil.max(self.area_veil) > 0.0 && std::env::var_os("TUNE_FRAMES").is_some() {
                eprintln!("кадр {:.1} мс, переход {:.2}, вкладка {:.2}", self.last_frame.map_or(0.0, |last| now.duration_since(last).as_secs_f32() * 1000.0), self.page_veil, self.area_veil);
            }
            let elapsed = self.last_frame.map_or(0.0, |last| now.duration_since(last).as_secs_f32()).min(FRAME_LIMIT);
            self.page_veil = (self.page_veil - elapsed / PAGE_FADE).max(0.0);
            self.area_veil = (self.area_veil - elapsed / AREA_FADE).max(0.0);
            self.track_veil = (self.track_veil - elapsed / TRACK_FADE).max(0.0);
            let bar_top = self.bar_top_target();
            self.bar_top = if bar_top > self.bar_top { (self.bar_top + elapsed / BAR_TOP_FADE).min(bar_top) } else { (self.bar_top - elapsed / BAR_TOP_FADE).max(bar_top) };
            self.cover_fades.retain(|_, level| {
                *level += elapsed / COVER_FADE;
                *level < 1.0
            });
            self.menu_fade.step(elapsed);
            self.pin_fade.step(elapsed);
            self.quality_fade.step(elapsed);
            self.eq_fade.step(elapsed);
            self.page_menu_fade.step(elapsed);
            self.dialog_fade.step(elapsed);
            self.full_fade.step(elapsed);
            if self.full_fade.item.is_some() {
                let target = self.full_target();
                let part = 1.0 - (-elapsed * FULL_TINT_SPEED).exp();
                let mix = |from: f32, to: f32| from + (to - from) * part;
                self.full_tint = if same_color(self.full_tint, target) { target } else { iced::Color::from_rgb(mix(self.full_tint.r, target.r), mix(self.full_tint.g, target.g), mix(self.full_tint.b, target.b)) };
            }
            self.notice_fade.step(elapsed);
            let lyrics = if self.section == Section::Lyrics && (self.lyrics_shown != self.lyrics_target || self.lyrics_back < 1.0 || self.page_veil > 0.0) {
                self.lyrics_shown += (self.lyrics_target - self.lyrics_shown) * (elapsed * LYRICS_SPEED).min(1.0);
                if (self.lyrics_target - self.lyrics_shown).abs() < 0.002 {
                    self.lyrics_shown = self.lyrics_target;
                }
                self.lyrics_back = (self.lyrics_back + elapsed / LYRICS_RETURN).min(1.0);
                let y = (self.lyrics_place() + self.lyrics_gap * (1.0 - smooth_step(self.lyrics_back))).clamp(0.0, 1.0);
                operation::snap_to(Id::new(LIST), scrollable::RelativeOffset { x: 0.0, y })
            } else {
                Task::none()
            };
            let queue = self.queue_scroll_step(elapsed);
            self.last_frame = Some(now).filter(|_| self.animating());
            return Task::batch([lyrics, queue, self.visible_covers()]);
        }
        let place = self.list_place();
        let offset = self.list_offset;
        let back = matches!(message, Message::Back);
        let travel = matches!(message, Message::Back | Message::Forward);
        let rank = self.section as u8;
        let tab = (self.search_tab, self.home_tab);
        let ready = self.page_ready();
        let loading = self.area_loading();
        let task = self.handle(message);
        // Другая страница открывается там, где её оставили; новая — с начала.
        let mut snap = if place == self.list_place() {
            Task::none()
        } else {
            if remembers_scroll(place.0) {
                self.scroll_places.insert(place.clone(), offset);
            }
            // Открыли исполнителя, альбом или другую страницу из плеера на всё окно — он уезжает.
            self.player_full = false;
            let here = self.list_place();
            let section_root = matches!(here.0, Section::Home | Section::Books | Section::Kids | Section::Likes | Section::Collection | Section::Albums | Section::Artists | Section::History | Section::Dislikes | Section::Downloads | Section::Search | Section::Settings)
                || (here.0 == Section::Playlists && here.1.is_none());
            let restore = self.scroll_places.get(&here).copied().filter(|_| remembers_scroll(here.0) && (section_root || travel));
            self.scroll_wait = restore.filter(|_| !self.page_ready());
            self.list_offset = restore.unwrap_or(0.0);
            self.queue_scroll = None;
            self.filter_reset();
            self.page_veil = 1.0;
            self.lyrics_shown = -1.0;
            self.lyrics_target = -1.0;
            self.lyrics_back = 1.0;
            self.lyrics_line = None;
            self.lyrics_fresh = true;
            let down = back || (self.section as u8) < rank;
            self.page_shift = (0.0, if down { -PAGE_SHIFT } else { PAGE_SHIFT });
            match restore {
                Some(y) => operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y }),
                None => operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START),
            }
        };
        // Страница дождалась данных: прокрутка встаёт на прежнее место.
        if place == self.list_place() && self.page_ready() {
            if let Some(y) = self.scroll_wait.take() {
                self.list_offset = y;
                snap = operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y });
            }
        }
        if place == self.list_place() {
            let tabbed = matches!(self.section, Section::Search | Section::Home | Section::Books | Section::Kids);
            if tabbed && (self.search_tab, self.home_tab) != tab {
                // Другая вкладка проявляется на месте, без сдвига: шапка и кнопки не трогаются.
                self.area_veil = 1.0;
            } else if tabbed {
                // Результаты пришли: они проявляются, если переход вкладки уже закончился.
                if loading && !self.area_loading() && self.area_veil == 0.0 {
                    self.area_veil = 0.85;
                }
            } else if !ready && self.page_ready() && self.page_veil == 0.0 {
                // Страница дождалась данных: они проявляются, а не выскакивают. Если переход
                // ещё идёт, второй не начинается, иначе страница дёрнется дважды.
                self.page_veil = 0.85;
                self.page_shift = (0.0, PAGE_SHIFT / 2.0);
            }
        }
        self.sync_overlays();
        let lyrics = self.lyrics_sync();
        self.lyrics_follow();
        // Лайки догружаются, когда до конца загруженного остаётся немного.
        let more = if self.section == Section::Likes && !self.filtering() && self.visible().end + ROW_MARGIN >= self.liked.len() && !matches!(self.retry, Some(Message::LikesMore)) {
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
        Task::batch([task, snap, more, covers, lyrics])
    }

    /// Открытые меню и окна: каждое появляется само и дорисовывает исчезновение после закрытия.
    fn sync_overlays(&mut self) {
        self.menu_fade.sync(self.menu.as_ref().map(|(track, x, y)| (track.clone(), *x, *y, self.menu_artists, self.menu_lists)));
        self.quality_fade.sync(self.quality_menu);
        self.eq_fade.sync(self.eq_menu);
        self.pin_fade.sync(self.pin_menu);
        self.dialog_fade.sync(self.dialog.clone());
        self.full_fade.sync(self.player_full.then_some(()));
        self.notice_fade.sync(self.notice.as_ref().map(|(_, text, undo)| (text.clone(), *undo)));
        self.page_menu_fade.sync(self.page_menu);
        let track = self.current_track().map(|track| track.id.clone());
        if track != self.shown_track {
            if self.player_full && self.shown_track.is_some() {
                self.track_veil = 1.0;
            }
            self.shown_track = track;
        }
    }

    /// Есть ли что показать на странице, или она ещё ждёт данных.
    /// Грузится содержимое под вкладками поиска или главной.
    fn area_loading(&self) -> bool {
        match self.section {
            Section::Search => self.searching,
            Section::Home | Section::Books | Section::Kids => self.home_loading,
            _ => false,
        }
    }

    /// Пелена над областью под вкладками.
    fn area<'a>(&self, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        let rest = 1.0 - anim::ease_out(1.0 - self.area_veil);
        reveal(content).veil(ui::CONTENT.scale_alpha(rest)).into()
    }

    fn page_ready(&self) -> bool {
        match self.section {
            Section::Likes | Section::Playlists if self.filtering() => true,
            Section::Home | Section::Books | Section::Kids => self.home_page == self.section && !self.home.blocks.is_empty(),
            Section::Wave => self.current_track().is_some(),
            Section::Collection | Section::Settings => true,
            Section::Lyrics => self.lyrics.as_ref().is_some_and(|(id, _)| self.current_track().is_some_and(|track| track.id == *id)),
            Section::Albums => !self.liked_albums.is_empty(),
            Section::Artists => !self.liked_artists.is_empty(),
            Section::Artist => self.artist.is_some(),
            Section::Discography => self.discography_key.is_none(),
            Section::Tag => !self.tag_loading,
            Section::Playlists if self.playlist.is_none() => !self.playlists.is_empty(),
            section => !self.list(section).is_empty(),
        }
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
                offline::save_account(&profile, &ids, &disliked);
                if let Some(token) = self.token().filter(|_| !self.offline) {
                    offline::flush_plays(token);
                }
                self.disliked = disliked.into_iter().collect();
                self.uid = Some(profile.uid.clone());
                self.profile = Some(profile);
                self.liked_set = ids.iter().cloned().collect();
                self.liked_ids = ids;
                let pins = self.token().map_or(Task::none(), |token| Task::perform(background(move || api::pins(&token)), Message::PinsLoaded));
                let avatar = self.profile.as_ref().and_then(|p| p.avatar.clone()).map_or(Task::none(), |uri| self.fetch_cover_jobs(vec![(AVATAR_COVER.to_string(), uri)], "islands-200"));
                let pins = Task::batch([pins, avatar, self.liked_pages_load()]);
                let page = match self.preview.clone() {
                    Some(Preview::Wave | Preview::WaveSeek) => self.wave_load(vec![api::MY_WAVE.to_string()], None, false),
                    Some(Preview::Lyrics) => self.wave_load(vec!["artist:3121".to_string()], None, false),
                    Some(Preview::WaveTune) => {
                        self.preview = None;
                        self.section = Section::Wave;
                        let wave = self.wave_load(vec![api::MY_WAVE.to_string()], None, false);
                        Task::batch([wave, self.update(Message::DialogOpen(Dialog::WaveTune))])
                    }
                    Some(Preview::WaveSeeded) => self.wave_load(vec!["artist:3121".to_string()], Some(wave_label("artist:3121", "Metallica")), false),
                    Some(Preview::Likes | Preview::LikesEnd | Preview::Menu | Preview::Panels | Preview::TrackInfo | Preview::QueueJump) => self.update(Message::Open(Section::Likes)),
                    Some(Preview::Collection) => self.update(Message::Open(Section::Collection)),
                    Some(Preview::Playlists | Preview::Playlist | Preview::PlaylistHome | Preview::PageMenu) => self.update(Message::Open(Section::Playlists)),
                    Some(Preview::Dislikes) => {
                        self.preview = None;
                        self.update(Message::Open(Section::Dislikes))
                    }
                    Some(Preview::History) => {
                        self.preview = None;
                        self.update(Message::Open(Section::History))
                    }
                    Some(Preview::Dialog) => {
                        self.preview = None;
                        let page = self.update(Message::Open(Section::Playlists));
                        Task::batch([page, self.update(Message::DialogOpen(Dialog::Create))])
                    }
                    Some(Preview::Home) => {
                        self.preview = None;
                        self.update(Message::Open(Section::Home))
                    }
                    Some(Preview::FilterFocus) => {
                        self.preview = None;
                        let open = self.update(Message::Open(Section::Likes));
                        Task::batch([open, Task::perform(background(|| std::thread::sleep(Duration::from_millis(2500))), |()| Message::PreviewFilter(0))])
                    }
                    Some(Preview::ScrollReturn) => {
                        self.preview = None;
                        let open = self.update(Message::Open(Section::Likes));
                        Task::batch([open, Task::perform(background(|| std::thread::sleep(Duration::from_millis(2500))), |()| Message::PreviewScroll(0))])
                    }
                    Some(Preview::SearchFocus) => {
                        self.preview = None;
                        let open = self.update(Message::Open(Section::Search));
                        Task::batch([open, Task::perform(background(|| std::thread::sleep(Duration::from_millis(800))), |()| Message::PreviewType(0))])
                    }
                    Some(Preview::SearchTabs(_)) => {
                        self.section = Section::Search;
                        self.query = "Queen".to_string();
                        self.update(Message::Search)
                    }
                    Some(Preview::SearchHistory) => {
                        self.preview = None;
                        self.searches = ["Queen", "Земфира", "Linkin Park Numb", "лоу-фай"].map(String::from).to_vec();
                        self.update(Message::Open(Section::Search))
                    }
                    Some(Preview::Tag(tag)) => {
                        self.preview = None;
                        let title = tag.clone();
                        self.update(Message::HomeOpen(HomeItem::Mix { tag, title, cover: None }))
                    }
                    Some(Preview::HomeTrends) => {
                        self.preview = None;
                        self.section = Section::Home;
                        self.update(Message::HomeTab(1))
                    }
                    Some(Preview::SimilarOf(list)) => {
                        self.preview = None;
                        self.preview_end = true;
                        self.open_list(list)
                    }
                    Some(Preview::Album(id)) => {
                        self.preview = None;
                        self.open_list(Playlist::album(&id, ""))
                    }
                    Some(Preview::Artist(id)) => {
                        self.preview = None;
                        self.update(Message::ArtistOpen(id))
                    }
                    // Подсказка снимается, когда исполнитель загрузится.
                    Some(Preview::ArtistAbout(id) | Preview::Discography(id)) => self.update(Message::ArtistOpen(id)),
                    Some(Preview::Filter(query)) => {
                        self.preview = None;
                        let page = self.update(Message::Open(Section::Likes));
                        // Отдельным сообщением: смена страницы очищает поиск уже после этого обработчика.
                        Task::batch([page, Task::done(Message::Filter(query))])
                    }
                    Some(Preview::Settings) => {
                        self.preview = None;
                        self.update(Message::Open(Section::Settings))
                    }
                    Some(Preview::WaveRestored) => {
                        self.preview = None;
                        self.section = Section::Wave;
                        session::load().map_or(Task::none(), |saved| self.restore(saved))
                    }
                    Some(Preview::Books) => {
                        self.preview = None;
                        self.section = Section::Books;
                        self.home_load(Section::Books, 1)
                    }
                    Some(Preview::Downloads) => {
                        self.preview = None;
                        self.update(Message::Open(Section::Downloads))
                    }
                    Some(Preview::Kids) => {
                        self.preview = None;
                        self.section = Section::Kids;
                        self.home_load(Section::Kids, 0)
                    }
                    Some(Preview::EqBar) => {
                        self.preview = None;
                        let (width, height) = *WINDOW.lock().unwrap();
                        self.eq_menu = Some(eq_place((width - 200.0, height - 60.0)));
                        session::load().map_or(Task::none(), |saved| self.restore(saved))
                    }
                    Some(Preview::Full) => {
                        self.preview = None;
                        self.player_full = true;
                        session::load().map_or(Task::none(), |saved| self.restore(saved))
                    }
                    Some(Preview::Restored) => {
                        self.preview = None;
                        self.section = Section::Queue;
                        session::load().map_or(Task::none(), |saved| self.restore(saved))
                    }
                    Some(Preview::Account) => {
                        self.preview = None;
                        self.update(Message::DialogOpen(Dialog::Account))
                    }
                    Some(Preview::LogoutAsk) => {
                        self.preview = None;
                        self.update(Message::DialogOpen(Dialog::Logout))
                    }
                    Some(Preview::PinMenu | Preview::PinPlay(_)) => Task::none(),
                    Some(Preview::Search(query)) => {
                        self.preview = None;
                        self.section = Section::Search;
                        self.query = query;
                        self.update(Message::Search)
                    }
                    None if self.section == Section::Likes => self.likes_more(),
                    // Главная сразу показывает трек и управление, как в оригинале.
                    None if self.queue.is_empty() => match session::load() {
                        Some(saved) => self.restore(saved),
                        None => self.token().map_or(Task::none(), |token| Task::perform(background(move || api::wave_last(&token)), Message::WaveLast)),
                    },
                    None => Task::none(),
                };
                return Task::batch([pins, page]);
            }
            Message::PinsLoaded(Ok(pins)) => {
                if self.pin_saving > 0 {
                    return Task::none();
                }
                self.pins = self.pins_shown(pins);
                let jobs = self.pins.iter().filter_map(|p| Some((format!("{PIN_COVER_PREFIX}{}", p.key()), p.cover.clone()?))).collect();
                let covers = self.fetch_cover_jobs(jobs, "100x100");
                match self.preview.clone() {
                    Some(Preview::PinMenu) => {
                        self.preview = None;
                        self.pin_menu = Some((0, 200.0, 300.0));
                    }
                    Some(Preview::PinPlay(index)) => {
                        self.preview = None;
                        self.muted = true;
                        self.apply_volume();
                        return Task::batch([covers, self.handle(Message::PinOpen(index))]);
                    }
                    _ => {}
                }
                return covers;
            }
            // Закрепления не должны мешать музыке.
            Message::PinsLoaded(Err(_)) => {}
            // Закреп включает то, что на нём: волну, плейлист, альбом или исполнителя.
            Message::PinOpen(index) => {
                let Some(pin) = self.pins.get(index).cloned() else { return Task::none() };
                if self.pin_active(&pin) && self.current.is_some() {
                    return self.handle(Message::Toggle);
                }
                self.pin_wanted += 1;
                match pin.target {
                    PinTarget::Wave(seeds) => {
                        let title = (seeds != [api::MY_WAVE]).then_some(pin.title);
                        return self.handle(Message::WaveStartSeeds(seeds, title));
                    }
                    PinTarget::List(list) => {
                        let Some(token) = self.token() else { return Task::none() };
                        let (id, title, wanted) = (self.pin_wanted, pin.title, list.clone());
                        return Task::perform(background(move || api::playlist_tracks(&wanted, &token)), move |r| Message::PinTracks(id, list.clone(), title.clone(), r));
                    }
                }
            }
            Message::PinTracks(id, list, title, result) => {
                if id != self.pin_wanted {
                    return Task::none();
                }
                return match result {
                    Ok(tracks) if tracks.is_empty() => self.notify("Здесь пока нет треков".to_string()),
                    Ok(tracks) => {
                        self.queue_from = self.queue_from_list(list);
                        self.play_list(tracks, title, 0)
                    }
                    Err(e) => self.fail(e),
                };
            }
            // Страница закрепа: открывается из его меню.
            Message::PinPage(index) => {
                self.pin_menu = None;
                match self.pins.get(index).map(|p| p.target.clone()) {
                    Some(PinTarget::List(list)) if list.source == Source::Artist => return self.handle(Message::ArtistOpen(list.kind)),
                    Some(PinTarget::List(list)) if self.is_favorites(&list) => return self.handle(Message::Open(Section::Likes)),
                    Some(PinTarget::List(list)) => return self.open_list(list),
                    Some(PinTarget::Wave(_)) | None => {}
                }
            }
            Message::PinMenu(index) => {
                let (x, y) = *CURSOR.lock().unwrap();
                self.pin_menu = Some((index, x, y));
            }
            Message::Quit => {}
            Message::WaveResumed(seeds, result) => {
                // Если волну так и не удалось продолжить, очередь всё равно доиграет.
                let Ok(wave) = result else { return Task::none() };
                if !self.wave_queue || self.wave_base != seeds || self.wave.is_some() || self.wave_pending.is_some() {
                    return Task::none();
                }
                if !self.active {
                    self.wave_pending = Some(wave.session);
                    return Task::none();
                }
                self.wave = Some(wave.session);
                self.feedback("radioStarted", None, None);
                return self.wave_refill();
            }
            Message::AiLessLoaded(result) => match result {
                Ok(enabled) => self.ai_less = Some(enabled),
                Err(e) => return self.fail_retry(e, Message::Open(Section::Settings)),
            },
            Message::AiLessSet(enabled) => {
                let Some(token) = self.token() else { return Task::none() };
                self.ai_less = Some(enabled);
                return Task::perform(background(move || api::set_ai_reduction(enabled, &token)), Message::AiLessSaved);
            }
            Message::AiLessSaved(result) => {
                if let Err(e) = result {
                    self.ai_less = self.ai_less.map(|enabled| !enabled);
                    return self.fail(e);
                }
            }
            Message::OpenUrl(url) => {
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return Task::none();
                }
                if let Err(e) = std::process::Command::new("explorer").arg(&url).spawn() {
                    return self.fail(format!("Не удалось открыть браузер: {e}"));
                }
            }
            Message::DiscographyOpen(also) => {
                self.discography_also = also;
                self.discography_by_year = false;
                return self.handle(Message::Open(Section::Discography));
            }
            Message::DiscographySort(by_year) => {
                if by_year != self.discography_by_year {
                    self.discography_by_year = by_year;
                    return self.discography_load(0);
                }
            }
            Message::SearchAgain(query) => {
                self.query = query;
                return self.handle(Message::Search);
            }
            Message::SearchHistoryClear => {
                self.searches.clear();
                session::save_searches(Vec::new());
            }
            Message::TagLoaded(tag, result) => {
                if self.tag.as_ref().is_none_or(|(open, _)| *open != tag) {
                    return Task::none();
                }
                self.tag_loading = false;
                match result {
                    Ok((name, lists)) => {
                        if let (Some(name), Some((_, title))) = (name, self.tag.as_mut()) {
                            *title = name;
                        }
                        self.tag_lists = lists;
                        let jobs = self.tag_lists.iter().filter_map(|item| Some((home_item_key(item), home_item_cover(item)?))).collect();
                        return self.fetch_cover_jobs(jobs, "200x200");
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::DiscographyMore => {
                if self.discography_key.is_none() && (self.discography.len() as u64) < self.discography_total {
                    return self.discography_load(self.discography.len() as u64 / api::RELEASES_PAGE);
                }
            }
            Message::AlbumOthers(id, result) => {
                let Some(album) = self.album.as_ref().filter(|album| album.id == id) else { return Task::none() };
                let skip: HashSet<&str> = album.versions.iter().map(|c| c.id.as_str()).chain([album.id.as_str()]).collect();
                let others: Vec<Card> = result.map(|releases| releases.albums).unwrap_or_default().into_iter().filter(|c| !skip.contains(c.id.as_str())).take(ALBUM_OTHERS).collect();
                let jobs = others.iter().filter_map(|c| Some((format!("{ALBUM_COVER}{}", c.id), c.cover.clone()?))).collect();
                self.album_others = others;
                return self.fetch_cover_jobs(jobs, "300x300");
            }
            Message::DiscographyLoaded(key, result) => {
                if self.discography_key.as_ref() != Some(&key) {
                    return Task::none();
                }
                self.discography_key = None;
                match result {
                    Ok(releases) => {
                        let jobs = releases.albums.iter().filter_map(|c| Some((format!("{ALBUM_COVER}{}", c.id), c.cover.clone()?))).collect();
                        self.discography.extend(releases.albums);
                        self.discography_total = releases.total;
                        return self.fetch_cover_jobs(jobs, "300x300");
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::OpenLink(link) => {
                // Покупки и настройки аккаунта живут на сайтах Яндекса.
                if let Err(e) = std::process::Command::new("explorer").arg(link).spawn() {
                    return self.fail(format!("Не удалось открыть браузер: {e}"));
                }
            }
            // Без сети приложение открывается с аккаунтом прошлого запуска и сразу на скачанном.
            Message::Account(Err(e)) if api::network_error(&e) && self.profile.is_none() => {
                let Some(account) = offline::load_account() else { return self.fail_retry(e, Message::AccountRetry) };
                self.offline = true;
                let opened = self.update(Message::Account(Ok(account)));
                self.offline = true;
                self.retry = None;
                let downloads = if self.downloads.is_empty() { Task::none() } else { self.update(Message::Open(Section::Downloads)) };
                return Task::batch([opened, downloads]);
            }
            Message::Account(Err(e)) => return self.fail_retry(e, Message::AccountRetry),
            Message::OnlineCheck => {
                let Some(token) = self.token().filter(|_| self.offline && !self.online_checking) else { return Task::none() };
                self.online_checking = true;
                return Task::perform(background(move || api::profile(&token).map(|_| ())), Message::OnlineChecked);
            }
            Message::OnlineChecked(result) => {
                self.online_checking = false;
                if result.is_err() || !self.offline {
                    return Task::none();
                }
                self.offline = false;
                if let Some(token) = self.token() {
                    offline::flush_plays(token);
                }
                // Обложки, которые не загрузились без сети, можно просить снова.
                let covers = &self.covers;
                self.requested.borrow_mut().retain(|key| covers.contains_key(key));
                let back = self.notify("Снова в сети".to_string());
                let reload = match self.retry.take() {
                    Some(retry) => self.update(retry),
                    None if self.profile.is_none() => self.update(Message::AccountRetry),
                    None => Task::none(),
                };
                return Task::batch([back, reload]);
            }
            Message::AccountRetry => {
                if let Some(token) = self.token() {
                    return load_account(token);
                }
            }
            Message::Back => {
                if let Some(place) = self.history.pop() {
                    self.forward.push(self.here());
                    return self.go(place);
                }
            }
            Message::Forward => {
                if let Some(place) = self.forward.pop() {
                    self.history.push(self.here());
                    return self.go(place);
                }
            }
            Message::Open(section) => {
                self.player_full = false;
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
                    Section::Home | Section::Books | Section::Kids if self.home_page != section || (self.home.blocks.is_empty() && !self.home_loading) => {
                        let tab = self.landing_tabs[landing_index(section)];
                        self.home_load(section, tab)
                    }
                    Section::Dislikes => {
                        let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                        self.history_loading = true;
                        Task::perform(background(move || api::disliked_tracks(&uid, HISTORY_LIMIT, &token)), Message::DislikesLoaded)
                    }
                    Section::History => {
                        let Some(token) = self.token() else { return Task::none() };
                        self.history_loading = true;
                        Task::perform(background(move || api::history(HISTORY_LIMIT, &token)), Message::HistoryLoaded)
                    }
                    Section::Collection => {
                        let likes = if self.liked.is_empty() { self.likes_more() } else { Task::none() };
                        let lists = if self.playlists.is_empty() { self.playlists_load() } else { Task::none() };
                        Task::batch([likes, lists])
                    }
                    Section::Likes if self.liked.is_empty() => self.likes_more(),
                    Section::Playlists if self.playlists.is_empty() => self.playlists_load(),
                    Section::Search => operation::focus(Id::new(SEARCH_INPUT)),
                    Section::Discography => self.discography_load(0),
                    Section::Settings if self.ai_less.is_none() => {
                        let Some(token) = self.token() else { return Task::none() };
                        Task::perform(background(move || api::ai_reduction(&token)), Message::AiLessLoaded)
                    }
                    _ => Task::none(),
                };
            }
            Message::Query(query) => {
                if query.trim() != self.query.trim() {
                    self.search_cache.clear();
                }
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
            Message::Filter(filter) => {
                self.filter = filter;
                // Новая буква отменяет поиск, который ещё ждёт паузы или ответа.
                self.filter_id += 1;
                self.list_offset = 0.0;
                let snap = operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START);
                if !self.filtering() {
                    self.filter_tracks.clear();
                    self.filter_loading = false;
                    return snap;
                }
                self.filter_loading = true;
                let id = self.filter_id;
                return Task::batch([snap, Task::perform(background(|| std::thread::sleep(SEARCH_DELAY)), move |()| Message::FilterTyped(id))]);
            }
            Message::FilterTyped(id) => {
                let (Some((owner, kind)), Some(token)) = (self.filter_place(), self.token()) else { return Task::none() };
                if id == self.filter_id {
                    let part = self.filter.trim().to_string();
                    return Task::perform(background(move || api::search_playlist(&owner, &kind, &part, &token)), move |r| Message::FilterLoaded(id, r));
                }
            }
            Message::FilterLoaded(id, result) => {
                if id == self.filter_id {
                    self.filter_loading = false;
                    self.list_offset = 0.0;
                    // Сервер не ответил — ищем хотя бы среди загруженного.
                    self.filter_tracks = result.unwrap_or_else(|_| {
                        let loaded = if self.section == Section::Likes { &self.liked } else { &self.playlist_tracks };
                        loaded.iter().filter(|track| track_matches(track, &self.filter)).cloned().collect()
                    });
                    return operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START);
                }
            }
            Message::SearchTabSet(tab) => {
                if self.search_tab != tab {
                    self.search_tab = tab;
                    self.list_offset = 0.0;
                    let snap = operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::START);
                    return Task::batch([snap, self.search_run()]);
                }
            }
            Message::ListOpen(playlist) => return self.open_list(playlist),
            Message::HomeLoaded(page, tab, result) => {
                // Пока грузилось, могли выбрать другую вкладку или другую страницу.
                if tab == self.home_tab && page == self.home_page {
                    self.home_loading = false;
                    match result {
                        Ok(home) => self.home = home,
                        Err(e) => return self.fail_retry(e, Message::HomeTab(tab)),
                    }
                }
            }
            Message::HomeTab(tab) => return self.home_load(self.home_page, tab),
            Message::HomeBlockTab(index, tab) => {
                let Some(block) = self.home.blocks.get_mut(index) else { return Task::none() };
                let Some((_, items)) = block.tabs.get(tab) else { return Task::none() };
                block.items = items.clone();
                self.home_block_tabs.insert(index, tab);
                self.home_pages.remove(&index);
                return self.home_covers();
            }
            Message::HomePage(index, forward) => {
                let step = self.cards_in_row();
                let total = self.home.blocks.get(index).map_or(0, |b| b.items.len());
                let from = self.home_pages.entry(index).or_insert(0);
                *from = if forward { (*from + step).min(total.saturating_sub(step)) } else { from.saturating_sub(step) };
            }
            Message::HomePlay(block, index) => {
                if let Some(block) = self.home.blocks.get(block) {
                    let (tracks, title) = (block.tracks.clone(), block.title.clone());
                    self.queue_from = QueueFrom::Unknown;
                    return self.play_list(tracks, title, index);
                }
            }
            Message::HomeOpen(item) => match item {
                HomeItem::Album(card) => return self.update(open_album(&card)),
                HomeItem::Artist(card) => return self.update(open_artist(&card)),
                HomeItem::Playlist(list) => return self.open_list(list),
                HomeItem::Wave { seeds, title, .. } => return self.update(Message::WaveStartSeeds(seeds, Some(title))),
                HomeItem::Likes { .. } => return self.update(Message::Open(Section::Likes)),
                HomeItem::Mix { tag, title, .. } => {
                    let Some(token) = self.token() else { return Task::none() };
                    if self.tag.as_ref().is_none_or(|(open, _)| *open != tag) {
                        self.tag_lists.clear();
                        self.tag_loading = true;
                    }
                    self.tag = Some((tag.clone(), title));
                    let open = self.handle(Message::Open(Section::Tag));
                    let wanted = tag.clone();
                    // Категория книг и подкастов грузится как страница из блоков, микс — по тегу.
                    let load = move || match tag.strip_prefix(api::CATEGORY) {
                        Some(page) => api::category(page, &token).map(|items| (None, items)),
                        None => api::tag_playlists(&tag, &token).map(|(name, lists)| (name, lists.into_iter().map(HomeItem::Playlist).collect())),
                    };
                    return Task::batch([open, Task::perform(background(load), move |result| Message::TagLoaded(wanted.clone(), result))]);
                }
                HomeItem::History { .. } => return self.update(Message::Open(Section::History)),
            },
            Message::LikedPages(Ok((albums, artists))) => {
                self.liked_albums = albums;
                self.liked_artists = artists;
                return self.fetch_card_covers(self.liked_albums.iter(), self.liked_artists.iter(), &[]);
            }
            // Коллекция покажется без этих блоков; музыке это не мешает.
            Message::LikedPages(Err(_)) => {}
            Message::EpisodesOrder(old_first) => {
                self.episodes_old_first = old_first;
                self.playlist_tracks = self.episodes_arranged();
            }
            Message::SeasonPick(season) => {
                self.season = season;
                self.playlist_tracks = self.episodes_arranged();
            }
            Message::EpisodeFinished(track, finished) => {
                self.menu = None;
                let Some(token) = self.token() else { return Task::none() };
                self.progress.insert(track.id.clone(), if finished { track.duration.as_secs_f32() } else { 0.0 });
                let id = track.id.clone();
                return Task::perform(background(move || api::mark_finished(&id, finished, &token)), Message::Marked);
            }
            Message::AlbumFinished(finished) => {
                let (Some(token), Some(album)) = (self.token(), self.album.as_mut()) else { return Task::none() };
                album.finished = finished;
                let id = album.id.clone();
                let all = if self.episodes.is_empty() { &self.playlist_tracks } else { &self.episodes };
                for track in all.iter().filter(|track| track.remember) {
                    self.progress.insert(track.id.clone(), if finished { track.duration.as_secs_f32() } else { 0.0 });
                }
                return Task::perform(background(move || api::mark_album_finished(&id, finished, &token)), Message::Marked);
            }
            Message::Marked(Ok(())) => {}
            Message::Marked(Err(e)) => return self.fail(e),
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
            Message::Found(id, tab, text, result) => {
                if id != self.search_id {
                    return Task::none();
                }
                self.searching = false;
                match result {
                    Ok(found) => {
                        self.search_cache.insert((text, tab), found.clone());
                        let covers = self.search_show(found);
                        match self.preview.take() {
                            Some(Preview::Search(_)) => {
                                self.queue = self.results.clone();
                                self.current = Some(0).filter(|_| !self.queue.is_empty());
                            }
                            Some(Preview::SearchTabs(step)) => return Task::batch([covers, Task::perform(background(|| std::thread::sleep(Duration::from_millis(1500))), move |()| Message::PreviewSearchTab(step))]),
                            other => self.preview = other,
                        }
                        return covers;
                    }
                    Err(e) => return self.fail_retry(e, Message::Search),
                }
            }
            Message::LikesMore => return self.likes_more(),
            Message::Scrolled(offset) => self.list_offset = offset,
            Message::LyricsScrolled(offset, place) => {
                self.list_offset = offset;
                self.lyrics_view = place;
            }
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
                    let main = album.cover.clone().map(|uri| (format!("{ALBUM_COVER}{}", album.id), uri));
                    let versions = album.versions.iter().filter_map(|c| Some((format!("{ALBUM_COVER}{}", c.id), c.cover.clone()?)));
                    let cover = main.into_iter().chain(versions).collect();
                    // Другие альбомы — дополнение: страница не ждёт их и не ругается без них.
                    let others = match (album.artists.first().map(|(_, id)| id.clone()).filter(|id| !id.is_empty()), self.token()) {
                        (Some(artist), Some(token)) => {
                            let id = album.id.clone();
                            Task::perform(background(move || api::releases(&artist, false, false, 0, &token)), move |result| Message::AlbumOthers(id.clone(), result))
                        }
                        _ => Task::none(),
                    };
                    let podcast = album.kind == AlbumKind::Podcast;
                    self.album = Some(album);
                    self.episodes_old_first = false;
                    self.season = None;
                    let tracks = if podcast {
                        self.episodes = tracks;
                        self.episodes_arranged()
                    } else {
                        self.episodes.clear();
                        tracks
                    };
                    let cover = self.fetch_cover_jobs(cover, "300x300");
                    return Task::batch([cover, others, self.handle(Message::PlaylistLoaded(kind, Ok(tracks)))]);
                }
                Err(e) => return self.handle(Message::PlaylistLoaded(kind, Err(e))),
            },
            Message::ArtistOpen(id) => {
                self.menu = None;
                self.remember();
                self.section = Section::Artist;
                self.artist = None;
                return self.handle(Message::ArtistLoad(id));
            }
            Message::ArtistByName(name) => {
                self.menu = None;
                let Some(token) = self.token() else { return Task::none() };
                let wanted = name.clone();
                return Task::perform(background(move || api::search_mixed(&name, SearchTab::Artists.types(), ARTIST_SEARCH, &token)), move |r| Message::ArtistFound(wanted.clone(), r));
            }
            Message::ArtistFound(name, result) => match result {
                Ok(found) => match pick_artist(&found.artists, &name) {
                    Some(id) => return self.update(Message::ArtistOpen(id)),
                    None => return self.notify(format!("Исполнитель {name} не найден")),
                },
                Err(e) => return self.fail(e),
            },
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
                        let covers = self.fetch_cover_jobs(jobs, "300x300");
                        return match self.preview.clone() {
                            Some(Preview::Discography(_)) => {
                                self.preview = None;
                                Task::batch([covers, self.handle(Message::DiscographyOpen(false))])
                            }
                            Some(Preview::ArtistAbout(_)) => {
                                self.preview = None;
                                let down = Task::perform(background(|| std::thread::sleep(Duration::from_secs(2))), |()| Message::PreviewArtistEnd);
                                Task::batch([covers, down])
                            }
                            _ => covers,
                        };
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
                        if matches!(self.preview, Some(Preview::TrackInfo)) {
                            self.preview = None;
                            if let Some(track) = self.liked.first().cloned() {
                                return self.handle(Message::DialogOpen(Dialog::TrackInfo(track)));
                            }
                        }
                        if matches!(self.preview, Some(Preview::QueueJump)) {
                            self.preview = None;
                            self.queue = self.liked.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                            self.muted = true;
                            self.apply_volume();
                            let page = self.handle(Message::Open(Section::Queue));
                            let down = Task::perform(background(|| std::thread::sleep(Duration::from_millis(800))), |()| Message::PreviewQueueDown);
                            return Task::batch([page, down]);
                        }
                        if matches!(self.preview, Some(Preview::Panels)) {
                            self.preview = None;
                            self.queue = self.liked.clone();
                            self.queue_title = Some("Мне нравится".to_string());
                            self.queue_from = QueueFrom::Likes;
                            self.current = Some(20).filter(|i| *i < self.queue.len());
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
            Message::WaveStart => return self.update(Message::WaveStartSeeds(vec![api::MY_WAVE.to_string()], None)),
            // Не вышло узнать — начинаем обычную Мою волну.
            Message::WaveLast(result) => {
                if self.queue.is_empty() {
                    let (seeds, title) = result.unwrap_or_else(|_| (vec![api::MY_WAVE.to_string()], None));
                    return self.wave_load(seeds, title, false);
                }
            }
            Message::WaveReset => {
                self.wave_tuning = [0; 4];
                self.save_settings();
                return self.update(Message::WaveStart);
            }
            Message::WaveTune(group, choice) => {
                self.wave_tuning[group] = choice;
                self.save_settings();
                return self.update(Message::WaveStartSeeds(self.wave_base.clone(), self.wave_title.clone()));
            }
            Message::WaveTuneReset => {
                self.wave_tuning = [0; 4];
                self.save_settings();
                return self.update(Message::WaveStartSeeds(self.wave_base.clone(), self.wave_title.clone()));
            }
            Message::LyricsToggle => {
                if self.section == Section::Lyrics {
                    return self.handle(Message::Back);
                }
                return self.handle(Message::Open(Section::Lyrics));
            }
            Message::LyricsLoaded(id, result) => {
                if self.lyrics_loading.as_deref() == Some(id.as_str()) {
                    self.lyrics_loading = None;
                }
                self.lyrics_line = None;
                self.lyrics_shown = -1.0;
                self.lyrics_target = -1.0;
                self.lyrics_fresh = true;
                return match result {
                    Ok(found) => {
                        self.lyrics = Some((id, found));
                        Task::none()
                    }
                    // Текст не главное: хватит надписи, что его нет. Важен только истёкший вход.
                    Err(e) => {
                        self.lyrics = Some((id, None));
                        if e == api::NEED_LOGIN {
                            self.fail(e)
                        } else {
                            Task::none()
                        }
                    }
                };
            }
            Message::LyricsSeek(time) => {
                self.seeking = Some(time);
                return self.update(Message::SeekDone);
            }
            Message::WaveStartSeeds(seeds, title) => {
                if self.section != Section::Wave {
                    self.remember();
                    self.section = Section::Wave;
                }
                return self.wave_load(seeds, title, true);
            }
            Message::WaveStarted(_, seeds, _, _) if seeds != self.wave_wanted => {}
            Message::WaveStarted(autoplay, seeds, title, result) => {
                self.wave_loading = false;
                match result {
                    Ok(wave) if autoplay && wave.tracks.is_empty() => {
                        return self.notify("Для этой волны у Яндекса нет треков".to_string());
                    }
                    // Пока грузилась фоновая волна, пользователь мог включить свой список.
                    Ok(_) if !autoplay && self.current.is_some() && !matches!(self.preview, Some(Preview::Wave | Preview::WaveSeek | Preview::WaveSeeded | Preview::Lyrics)) => {}
                    Ok(wave) => {
                        if let Some(p) = &self.player {
                            p.stop();
                        }
                        self.active = false;
                        self.starting = false;
                        self.wave = None;
                        self.wave_pending = Some(wave.session);
                        self.queue_title = None;
                        self.wave_title = title;
                        self.wave_base = seeds;
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
                        if matches!(self.preview, Some(Preview::Lyrics)) {
                            self.section = Section::Lyrics;
                            self.current = self.queue.iter().position(|track| track.lyrics).or(self.current);
                            self.position = Duration::from_secs(180);
                        }
                        if matches!(self.preview, Some(Preview::Wave | Preview::WaveSeek | Preview::WaveSeeded | Preview::Lyrics)) {
                            self.preview = None;
                        }
                        if autoplay {
                            return Task::batch([covers, self.play(0)]);
                        }
                        return covers;
                    }
                    Err(e) => return self.fail_retry(e, Message::WaveStartSeeds(seeds, title)),
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
                        if matches!(self.preview, Some(Preview::PageMenu)) {
                            let at = self.playlists.iter().position(|p| p.title == "Загруженное").unwrap_or(0);
                            return Task::batch([covers, self.update(Message::PlaylistOpen(at))]);
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
            Message::LinkTrack(result) => match result.map(|tracks| tracks.into_iter().next()) {
                Ok(Some(track)) if track.album.is_some() => return self.handle(Message::OpenAlbum(track)),
                // Трек без альбома показать негде, поэтому просто включаем.
                Ok(Some(track)) => {
                    self.queue_from = QueueFrom::Unknown;
                    let title = track.title.clone();
                    return self.play_list(vec![track], title, 0);
                }
                Ok(None) => return self.fail("Этот трек недоступен".to_string()),
                Err(e) => return self.fail(e),
            },
            Message::LinkPlaylist(result) => match result {
                Ok(list) => return self.open_list(list),
                Err(e) => return self.fail(e),
            },
            Message::SimilarLoaded(kind, result) => {
                if self.playlist.as_ref().is_some_and(|list| list.kind == kind) {
                    // Похожее не главное на странице: если не пришло, просто его не показываем.
                    self.similar = result.unwrap_or_default();
                    self.similar.truncate(SIMILAR_LIMIT);
                    if std::mem::take(&mut self.preview_end) {
                        return operation::snap_to(Id::new(LIST), scrollable::RelativeOffset { x: 0.0, y: 1.0 });
                    }
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
                        if matches!(preview, Some(Preview::PageMenu)) {
                            self.page_menu = Some((560.0, 175.0));
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
                self.menu_artists = false;
                self.menu_lists = false;
            }
            Message::MenuClose => {
                // Esc сначала закрывает меню и окна, потом плеер на всё окно.
                if self.menu.is_none() && self.pin_menu.is_none() && self.quality_menu.is_none() && self.eq_menu.is_none() && self.page_menu.is_none() && self.dialog.is_none() {
                    self.player_full = false;
                }
                self.menu = None;
                self.pin_menu = None;
                self.quality_menu = None;
                self.eq_menu = None;
                self.page_menu = None;
                self.dialog = None;
            }
            Message::MenuArtists => self.menu_artists = true,
            Message::MenuLists => {
                self.menu_lists = true;
                if self.playlists.is_empty() {
                    return self.playlists_load();
                }
            }
            Message::DialogOpen(dialog) => {
                self.menu = None;
                self.dialog_text = match &dialog {
                    Dialog::Rename(list) => list.title.clone(),
                    Dialog::Describe(_) => self.list_meta.as_ref().map_or_else(String::new, |meta| meta.1.clone()),
                    _ => String::new(),
                };
                if let Dialog::TrackInfo(track) = &dialog {
                    let Some(token) = self.token() else { return Task::none() };
                    let id = track.id.clone();
                    self.track_info = None;
                    self.dialog = Some(dialog);
                    return Task::perform(background({
                        let id = id.clone();
                        move || api::track_details(&id, &token)
                    }), move |r| Message::TrackInfoLoaded(id.clone(), r));
                }
                let focus = !matches!(dialog, Dialog::Delete(_) | Dialog::RemoveTrack(_) | Dialog::WaveTune | Dialog::Logout | Dialog::Account | Dialog::ClearDownloads | Dialog::DownloadLikes);
                self.dialog = Some(dialog);
                if focus {
                    return operation::focus(Id::new(DIALOG_INPUT));
                }
            }
            Message::DialogInput(value) => self.dialog_text = value,
            Message::TrackInfoLoaded(id, result) => {
                if matches!(&self.dialog, Some(Dialog::TrackInfo(track)) if track.id == id) {
                    self.track_info = Some((id, result.unwrap_or_default()));
                }
            }
            Message::TrackInfoPlay(index) => {
                let Some((_, details)) = &self.track_info else { return Task::none() };
                let similar = details.similar.clone();
                self.dialog = None;
                return self.play_list(similar, "Похожие треки".to_string(), index);
            }
            Message::MenuLyrics => {
                self.menu = None;
                return self.handle(Message::Open(Section::Lyrics));
            }
            Message::DialogConfirm => {
                let (Some(uid), Some(token), Some(dialog)) = (self.uid.clone(), self.token(), self.dialog.clone()) else { return Task::none() };
                let title = self.dialog_text.trim().to_string();
                if title.is_empty() && matches!(dialog, Dialog::Create | Dialog::Rename(_)) {
                    return Task::none();
                }
                self.dialog = None;
                return match dialog {
                    Dialog::WaveTune | Dialog::TrackInfo(_) | Dialog::Account => Task::none(),
                    Dialog::Logout => self.handle(Message::Logout),
                    Dialog::ClearDownloads => self.handle(Message::DownloadsClear),
                    Dialog::DownloadLikes => self.handle(Message::DownloadLikes),
                    Dialog::Create => Task::perform(background(move || api::create_playlist(&uid, &title, &token)), Message::PlaylistCreated),
                    Dialog::Rename(list) => {
                        let kind = list.kind;
                        let (wanted, name) = (kind.clone(), title.clone());
                        Task::perform(background(move || api::rename_playlist(&uid, &kind, &title, &token)), move |r| Message::PlaylistRenamed(wanted.clone(), name.clone(), r))
                    }
                    Dialog::Describe(list) => {
                        if let Some(meta) = &mut self.list_meta {
                            meta.1 = title.clone();
                        }
                        Task::perform(background(move || api::set_description(&uid, &list.kind, &title, &token)), Message::SimpleSaved)
                    }
                    // Плейлист сразу пропадает из окна, а удаляется на сервере, когда выйдет время отмены.
                    Dialog::Delete(list) => {
                        let at = self.playlists.iter().position(|p| p.kind == list.kind).unwrap_or(0);
                        self.playlists.retain(|p| p.kind != list.kind);
                        let leave = self.playlist.as_ref().is_some_and(|open| open.kind == list.kind) && self.section == Section::Playlists;
                        let notice = self.pending_start(Pending::Playlist(list.clone(), at), format!("Плейлист «{}» удалён", list.title));
                        let back = if leave { self.handle(Message::Back) } else { Task::none() };
                        let _ = (uid, token);
                        Task::batch([notice, back])
                    }
                    Dialog::RemoveTrack(track) => self.handle(Message::TrackRemove(track)),
                };
            }
            Message::PlaylistCreated(Ok(list)) => {
                self.playlists.insert(0, list.clone());
                return self.open_list(list);
            }
            Message::PlaylistRenamed(kind, title, Ok(())) => {
                for list in self.playlists.iter_mut().chain(self.playlist.as_mut()).filter(|list| list.kind == kind) {
                    list.title = title.clone();
                }
            }
            Message::PlaylistDeleted(kind, Ok(())) => {
                self.playlists.retain(|list| list.kind != kind);
                self.history.retain(|place| place.playlist.as_ref().is_none_or(|list| list.kind != kind));
                self.forward.retain(|place| place.playlist.as_ref().is_none_or(|list| list.kind != kind));
                if self.playlist.as_ref().is_some_and(|list| list.kind == kind) {
                    return self.handle(Message::Back);
                }
            }
            Message::PlaylistCreated(Err(e)) | Message::PlaylistRenamed(_, _, Err(e)) | Message::PlaylistDeleted(_, Err(e)) => return self.fail(e),
            Message::TrackAdd(track, list) => {
                self.menu = None;
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let (kind, title) = (list.kind.clone(), list.title.clone());
                return Task::perform(background(move || api::add_to_playlist(&uid, &list.kind, &track, &token)), move |r| Message::TrackAdded(kind.clone(), title.clone(), r));
            }
            Message::TrackAdded(kind, title, Ok(())) => {
                let notice = self.notify(format!("Трек добавлен в плейлист {title}"));
                self.playlists_loading = false;
                let lists = self.playlists_load();
                let open = match self.playlist.clone().filter(|list| list.kind == kind) {
                    Some(list) => self.load_list(list),
                    None => Task::none(),
                };
                return Task::batch([notice, lists, open]);
            }
            Message::TrackAdded(_, _, Err(e)) | Message::TrackRemoved(Err(e)) => return self.fail(e),
            Message::TrackRemove(track) => {
                self.menu = None;
                let Some(list) = self.playlist.as_mut() else { return Task::none() };
                list.count = list.count.saturating_sub(1);
                let list = list.clone();
                let at = self.playlist_tracks.iter().position(|t| t.id == track.id).unwrap_or(0);
                self.playlist_tracks.retain(|t| t.id != track.id);
                self.filter_tracks.retain(|t| t.id != track.id);
                return self.pending_start(Pending::Track(list, track, at), "Трек удалён из плейлиста".to_string());
            }
            Message::TrackRemoved(Ok(())) => {
                self.playlists_loading = false;
                return self.playlists_load();
            }
            Message::DislikesLoaded(result) => {
                self.history_loading = false;
                match result {
                    Ok(tracks) => self.disliked_list = tracks,
                    Err(e) => return self.fail_retry(e, Message::Open(Section::Dislikes)),
                }
            }
            Message::LikedLists(Ok(lists)) => {
                self.liked_lists = lists;
                return self.fetch_card_covers(std::iter::empty(), std::iter::empty(), &self.liked_lists);
            }
            Message::LikedLists(Err(_)) => {}
            Message::ListLike(list) => {
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let liked = !self.liked_lists.iter().any(|l| l.kind == list.kind && l.owner == list.owner);
                if liked {
                    self.liked_lists.insert(0, list.clone());
                } else {
                    self.liked_lists.retain(|l| l.kind != list.kind || l.owner != list.owner);
                }
                return Task::perform(background(move || api::set_playlist_liked(&uid, &list.owner, &list.kind, liked, &token)), Message::SimpleSaved);
            }
            Message::ListMeta(kind, Ok((description, visibility))) => {
                if self.playlist.as_ref().is_some_and(|list| list.kind == kind) {
                    self.list_meta = Some((kind, description, visibility));
                }
            }
            // Без описания плейлист всё равно работает.
            Message::ListMeta(_, Err(_)) => {}
            Message::Visibility(list) => {
                let (Some(uid), Some(token), Some(meta)) = (self.uid.clone(), self.token(), self.list_meta.as_mut()) else { return Task::none() };
                let value = if meta.2 == "private" { "public" } else { "private" };
                meta.2 = value.to_string();
                return Task::perform(background(move || api::set_visibility(&uid, &list.kind, value, &token)), Message::SimpleSaved);
            }
            // Панель меняется сразу, а сервер догоняет: ответ идёт больше секунды.
            Message::Pin(target) => {
                self.pin_menu = None;
                let Some(token) = self.token() else { return Task::none() };
                let pinned = !self.pins.iter().any(|pin| target.matches(pin));
                if pinned {
                    let covers = self.pin_show(&target);
                    self.pin_saving += 1;
                    return Task::batch([covers, Task::perform(background(move || api::set_pinned(&target, true, &token)), Message::PinSaved)]);
                }
                self.pins.retain(|pin| !target.matches(pin));
                self.pin_saving += 1;
                return Task::perform(background(move || api::set_pinned(&target, false, &token)), Message::PinSaved);
            }
            Message::PinSaved(result) => {
                self.pin_saving = self.pin_saving.saturating_sub(1);
                let reload = if self.pin_saving > 0 { Task::none() } else { self.token().map_or(Task::none(), |token| Task::perform(background(move || api::pins(&token)), Message::PinsLoaded)) };
                return match result {
                    Ok(()) => reload,
                    Err(e) => Task::batch([reload, self.fail(e)]),
                };
            }
            Message::CopyText(link) => {
                let notice = self.notify("Ссылка скопирована".to_string());
                return Task::batch([iced::clipboard::write(link), notice]);
            }
            Message::SimpleSaved(Ok(())) => {}
            Message::SimpleSaved(Err(e)) => {
                let reload = self.liked_pages_load();
                return Task::batch([reload, self.fail(e)]);
            }
            Message::HistoryLoaded(result) => {
                self.history_loading = false;
                match result {
                    Ok(tracks) => self.history_tracks = tracks,
                    Err(e) => return self.fail_retry(e, Message::Open(Section::History)),
                }
            }
            Message::QualityOpen => self.quality_menu = Some(*CURSOR.lock().unwrap()),
            Message::QualitySet(quality) => {
                let changed = quality != self.quality;
                self.quality = quality;
                self.quality_menu = None;
                self.save_settings();
                // Играющий трек сразу переходит на новое качество с того же места.
                if let (true, true, Some(index)) = (changed, self.active, self.current) {
                    self.restart = Some(self.player.as_ref().is_some_and(Player::is_paused));
                    self.resume = Some(self.position);
                    self.preloaded = None;
                    return self.play(index);
                }
            }
            Message::PreviewFilter(step) => {
                const WORD: &str = "Mary";
                if step == 0 {
                    let next = Task::perform(background(|| std::thread::sleep(Duration::from_millis(500))), |()| Message::PreviewFilter(1));
                    return Task::batch([operation::focus(Id::new(FILTER_INPUT)), next]);
                }
                let filter = self.filter.clone();
                let check = operation::is_focused(Id::new(FILTER_INPUT)).map(move |focused| {
                    eprintln!("фильтр «{filter}», фокус {focused}");
                    Message::Frame(Instant::now())
                });
                if step > WORD.len() + 2 {
                    return check;
                }
                let typed = if step <= WORD.len() { self.update(Message::Filter(WORD[..step].to_string())) } else { Task::none() };
                let next = Task::perform(background(|| std::thread::sleep(Duration::from_millis(700))), move |()| Message::PreviewFilter(step + 1));
                return Task::batch([check, typed, next]);
            }
            Message::PreviewScroll(step) => {
                eprintln!("шаг {step}: раздел {:?}, прокрутка {:.0}", self.section, self.list_offset);
                let task = match step {
                    0 => operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y: 1500.0 }),
                    1 => self.update(Message::Open(Section::Home)),
                    2 => self.update(Message::Open(Section::Likes)),
                    _ => return Task::none(),
                };
                let next = Task::perform(background(|| std::thread::sleep(Duration::from_millis(2000))), move |()| Message::PreviewScroll(step + 1));
                return Task::batch([task, next]);
            }
            Message::PreviewType(step) => {
                const WORD: &str = "Linkin Park numb";
                let check = operation::is_focused(Id::new(SEARCH_INPUT)).map(move |focused| Message::PreviewFocused(step, focused));
                if step >= WORD.len() + 4 {
                    return check;
                }
                let typed = if step < WORD.len() { self.update(Message::Query(WORD[..=step].to_string())) } else { Task::none() };
                let pause = if step % 4 == 3 { 900 } else { 150 };
                let next = Task::perform(background(move || std::thread::sleep(Duration::from_millis(pause))), move |()| Message::PreviewType(step + 1));
                return Task::batch([check, typed, next]);
            }
            Message::PreviewFocused(step, focused) => eprintln!("шаг {step}: запрос «{}», найдено {}, фокус {focused}", self.query, self.results.len()),
            Message::PreviewSearchTab(step) => {
                // Вперёд по всем вкладкам и обратно: второй круг уже из кэша.
                const ORDER: [usize; 9] = [1, 2, 3, 4, 3, 2, 1, 0, 2];
                let Some(&tab) = ORDER.get(step - 1) else { return Task::none() };
                let next = Task::perform(background(|| std::thread::sleep(Duration::from_millis(1500))), move |()| Message::PreviewSearchTab(step + 1));
                return Task::batch([self.update(Message::SearchTabSet(SearchTab::ALL[tab])), next]);
            }
            Message::PreviewArtistEnd => return operation::snap_to(Id::new(LIST), scrollable::RelativeOffset::END),
            Message::PreviewQueueDown => {
                let down = operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y: 1200.0 });
                let press = Task::perform(background(|| std::thread::sleep(Duration::from_secs(1))), |()| Message::QueuePlay(25));
                return Task::batch([down, press]);
            }
            Message::QueueSource => {
                return match self.queue_from.clone() {
                    QueueFrom::Unknown => Task::none(),
                    QueueFrom::List(list) => self.open_list(list),
                    QueueFrom::Artist(id) => self.handle(Message::ArtistOpen(id)),
                    QueueFrom::Likes => self.handle(Message::Open(Section::Likes)),
                    QueueFrom::History => self.handle(Message::Open(Section::History)),
                    QueueFrom::Dislikes => self.handle(Message::Open(Section::Dislikes)),
                    QueueFrom::Search => self.handle(Message::Open(Section::Search)),
                    QueueFrom::Downloads => self.handle(Message::Open(Section::Downloads)),
                };
            }
            Message::QueueToggle => {
                if self.section == Section::Queue {
                    return self.handle(Message::Back);
                }
                return self.handle(Message::Open(Section::Queue));
            }
            Message::QueuePlay(index) => {
                self.skip_feedback();
                let moved = index.saturating_sub(self.current.unwrap_or(index));
                let task = self.play(index);
                let scroll = self.queue_scroll_start(moved);
                return Task::batch([task, scroll]);
            }
            Message::QueueRemove(index) => {
                self.menu = None;
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
                    self.disliked_list.retain(|t| t.id != track.id);
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
                let seed = format!("track:{}", track.id);
                let title = wave_label(&seed, &track.title);
                return self.update(Message::WaveStartSeeds(vec![seed], Some(title)));
            }
            Message::OpenAlbum(track) => {
                self.menu = None;
                if let Some(album) = &track.album {
                    return self.open_list(Playlist::album(album, &track.album_title));
                }
            }
            Message::CopyLink(track) => {
                self.menu = None;
                let notice = self.notify("Ссылка на трек скопирована".to_string());
                return Task::batch([iced::clipboard::write(track.share_link()), notice]);
            }
            Message::Unavailable => return self.notify(UNAVAILABLE.to_string()),
            Message::PageMenuOpen => self.page_menu = Some(*CURSOR.lock().unwrap()),
            Message::PageMenuPick(message) => {
                self.page_menu = None;
                return self.update(*message);
            }
            Message::Undo => {
                let Some((id, pending)) = self.pending.take() else { return Task::none() };
                if self.notice.as_ref().is_some_and(|(current, _, _)| *current == id) {
                    self.notice = None;
                }
                match pending {
                    Pending::Playlist(list, at) => {
                        let at = at.min(self.playlists.len());
                        self.playlists.insert(at, list);
                    }
                    Pending::Track(list, track, at) => {
                        if let Some(open) = self.playlist.as_mut().filter(|open| open.kind == list.kind && open.owner == list.owner) {
                            open.count += 1;
                            let at = at.min(self.playlist_tracks.len());
                            self.playlist_tracks.insert(at, track);
                        }
                    }
                }
            }
            Message::UndoExpire(id) => {
                if self.notice.as_ref().is_some_and(|(current, _, _)| *current == id) {
                    self.notice = None;
                }
                if self.pending.as_ref().is_some_and(|(current, _)| *current == id) {
                    return self.pending_commit();
                }
            }
            Message::CoverPick(list) => {
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let owner = self.hwnd;
                let kind = list.kind.clone();
                return Task::perform(
                    background(move || {
                        let Some(path) = pick::image(owner, "Обложка плейлиста") else { return Ok(None) };
                        let (bytes, mime) = api::cover_file(&path)?;
                        api::upload_cover(&uid, &kind, bytes, mime, &token).map(Some)
                    }),
                    move |result| Message::CoverUploaded(list.clone(), result),
                );
            }
            Message::CoverUploaded(_, Ok(None)) => {}
            Message::CoverUploaded(list, Ok(Some(uri))) => {
                // Старая картинка забывается: карточки возьмут новую по свежему списку плейлистов.
                let key = playlist_cover_key(&list);
                self.covers.remove(&key);
                self.requested.borrow_mut().remove(&key);
                if let Some(open) = self.playlist.as_mut().filter(|open| open.kind == list.kind && open.owner == list.owner) {
                    open.cover = uri;
                }
                self.playlists_loading = false;
                return Task::batch([self.notify("Обложка обновлена".to_string()), self.playlists_load()]);
            }
            Message::CoverUploaded(_, Err(e)) => return self.fail(e),
            Message::DragStart(list, index) => {
                self.drag = Some((list, index, None));
                DRAGGING.store(true, Ordering::Release);
            }
            Message::DragOver(index, y) => {
                if let Some((_, _, over)) = self.drag.as_mut() {
                    *over = Some((index, y > ROW / 2.0));
                }
            }
            Message::DragEnd => {
                DRAGGING.store(false, Ordering::Release);
                let Some((list, from, Some((at, below)))) = self.drag.take() else { return Task::none() };
                let Some(to) = drop_index(from, at, below) else { return Task::none() };
                return self.move_track(list, from, to);
            }
            Message::TrackMoved(Ok(())) => {}
            Message::TrackMoved(Err(e)) => {
                // Порядок на сервере не поменялся: список перечитывается, чтобы окно не врало.
                let reload = self.playlist.clone().map_or(Task::none(), |list| self.load_list(list));
                return Task::batch([self.fail(e), reload]);
            }
            Message::CrossfadeToggle => {
                self.crossfade = !self.crossfade;
                self.save_settings();
            }
            Message::Download(tracks) => {
                self.menu = None;
                let fresh: Vec<Track> = tracks
                    .into_iter()
                    .filter(|track| !track.unavailable && !self.download_ids.contains(&track.id) && self.download_now.as_deref() != Some(track.id.as_str()) && !self.download_queue.iter().any(|queued| queued.id == track.id))
                    .collect();
                if fresh.is_empty() {
                    return self.notify("Уже скачано".to_string());
                }
                let count = fresh.len();
                self.download_batch += count;
                self.download_queue.extend(fresh);
                let note = self.notify(if count == 1 { "Скачиваем трек".to_string() } else { format!("Скачиваем {count} {}", tracks_word(count)) });
                return Task::batch([note, self.download_next()]);
            }
            Message::Downloaded(id, result) => {
                self.download_now = None;
                match result {
                    Ok(saved) => {
                        self.downloads.retain(|old| old.track.id != saved.track.id);
                        self.downloads.insert(0, saved);
                        self.downloads_changed();
                        self.download_done += 1;
                    }
                    Err(e) => {
                        eprintln!("трек {id} не скачался: {e}");
                        self.download_failed += 1;
                    }
                }
                if !self.download_queue.is_empty() {
                    return self.download_next();
                }
                let (done, failed) = (std::mem::take(&mut self.download_done), std::mem::take(&mut self.download_failed));
                self.download_batch = 0;
                let note = match (done, failed) {
                    (_, 0) => format!("Скачано: {done} {}", tracks_word(done)),
                    (0, _) => "Не удалось скачать. Проверьте подключение к интернету".to_string(),
                    _ => format!("Скачано: {done} {}, не удалось: {failed}", tracks_word(done)),
                };
                return self.notify(note);
            }
            Message::DownloadRemove(ids) => {
                self.menu = None;
                let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.downloads).into_iter().partition(|saved| ids.contains(&saved.track.id));
                gone.iter().for_each(offline::remove);
                self.downloads = keep;
                self.download_queue.retain(|track| !ids.contains(&track.id));
                self.downloads_changed();
                return self.notify(if gone.len() == 1 { "Удалено из скачанного".to_string() } else { format!("Удалено из скачанного: {} {}", gone.len(), tracks_word(gone.len())) });
            }
            Message::DownloadStop => {
                let left = self.download_queue.len();
                self.download_queue.clear();
                self.download_batch = self.download_batch.saturating_sub(left);
                return self.notify("Скачивание остановлено: текущий трек докачается".to_string());
            }
            Message::DownloadLikes => {
                let Some(token) = self.token() else { return Task::none() };
                let ids = self.liked_ids.clone();
                // Сведения о треках просятся пачками: сотни номеров в одном запросе Яндекс не любит.
                return Task::perform(
                    background(move || {
                        let mut tracks = Vec::new();
                        for chunk in ids.chunks(LIKES_PAGE) {
                            tracks.extend(api::tracks_by_ids(chunk, &token)?);
                        }
                        Ok(tracks)
                    }),
                    Message::LikesForDownload,
                );
            }
            Message::LikesForDownload(Ok(tracks)) => return self.update(Message::Download(tracks)),
            Message::LikesForDownload(Err(e)) => return self.fail(e),
            Message::DownloadsClear => {
                self.download_queue.clear();
                std::mem::take(&mut self.downloads).iter().for_each(offline::remove);
                self.downloads_changed();
                return self.notify("Скачанное удалено".to_string());
            }
            Message::KidsToggle => {
                self.kids = !self.kids;
                self.save_settings();
            }
            Message::EqOpen => self.eq_menu = Some(eq_place(*CURSOR.lock().unwrap())),
            Message::EqToggle => {
                self.eq.enabled = !self.eq.enabled;
                self.eq_apply(true);
            }
            Message::EqPreset(preset) => {
                // Своя настройка хранит то, что выставлено руками, остальные берут числа пресета.
                if preset != equalizer::CUSTOM {
                    self.eq.gains = equalizer::PRESETS[preset].1;
                    self.eq.preamp = 1.0;
                }
                self.eq.preset = preset;
                self.eq.enabled = true;
                self.eq_apply(true);
            }
            Message::EqBand(band, gain) => {
                self.eq.gains[band] = (gain * 2.0).round() / 2.0;
                self.eq.preset = equalizer::CUSTOM;
                self.eq.enabled = true;
                self.eq_apply(false);
            }
            Message::EqPreamp(preamp) => {
                self.eq.preamp = (preamp * 20.0).round() / 20.0;
                self.eq.preset = equalizer::CUSTOM;
                self.eq.enabled = true;
                self.eq_apply(false);
            }
            Message::EqSave => self.save_settings(),
            Message::FullToggle => {
                self.player_full = !self.player_full && self.current_track().is_some();
                // Плеер выезжает уже в цвете обложки, без перехода от серого.
                if self.player_full && self.full_fade.item.is_none() {
                    self.full_tint = self.full_target();
                }
            }
            Message::Availability(Ok(found)) => {
                let found: HashMap<String, bool> = found.into_iter().collect();
                for track in &mut self.queue {
                    if let Some(&open) = found.get(&track.id) {
                        track.unavailable = !open;
                    }
                }
            }
            // Проверка не удалась: треки остаются как были.
            Message::Availability(Err(_)) => {}
            Message::NoticeClear(id) => {
                if self.notice.as_ref().is_some_and(|(current, _, _)| *current == id) {
                    self.notice = None;
                }
            }
            Message::WindowSize(size) => *WINDOW.lock().unwrap() = (size.width, size.height),
            Message::WindowReady(hwnd) => {
                self.hwnd = hwnd;
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
            Message::Cover(id, handle) => {
                // Крупная обложка заменяет уже видную мелкую, ей проявляться не нужно.
                if let Some(track) = id.strip_prefix(WAVE_BIG) {
                    if let Some(tint) = average_color(&handle) {
                        self.cover_tints.insert(track.to_string(), tint);
                    }
                } else {
                    self.cover_fades.insert(handle.id(), 0.0);
                }
                self.covers.insert(id, handle);
            }
            Message::Play(section, index) => {
                if section == Section::Wave {
                    // Выбрали трек в очереди волны: сессия продолжается.
                    self.skip_feedback();
                } else {
                    let tracks = self.list(section).to_vec();
                    let title = match section {
                        Section::Playlists => match (&self.playlist, &self.album) {
                            (Some(list), Some(album)) if list.title.is_empty() => album.title.clone(),
                            (Some(list), _) => list.title.clone(),
                            _ => "Плейлист".to_string(),
                        },
                        Section::Search => "Поиск".to_string(),
                        Section::Artist => self.artist.as_ref().map_or_else(String::new, |a| a.name.clone()),
                        Section::History => "История".to_string(),
                        Section::Dislikes => "Дизлайки".to_string(),
                        Section::Downloads => "Скачанное".to_string(),
                        _ => "Мне нравится".to_string(),
                    };
                    self.queue_from = match section {
                        Section::Playlists => self.playlist.clone().map_or(QueueFrom::Unknown, QueueFrom::List),
                        Section::Artist => self.artist.as_ref().map_or(QueueFrom::Unknown, |artist| QueueFrom::Artist(artist.id.clone())),
                        Section::Search => QueueFrom::Search,
                        Section::History => QueueFrom::History,
                        Section::Dislikes => QueueFrom::Dislikes,
                        Section::Downloads => QueueFrom::Downloads,
                        _ => QueueFrom::Likes,
                    };
                    return self.play_list(tracks, title, index);
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
                    Ok(sound) => {
                        self.sound = Some(sound);
                        self.active = true;
                        self.listen_tick = Some(Instant::now());
                        if let Some(listen) = self.listen.as_mut().filter(|listen| listen.started.is_none()) {
                            listen.started = Some(api::now_stamp());
                            let report = listen.report(0.0, None, true);
                            self.send_plays(vec![report]);
                        }
                        self.failures = 0;
                        self.save_session();
                        if let Some(place) = self.resume_seek.take() {
                            self.seeking = Some(place.as_secs_f32());
                            return self.update(Message::SeekDone);
                        }
                    }
                    Err(e) => {
                        self.unhold = false;
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
                self.listen_count();
                if let Some(p) = self.player.clone() {
                    p.toggle();
                    if p.is_paused() {
                        self.progress_save(false);
                    }
                    // Пауза у самого конца паузой не считается, как и в официальном клиенте.
                    if p.is_paused() && self.current_track().is_some_and(|t| t.duration.saturating_sub(p.position()) > Duration::from_millis(200)) {
                        if let Some(listen) = self.listen.as_mut() {
                            listen.pause = true;
                        }
                    }
                }
                self.save_session();
            }
            Message::Next => {
                if let Some(next) = self.next_index() {
                    self.skip_feedback();
                    self.change_reason = Some("skip");
                    return self.play(next);
                }
            }
            Message::Previous => {
                if self.active && (self.position >= PREVIOUS_RESTART || self.current == Some(0)) {
                    self.seeking = Some(0.0);
                    return self.update(Message::SeekDone);
                }
                if let Some(i) = self.current.and_then(|i| (0..i).rev().find(|&at| !self.queue[at].unavailable)) {
                    self.skip_feedback();
                    self.change_reason = Some("back_skip");
                    return self.play(i);
                }
            }
            Message::Tick => {
                if self.active && self.progress_at.is_none_or(|at| at.elapsed() >= PROGRESS_SAVE) && self.player.as_ref().is_some_and(|p| !p.is_paused()) {
                    self.progress_save(false);
                }
                self.preload_next();
                self.listen_count();
                if let Some(listen) = self.listen.as_ref().filter(|listen| listen.started.is_some() && listen.played >= self.listen_next) {
                    let report = listen.report(self.position.as_secs_f32(), Some("in_progress"), false);
                    self.listen_next += LISTEN_REPORT;
                    self.send_plays(vec![report]);
                }
                if let (true, Some(p)) = (self.active, &self.player) {
                    self.position = p.position();
                    if self.session_at.is_none_or(|at| at.elapsed() >= SESSION_SAVE) {
                        self.save_session();
                    }
                    if let Some(next) = self.cross_next() {
                        if let Some(track) = self.current_track().cloned() {
                            self.feedback("trackFinished", Some(&track), Some(self.position.as_secs_f32()));
                        }
                        self.change_reason = Some("finish");
                        self.cross_out = self.current_track().and_then(|track| track.fade).map(|fade| (fade.out_start, fade.out_stop));
                        return self.play(next);
                    }
                    let Some(p) = &self.player else { return Task::none() };
                    if p.is_empty() {
                        self.active = false;
                        if let Some(track) = self.current_track().cloned() {
                            self.feedback("trackFinished", Some(&track), Some(track.duration.as_secs_f32()));
                        }
                        self.change_reason = Some("finish");
                        if let (Repeat::One, false, Some(index)) = (self.repeat, self.wave_queue, self.current) {
                            return self.play(index);
                        }
                        if let Some(next) = self.next_index() {
                            return self.play(next);
                        }
                        self.listen_finish();
                    }
                }
            }
            Message::Seek(value) => self.seeking = Some(value),
            // Трек после перезапуска ещё не запущен: перемотка только запоминает место.
            Message::SeekDone if self.resume.is_some() => {
                if let Some(value) = self.seeking.take() {
                    let place = Duration::from_secs_f32(value.max(0.0));
                    self.position = place;
                    self.resume = Some(place);
                }
            }
            Message::SeekDone => {
                if let (Some(value), Some(p)) = (self.seeking.take(), self.player.clone()) {
                    let position = Duration::from_secs_f32(value.max(0.0));
                    self.position = position;
                    // Перемотка к месту прошлого запуска — не перемотка пользователя.
                    if !self.unhold {
                        if let Some(listen) = self.listen.as_mut() {
                            listen.seek = true;
                        }
                    }
                    return Task::perform(background(move || p.seek(position)), Message::Seeked);
                }
            }
            Message::Seeked(result) => {
                if std::mem::take(&mut self.unhold) {
                    if let Some(p) = self.player.as_ref().filter(|p| p.is_paused()) {
                        p.toggle();
                    }
                }
                if let Err(e) = result {
                    self.error = Some(e);
                }
            }
            Message::SeekBy(step) => {
                let Some(end) = self.current_track().map(|track| track.duration.as_secs_f32()).filter(|_| self.active || self.resume.is_some()) else { return Task::none() };
                // До первого запуска после перезапуска перемотка запоминает место: см. SeekDone.
                let from = self.seeking.unwrap_or(self.position.as_secs_f32());
                self.seeking = Some((from + step).clamp(0.0, (end - 1.0).max(0.0)));
                return self.update(Message::SeekDone);
            }
            Message::VolumeBy(step) => return self.handle(Message::Volume((self.volume + step).clamp(0.0, 1.0))),
            Message::LikeCurrent => {
                if let Some(track) = self.current_track().cloned() {
                    return self.handle(Message::Like(track));
                }
            }
            Message::DislikeCurrent => {
                if let Some(track) = self.current_track().cloned() {
                    return self.handle(Message::Dislike(track));
                }
            }
            // Кадры обрабатывает update.
            Message::Frame(_) => {}
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
        let ticks = match &self.screen {
            Screen::Main(_) if self.active => time::every(TICK).map(|_| Message::Tick),
            _ => Subscription::none(),
        };
        let frames = if self.animating() { window::frames().map(Message::Frame) } else { Subscription::none() };
        let online = if self.offline { time::every(ONLINE_CHECK).map(|_| Message::OnlineCheck) } else { Subscription::none() };
        Subscription::batch([ticks, frames, online]).pipe_media()
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
                .push(shine(button(text("Войти через Яндекс").font(ui::font_headline())).padding([10, 20]).style(ui::accent).on_press(Message::LoginOpen), 24.0));
        }
        if let Some(error) = &self.error {
            content = content.push(text(human_error(error)).color(ui::DANGER));
        }
        container(content).center(Length::Fill).style(ui::root).into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let page: Element<'_, Message> = match self.section {
            Section::Home | Section::Books | Section::Kids => self.view_home(),
            Section::Queue => self.view_queue(),
            Section::Lyrics => self.view_lyrics(),
            Section::Settings => self.view_settings(),
            Section::Discography => self.view_discography(),
            Section::Tag => self.view_tag(),
            Section::History => self.view_track_page("История", &self.history_tracks, "Здесь появятся треки, которые вы слушали", Section::History),
            Section::Dislikes => self.view_track_page("Дизлайки", &self.disliked_list, "Здесь появятся треки, которые вам не понравились", Section::Dislikes),
            Section::Downloads => self.view_track_page("Скачанное", &self.download_tracks, "Скачайте треки через меню «…» — они будут играть и без интернета", Section::Downloads),
            Section::Wave => self.view_wave(),
            Section::Collection => self.view_collection(),
            Section::Likes => self.view_likes(),
            Section::Playlists => self.view_playlists(),
            Section::Artist => self.view_artist(),
            Section::Albums => self.view_card_page("Альбомы", &self.liked_albums, ALBUM_COVER, false, "Здесь появятся альбомы, которые вам понравились"),
            Section::Artists => self.view_card_page("Исполнители", &self.liked_artists, ARTIST_COVER, true, "Здесь появятся исполнители, которые вам понравились"),
            Section::Search => self.view_search(),
        };
        // Вместо затухания самой страницы её на миг закрывает фон и плавно уходит.
        let rest = 1.0 - anim::ease_out(1.0 - self.page_veil);
        let back = ui::CONTENT;
        let page: Element<'_, Message> = reveal(page).shift(self.page_shift.0 * rest, self.page_shift.1 * rest).veil(back.scale_alpha(rest)).into();
        let mut page = column![page].spacing(12).height(Length::Fill);
        if self.offline {
            let mut line = row![ui::icon(ui::OFFLINE, 20.0, ui::MUTED), text("Нет подключения к интернету. Скачанное слушать можно, остальное загрузится, когда связь вернётся").color(ui::TEXT).width(Length::Fill)].spacing(12).align_y(Alignment::Center);
            if !self.downloads.is_empty() && self.section != Section::Downloads {
                line = line.push(shine(button(text("Скачанное").color(ui::TEXT)).padding([6, 14]).style(ui::secondary).on_press(Message::Open(Section::Downloads)), 24.0));
            }
            page = page.push(container(line).width(Length::Fill).padding([10, 16]).style(ui::tile));
        }
        if self.download_now.is_some() && matches!(self.section, Section::Downloads | Section::Likes | Section::Playlists) {
            let at = (self.download_done + self.download_failed + 1).min(self.download_batch.max(1));
            let mut line = row![ui::icon(ui::DOWNLOAD, 20.0, ui::ACCENT), text(format!("Скачиваем: {at} из {}", self.download_batch.max(at))).color(ui::TEXT).width(Length::Fill)].spacing(12).align_y(Alignment::Center);
            if !self.download_queue.is_empty() {
                line = line.push(shine(button(text("Остановить").color(ui::TEXT)).padding([6, 14]).style(ui::secondary).on_press(Message::DownloadStop), 24.0));
            }
            page = page.push(container(line).width(Length::Fill).padding([10, 16]).style(ui::tile));
        }
        if let Some(error) = &self.error {
            // Плашка с понятным текстом: подробности ошибки нужны только в журнале.
            let mut line = row![ui::icon(ui::INFO, 20.0, ui::DANGER), text(human_error(error)).color(ui::TEXT).width(Length::Fill)].spacing(12).align_y(Alignment::Center);
            if self.retry.is_some() {
                line = line.push(shine(button(text("Повторить").color(ui::TEXT)).padding([6, 14]).style(ui::secondary).on_press(Message::Retry), 24.0));
            }
            page = page.push(container(line).width(Length::Fill).padding([10, 16]).style(ui::tile));
        }
        let mut main = column![container(page).padding(24).width(Length::Fill).height(Length::Fill).style(ui::content).clip(true)].spacing(12);
        if let Some(bar) = self.view_bar() {
            main = main.push(bar);
        }
        let main: Element<'_, Message> = container(main).padding(Padding::ZERO.right(12).bottom(12)).width(Length::Fill).height(Length::Fill).into();
        let columns = row![self.view_sidebar(), main];
        let base = container(columns).width(Length::Fill).height(Length::Fill).style(ui::root);
        let mut layers = stack![base];
        if let Some(full) = self.view_full() {
            layers = layers.push(full);
        }
        if self.menu.is_some() || self.quality_menu.is_some() || self.eq_menu.is_some() || self.page_menu.is_some() || self.pin_menu.is_some() {
            layers = layers.push(mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill)).on_press(Message::MenuClose).on_right_press(Message::MenuClose));
        }
        if let Some((track, x, y, artists, lists)) = &self.menu_fade.item {
            layers = layers.push(self.view_menu(track, *x, *y, *artists, *lists));
        }
        if let Some((index, x, y)) = self.pin_fade.item {
            layers = layers.push(self.view_pin_menu(index, x, y));
        }
        if let Some((x, y)) = self.quality_fade.item {
            layers = layers.push(self.view_quality(x, y));
        }
        if let Some((x, y)) = self.eq_fade.item {
            layers = layers.push(self.view_eq_popup(x, y));
        }
        if let Some((x, y)) = self.page_menu_fade.item {
            layers = layers.push(self.view_page_menu(x, y));
        }
        if let Some(dialog) = &self.dialog_fade.item {
            layers = layers.push(self.view_dialog(dialog));
        }
        if let Some((notice, undo)) = &self.notice_fade.item {
            let alpha = self.notice_fade.shown();
            let mut line = row![text(human_error(notice)).color(ui::TEXT.scale_alpha(alpha))].spacing(16).align_y(Alignment::Center);
            if *undo {
                line = line.push(shine(button(text("Отменить").font(ui::font_medium()).color(ui::ACCENT.scale_alpha(alpha))).padding([4, 10]).style(ui::button_faded(ui::ghost, alpha)).on_press(Message::Undo), 16.0));
            }
            let padding = if *undo { Padding::from([8, 18]).right(8) } else { Padding::from([12, 18]) };
            let body = container(line).padding(padding).style(ui::popover_faded(alpha));
            layers = layers.push(
                container(reveal(body).shift(0.0, -16.0 * (1.0 - alpha)).scale(0.97 + 0.03 * alpha, 0.5, 0.0).inert(!(*undo && self.notice_fade.live)))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Start)
                    .padding(Padding::new(0.0).top(16)),
            );
        }
        smooth(layers)
    }

    /// Контекстное меню трека: те же пункты и названия, что в оригинале.
    fn view_dialog<'a>(&'a self, dialog: &'a Dialog) -> Element<'a, Message> {
        let (title, confirm, danger): (String, &str, bool) = match dialog {
            Dialog::WaveTune => return self.view_wave_tune(),
            Dialog::TrackInfo(track) => return self.view_track_info(track),
            Dialog::Account => return self.view_account(),
            Dialog::Logout => ("Выйти из аккаунта?".to_string(), "Выйти", true),
            Dialog::DownloadLikes => {
                let left = self.liked_ids.iter().filter(|id| !self.download_ids.contains(*id)).count();
                (format!("Скачать «Мне нравится»: {left} {}?", tracks_word(left)), "Скачать", false)
            }
            Dialog::ClearDownloads => (format!("Удалить всё скачанное ({})?", offline::size_label(self.downloads.iter().map(|saved| saved.bytes).sum())), "Удалить", true),
            Dialog::Create => ("Новый плейлист".to_string(), "Создать", false),
            Dialog::Rename(_) => ("Название плейлиста".to_string(), "Сохранить", false),
            Dialog::Describe(_) => ("Описание плейлиста".to_string(), "Сохранить", false),
            Dialog::Delete(list) => (format!("Удалить плейлист «{}»?", list.title), "Удалить", true),
            Dialog::RemoveTrack(track) => (format!("Удалить «{}» из плейлиста?", track.title), "Удалить", true),
        };
        let alpha = self.dialog_fade.shown();
        let mut body = column![text(title).size(20).font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha))].spacing(16);
        if !danger {
            body = body.push(text_input(if matches!(dialog, Dialog::Describe(_)) { "Описание" } else { "Название" }, &self.dialog_text).id(DIALOG_INPUT).on_input(Message::DialogInput).on_submit(Message::DialogConfirm).padding([10, 16]).style(ui::input_faded(alpha)));
        }
        let buttons = row![
            Space::new().width(Length::Fill),
            shine(button(text("Отмена").color(ui::TEXT.scale_alpha(alpha))).padding([8, 18]).style(ui::button_faded(ui::secondary, alpha)).on_press(Message::MenuClose), 24.0),
            shine(button(text(confirm).font(ui::font_medium()).color(ui::ON_ACCENT.scale_alpha(alpha))).padding([8, 18]).style(ui::button_faded(ui::accent, alpha)).on_press(Message::DialogConfirm), 24.0),
        ]
        .spacing(8);
        let window = reveal(container(body.push(buttons)).width(DIALOG_WIDTH).padding(24).style(ui::popover_faded(alpha))).scale(0.94 + 0.06 * alpha, 0.5, 0.5).shift(0.0, 10.0 * (1.0 - alpha));
        // Затемнение ловит клики, чтобы страница под окном не реагировала.
        let dim = ui::BLACK.scale_alpha(0.6 * alpha);
        reveal(mouse_area(container(mouse_area(window).on_press(Message::DialogInput(self.dialog_text.clone()))).center(Length::Fill).style(move |_| container::Style::default().background(dim))).on_press(Message::MenuClose)).inert(!self.dialog_fade.live).into()
    }

    fn avatar<'a>(&self, side: f32, letter: f32) -> Element<'a, Message> {
        if let Some(handle) = self.covers.get(AVATAR_COVER) {
            return image(handle.clone()).opacity(self.cover_alpha(handle)).width(side).height(side).border_radius(side / 2.0).into();
        }
        let first = self.profile.as_ref().and_then(|p| p.name.chars().next()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        container(text(first).size(letter).font(ui::font_medium())).center(side).style(skeleton_round(side / 2.0)).into()
    }

    /// Окно аккаунта. Изменить данные и подписку можно только на сайтах Яндекса.
    fn view_account(&self) -> Element<'_, Message> {
        let alpha = self.dialog_fade.shown();
        let Some(profile) = &self.profile else { return Space::new().into() };
        let tint = |color: iced::Color| color.scale_alpha(alpha);
        let close = glow(button(ui::icon(ui::CLOSE, 20.0, tint(ui::MUTED))).padding(4).style(ui::ghost).on_press(Message::MenuClose), 16.0);
        let head = row![text("Аккаунт").size(20).font(ui::font_medium()).color(tint(ui::TEXT)), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
        let mut who = column![fit(&profile.name).size(18).font(ui::font_medium()).color(tint(ui::TEXT)).no_tip()].spacing(2).width(Length::Fill).clip(true);
        for line in [profile.login.as_str(), profile.email.as_str()] {
            if !line.is_empty() {
                who = who.push(fit(line).size(14).color(tint(ui::MUTED)).no_tip());
            }
        }
        let plus = match (profile.plus, &profile.plus_until, profile.family) {
            (false, ..) => "Нет подписки Плюс".to_string(),
            (true, Some(until), true) => format!("Плюс по семейной подписке, до {until}"),
            (true, Some(until), false) => format!("Плюс до {until}"),
            (true, None, true) => "Плюс по семейной подписке".to_string(),
            (true, None, false) => "Подписка Плюс активна".to_string(),
        };
        let link = |label, target| shine(button(container(text(label).color(tint(ui::TEXT))).center_x(Length::Fill)).width(Length::Fill).padding([9, 18]).style(ui::button_faded(ui::secondary, alpha)).on_press(Message::OpenLink(target)), 24.0);
        let exit = shine(button(container(text("Выйти из аккаунта").color(tint(ui::TEXT))).center_x(Length::Fill)).width(Length::Fill).padding([9, 18]).style(ui::button_faded(ui::secondary, alpha)).on_press(Message::DialogOpen(Dialog::Logout)), 24.0);
        let body = column![
            head,
            row![self.avatar(ACCOUNT_AVATAR, 30.0), who].spacing(16).align_y(Alignment::Center),
            text(plus).color(tint(ui::TEXT)),
            column![link("Управление аккаунтом", ACCOUNT_LINK), link(if profile.plus { "Управление подпиской" } else { "Подключить Плюс" }, PLUS_LINK), exit].spacing(8),
        ]
        .spacing(16);
        let window = reveal(container(body).width(DIALOG_WIDTH).padding(24).style(ui::popover_faded(alpha))).scale(0.94 + 0.06 * alpha, 0.5, 0.5).shift(0.0, 10.0 * (1.0 - alpha));
        let dim = ui::BLACK.scale_alpha(0.6 * alpha);
        reveal(mouse_area(container(mouse_area(window).on_press(Message::DialogInput(self.dialog_text.clone()))).center(Length::Fill).style(move |_| container::Style::default().background(dim))).on_press(Message::MenuClose)).inert(!self.dialog_fade.live).into()
    }

    /// Меню закрепа: открепить и перейти на страницу, если она есть.
    fn view_pin_menu(&self, index: usize, x: f32, y: f32) -> Element<'_, Message> {
        let Some(pin) = self.pins.get(index) else { return Space::new().into() };
        let (alpha, live) = (self.pin_fade.shown(), self.pin_fade.live);
        let item = |icon, label, message| {
            glow(button(row![ui::icon(icon, 20.0, ui::MUTED.scale_alpha(alpha)), text(label).color(ui::TEXT.scale_alpha(alpha))].spacing(12).align_y(Alignment::Center)).width(Length::Fill).height(MENU_ITEM).padding([9, 10]).style(ui::menu_item).on_press(message), 6.0)
        };
        let mut items = column![];
        let mut count = 1.0;
        if let PinTarget::List(list) = &pin.target {
            let (icon, label) = match list.source {
                Source::Album => (ui::ALBUM, "Перейти к альбому"),
                Source::Artist => (ui::ARTIST, "Перейти к исполнителю"),
                Source::Playlist => (ui::QUEUE, "Перейти к плейлисту"),
            };
            items = items.push(item(icon, label, Message::PinPage(index)));
            count += 1.0;
        }
        items = items.push(item(ui::PIN, "Открепить", Message::Pin(pin.reference())));
        popup(items.into(), x, y, MENU_WIDTH, count * MENU_ITEM, alpha, live)
    }

    /// Окно со сведениями о треке.
    fn view_track_info<'a>(&'a self, track: &'a Track) -> Element<'a, Message> {
        let alpha = self.dialog_fade.shown();
        let close = glow(button(ui::icon(ui::CLOSE, 20.0, ui::MUTED.scale_alpha(alpha))).padding(4).style(ui::ghost).on_press(Message::MenuClose), 16.0);
        let head = row![text("Сведения о треке").size(20).font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha)), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
        let mut body = column![].spacing(14).width(Length::Fill).padding(Padding::ZERO.right(SCROLLBAR));
        let mut rows: Vec<(&str, &str)> = vec![("Название", track.title.as_str())];
        if !track.album_title.is_empty() {
            rows.push(("Альбом", track.album_title.as_str()));
        }
        let duration = clock(track.duration);
        rows.push(("Длительность", duration.as_str()));
        let details = self.track_info.as_ref().filter(|(id, _)| *id == track.id).map(|(_, details)| details);
        rows.extend(details.into_iter().flat_map(|details| details.facts.iter().map(|(title, value)| (title.as_str(), value.as_str()))));
        for (title, value) in rows {
            body = body.push(column![text(title.to_string()).size(13).color(ui::MUTED.scale_alpha(alpha)), text(value.to_string()).color(ui::TEXT.scale_alpha(alpha))].spacing(2));
        }
        match details {
            None => body = body.push(text("Загружаем сведения…").color(ui::MUTED.scale_alpha(alpha))),
            Some(details) if !details.similar.is_empty() => {
                let similar = details.similar.iter().enumerate().fold(column![].spacing(2), |list, (index, similar)| {
                    let line = row![
                        ui::icon(ui::PLAY, 18.0, ui::MUTED.scale_alpha(alpha)),
                        column![
                            fit(&similar.title).color(ui::TEXT.scale_alpha(alpha)).no_tip(),
                            fit(&similar.artists).size(13).color(ui::MUTED.scale_alpha(alpha)).no_tip(),
                        ]
                        .width(Length::Fill)
                        .clip(true),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center);
                    list.push(glow(button(line).width(Length::Fill).padding([6, 8]).style(ui::row).on_press(Message::TrackInfoPlay(index)), 8.0))
                });
                body = body.push(column![text("Похожие треки").size(16).font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha)), similar].spacing(6));
            }
            Some(_) => {}
        }
        // Голова стоит первой: прокрутка занимает остаток, сколько бы ни было авторов.
        let content = column![head, ui::scroll(body).width(Length::Fill).style(ui::scroll_still)].spacing(14);
        let tall = WINDOW.lock().unwrap().1 - INFO_MARGIN;
        let window = reveal(container(content).width(DIALOG_WIDTH).max_height(tall).padding(24).style(ui::popover_faded(alpha))).scale(0.94 + 0.06 * alpha, 0.5, 0.5).shift(0.0, 10.0 * (1.0 - alpha));
        let dim = ui::BLACK.scale_alpha(0.6 * alpha);
        reveal(mouse_area(container(mouse_area(window).on_press(Message::DialogInput(self.dialog_text.clone()))).center(Length::Fill).style(move |_| container::Style::default().background(dim))).on_press(Message::MenuClose)).inert(!self.dialog_fade.live).into()
    }

    /// Настройки волны: каждый выбор сразу запускает волну заново.
    fn view_wave_tune(&self) -> Element<'_, Message> {
        let alpha = self.dialog_fade.shown();
        let mut body = column![text("Настроить волну").size(20).font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha))].spacing(16);
        for (group, (title, options)) in settings::WAVE_GROUPS.iter().enumerate() {
            let chips = options.iter().enumerate().fold(row![].spacing(8), |chips, (choice, (name, _))| {
                let active = self.wave_tuning[group] == choice;
                chips.push(shine(button(text(*name).wrapping(Wrapping::None)).padding([7, 14]).style(ui::chip_faded(active, alpha)).on_press(Message::WaveTune(group, choice)), 18.0))
            });
            body = body.push(column![text(*title).size(14).color(ui::MUTED.scale_alpha(alpha)), chips.wrap().vertical_spacing(8)].spacing(8));
        }
        let buttons = row![
            shine(button(text("Сбросить").color(ui::TEXT.scale_alpha(alpha))).padding([8, 18]).style(ui::button_faded(ui::secondary, alpha)).on_press(Message::WaveTuneReset), 24.0),
            Space::new().width(Length::Fill),
            shine(button(text("Готово").font(ui::font_medium()).color(ui::ON_ACCENT.scale_alpha(alpha))).padding([8, 18]).style(ui::button_faded(ui::accent, alpha)).on_press(Message::MenuClose), 24.0),
        ];
        let window = reveal(container(body.push(buttons)).width(TUNE_WIDTH).padding(24).style(ui::popover_faded(alpha))).scale(0.94 + 0.06 * alpha, 0.5, 0.5).shift(0.0, 10.0 * (1.0 - alpha));
        let dim = ui::BLACK.scale_alpha(0.6 * alpha);
        reveal(mouse_area(container(mouse_area(window).on_press(Message::DialogInput(self.dialog_text.clone()))).center(Length::Fill).style(move |_| container::Style::default().background(dim))).on_press(Message::MenuClose)).inert(!self.dialog_fade.live).into()
    }

    /// Текст песни: звучащая строка стоит посередине, клик по строке перематывает.
    fn view_lyrics(&self) -> Element<'_, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let Some(track) = self.current_track() else {
            return column![back, placeholder("Сейчас ничего не играет", None)].spacing(16).height(Length::Fill).into();
        };
        let about = column![fit(&track.title).size(24).font(ui::font_headline()).color(ui::TEXT), fit(&track.artists).color(ui::MUTED)];
        let about = about.align_x(Alignment::Center);
        // Пустое место справа равно кнопке слева, чтобы название стояло точно посередине.
        let header = row![back, container(about).center_x(Length::Fill).clip(true), Space::new().width(36)].spacing(12).align_y(Alignment::Center);
        let found = self.lyrics.as_ref().filter(|(id, _)| *id == track.id);
        let Some(lyrics) = found.and_then(|(_, lyrics)| lyrics.as_ref()) else {
            let label = if found.is_some() { "У этого трека нет текста" } else { "Загружаем текст…" };
            return column![header, placeholder(label, None)].spacing(16).height(Length::Fill).into();
        };
        // Запас сверху и снизу на половину страницы: первая и последняя строки тоже встают посередине.
        let room = ((WINDOW.lock().unwrap().1 - LYRICS_CHROME) / 2.0).max(0.0);
        let mut lines = column![Space::new().height(if lyrics.synced() { room } else { 0.0 })].spacing(14).width(Length::Fill).align_x(Alignment::Center);
        for (index, (time, words)) in lyrics.lines.iter().enumerate() {
            let line = text(words).size(LYRICS_SIZE).font(ui::font_headline()).width(Length::Fill).align_x(Alignment::Center);
            let line: Element<'_, Message> = match time {
                Some(time) => {
                    // Всё считается от дробного положения фокуса, поэтому переход идёт сам.
                    let distance = (index as f32 - self.lyrics_shown).abs();
                    let focus = (1.0 - distance).clamp(0.0, 1.0);
                    let dim = (LYRICS_DIM_NEAR - LYRICS_DIM_STEP * (distance - 1.0).max(0.0)).max(LYRICS_DIM_FAR);
                    let color = ui::WHITE.scale_alpha(dim + (1.0 - dim) * focus);
                    let line = hover_line(container(line).width(Length::Fill).padding([2, 24]), color, ui::WHITE.scale_alpha(color.a.max(LYRICS_HOVER)), Message::LyricsSeek(*time));
                    reveal(line).scale(1.0 + LYRICS_GROW * focus, 0.5, 0.5).into()
                }
                None => container(line.color(ui::TEXT)).padding([0, 24]).into(),
            };
            lines = lines.push(line);
        }
        if !lyrics.authors.is_empty() {
            lines = lines.push(container(text(format!("Авторы: {}", lyrics.authors.join(", "))).size(14).color(ui::MUTED).width(Length::Fill).align_x(Alignment::Center)).padding([0, 24]));
        }
        lines = lines.push(Space::new().height(if lyrics.synced() { room } else { 0.0 }));
        let hidden = scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(0).scroller_width(0));
        column![header, ui::scroll(lines).id(LIST).direction(hidden).on_scroll(|viewport| Message::LyricsScrolled(viewport.absolute_offset().y, viewport.relative_offset().y)).height(Length::Fill)].spacing(16).height(Length::Fill).into()
    }

    /// Запрашивает текст, когда открыта его страница и сменился трек.
    fn lyrics_sync(&mut self) -> Task<Message> {
        if self.section != Section::Lyrics {
            return Task::none();
        }
        let (Some(track), Some(token)) = (self.current_track(), self.token()) else { return Task::none() };
        let id = track.id.clone();
        if self.lyrics.as_ref().is_some_and(|(known, _)| *known == id) || self.lyrics_loading.as_ref() == Some(&id) {
            return Task::none();
        }
        self.lyrics_loading = Some(id.clone());
        self.lyrics_line = None;
        let wanted = id.clone();
        Task::perform(background(move || api::lyrics(&id, &token)), move |result| Message::LyricsLoaded(wanted.clone(), result))
    }

    /// Находит звучащую строку и задаёт, куда прокрутить страницу.
    /// Куда встать, чтобы фокус был посередине.
    fn lyrics_place(&self) -> f32 {
        let count = self.lyrics.as_ref().and_then(|(_, lyrics)| lyrics.as_ref()).map_or(1, |lyrics| lyrics.lines.len().max(1));
        ((self.lyrics_shown + 0.5) / count as f32).clamp(0.0, 1.0)
    }

    fn lyrics_follow(&mut self) {
        if self.section != Section::Lyrics {
            return;
        }
        let Some((_, Some(lyrics))) = self.lyrics.as_ref().filter(|(id, _)| self.current_track().is_some_and(|t| t.id == *id)) else { return };
        let line = lyrics.line_at(self.position.as_secs_f32());
        if line != self.lyrics_line {
            self.lyrics_target = line.map_or(-1.0, |index| index as f32);
            self.lyrics_line = line;
            // Текст могли прокрутить: возвращаемся к строке от того места, где он стоит.
            self.lyrics_gap = self.lyrics_view - self.lyrics_place();
            self.lyrics_back = 0.0;
        }
        if self.lyrics_fresh {
            self.lyrics_shown = self.lyrics_target;
            self.lyrics_back = 1.0;
            self.lyrics_fresh = false;
        }
    }

    fn view_menu<'a>(&'a self, track: &'a Track, x: f32, y: f32, artists: bool, lists: bool) -> Element<'a, Message> {
        let (alpha, live) = (self.menu_fade.shown(), self.menu_fade.live);
        let item = |icon, label: &'a str, message: Message| {
            glow(button(row![ui::icon(icon, 20.0, ui::MUTED.scale_alpha(alpha)), text(label).color(ui::TEXT.scale_alpha(alpha))].spacing(12).align_y(Alignment::Center))
                .width(Length::Fill)
                .height(MENU_ITEM)
                .padding([9, 10])
                .style(ui::menu_item)
                .on_press(message), 6.0)
        };
        if artists {
            let items = track.artist_list.iter().fold(column![], |items, (name, id)| items.push(item(ui::ARTIST, name.as_str(), artist_message(name, id))));
            return popup(items.into(), x, y, MENU_WIDTH, track.artist_list.len() as f32 * MENU_ITEM, alpha, live);
        }
        if lists {
            let own: Vec<&'a Playlist> = self.playlists.iter().filter(|list| self.is_own(list)).collect();
            if own.is_empty() {
                let label = if self.playlists_loading { "Загружаем…" } else { "Сначала создайте плейлист" };
                return popup(container(text(label).color(ui::MUTED.scale_alpha(alpha))).padding([9, 10]).into(), x, y, MENU_WIDTH, MENU_ITEM, alpha, live);
            }
            let shown = own.len().min(MENU_LISTS) as f32 * MENU_ITEM;
            let items = own.iter().fold(column![], |items, list| items.push(item(ui::QUEUE, list.title.as_str(), Message::TrackAdd(track.clone(), (*list).clone()))));
            return popup(ui::scroll(items).height(shown).style(ui::scroll_still).into(), x, y, MENU_WIDTH, shown, alpha, live);
        }
        let liked = self.liked_set.contains(&track.id);
        let disliked = self.disliked.contains(&track.id);
        let mut items = column![
            item(if liked { ui::HEART } else { ui::HEART_OUTLINE }, if liked { "Убрать из «Мне нравится»" } else { "Нравится" }, Message::Like(track.clone())),
            item(ui::DISLIKE, if disliked { "Убрать дизлайк" } else { "Не нравится" }, Message::Dislike(track.clone())),
            item(ui::WAVE, "Моя волна по треку", Message::TrackWave(track.clone())),
            item(ui::QUEUE_NEXT, "Играть следующим", Message::PlayNext(track.clone())),
            item(ui::QUEUE_LAST, "Добавить в конец очереди", Message::PlayLast(track.clone())),
            item(ui::PLUS, "Добавить в плейлист", Message::MenuLists),
        ];
        let mut count = 6.0;
        if self.section == Section::Playlists && self.playlist.as_ref().is_some_and(|list| self.is_own(list)) {
            items = items.push(item(ui::DELETE, "Удалить из плейлиста", Message::DialogOpen(Dialog::RemoveTrack(track.clone()))));
            count += 1.0;
        }
        if track.album.is_some() {
            items = items.push(item(ui::ALBUM, "Перейти к альбому", Message::OpenAlbum(track.clone())));
            count += 1.0;
        }
        if let Some((name, id)) = track.artist_list.first() {
            items = items.push(if track.artist_list.len() > 1 {
                item(ui::ARTIST, "Перейти к исполнителям", Message::MenuArtists)
            } else {
                item(ui::ARTIST, "Перейти к исполнителю", artist_message(name, id))
            });
            count += 1.0;
        }
        if self.section == Section::Queue {
            if let Some(index) = self.queue.iter().enumerate().position(|(index, queued)| queued.id == track.id && self.current != Some(index)) {
                items = items.push(item(ui::CLOSE, "Убрать из очереди", Message::QueueRemove(index)));
                count += 1.0;
            }
        }
        // Страница текста показывает только играющий трек.
        if track.lyrics && self.section != Section::Lyrics && self.current_track().is_some_and(|current| current.id == track.id) {
            items = items.push(item(ui::LYRICS, "Текст песни", Message::MenuLyrics));
            count += 1.0;
        }
        if track.remember {
            let finished = self.listened(track);
            items = items.push(item(ui::CHECK, if finished { "Отметить непрослушанным" } else { "Отметить прослушанным" }, Message::EpisodeFinished(track.clone(), !finished)));
            count += 1.0;
        }
        if !track.unavailable {
            let saved = self.download_ids.contains(&track.id);
            items = items.push(if saved {
                item(ui::DELETE, "Удалить из скачанного", Message::DownloadRemove(vec![track.id.clone()]))
            } else {
                item(ui::DOWNLOAD, "Скачать", Message::Download(vec![track.clone()]))
            });
            count += 1.0;
        }
        items = items.push(item(ui::LINK, "Скопировать ссылку", Message::CopyLink(track.clone())));
        items = items.push(item(ui::INFO, "Сведения о треке", Message::DialogOpen(Dialog::TrackInfo(track.clone()))));
        count += 2.0;
        popup(items.into(), x, y, MENU_WIDTH, count * MENU_ITEM, alpha, live)
    }

    /// Выбор качества. Новое качество действует со следующего трека.
    fn view_quality(&self, x: f32, y: f32) -> Element<'_, Message> {
        let (alpha, live) = (self.quality_fade.shown(), self.quality_fade.live);
        let mut items = column![container(text("Качество звука").font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha))).padding([6, 10])];
        for quality in Quality::ALL {
            let mark: Element<'_, Message> = if quality == self.quality { ui::icon(ui::CHECK, 20.0, ui::ACCENT.scale_alpha(alpha)) } else { Space::new().width(20).into() };
            items = items.push(
                glow(button(row![mark, text(quality.label()).color(ui::TEXT.scale_alpha(alpha))].spacing(12).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .height(MENU_ITEM)
                    .padding([9, 10])
                    .style(ui::menu_item)
                    .on_press(Message::QualitySet(quality)), 6.0),
            );
        }
        if let Some(sound) = &self.sound {
            items = items.push(container(text(format!("Сейчас играет: {}", sound.label)).size(13).color(ui::MUTED.scale_alpha(alpha))).padding([6, 10]));
        }
        popup(items.into(), x, y, QUALITY_WIDTH, 5.0 * MENU_ITEM, alpha, live)
    }

    /// Страница очереди: что играет сейчас и что будет дальше.
    fn view_queue(&self) -> Element<'_, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let Some((current, track)) = self.current.zip(self.current_track()) else {
            return column![back, placeholder("Очередь пуста", Some("Включите трек, альбом или плейлист"))].spacing(16).height(Length::Fill).into();
        };
        let title = row![back, text("Сейчас играет").size(36).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)].spacing(12).align_y(Alignment::Center);
        let title = column![title, container(self.view_queue_source()).padding(Padding::ZERO.left(QUEUE_SOURCE_INSET)).width(Length::Fill).clip(true)].spacing(4);
        let next = text("Далее в очереди").size(24).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None);
        let header = container(column![title, self.view_queue_row(current, track), next].spacing(12)).height(QUEUE_HEADER);
        let total = self.row_count(Section::Queue);
        let range = self.visible();
        let mut list = column![header, Space::new().height(range.start as f32 * ROW)].padding(Padding::ZERO.right(SCROLLBAR));
        for row_index in range.clone() {
            let index = current + 1 + row_index;
            list = list.push(self.view_queue_row(index, &self.queue[index]));
        }
        list = list.push(Space::new().height((total - range.end) as f32 * ROW));
        ui::scroll(list).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).height(Length::Fill).into()
    }

    /// Включили трек ниже: строки над ним уходят из списка. Оставшиеся сначала стоят на своих
    /// местах, чтобы ничего не прыгнуло, а потом список плавно едет к началу.
    fn queue_scroll_start(&mut self, moved: usize) -> Task<Message> {
        let from = (self.list_offset - moved as f32 * ROW).max(0.0);
        self.list_offset = from;
        self.queue_scroll = Some((from, 0.0));
        operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y: from })
    }

    fn queue_scroll_step(&mut self, elapsed: f32) -> Task<Message> {
        let Some((from, passed)) = self.queue_scroll else { return Task::none() };
        let passed = (passed + elapsed / QUEUE_SCROLL).min(1.0);
        self.queue_scroll = (passed < 1.0).then_some((from, passed));
        let y = from * (1.0 - smooth_step(passed));
        self.list_offset = y;
        operation::scroll_to(Id::new(LIST), scrollable::AbsoluteOffset { x: 0.0, y })
    }

    /// Откуда играет очередь. Название ведёт обратно к источнику.
    fn view_queue_source<'a>(&'a self) -> Element<'a, Message> {
        let name = if self.wave_queue { self.wave_title.as_deref().unwrap_or("Моя волна") } else { self.queue_title.as_deref().unwrap_or_default() };
        let name = clipped(name, QUEUE_SOURCE_LABEL);
        let plain = |label: String| text(label).size(QUEUE_SOURCE_SIZE).color(ui::MUTED).wrapping(Wrapping::None);
        let link = |label: String| hover_line(text(label).size(QUEUE_SOURCE_SIZE).wrapping(Wrapping::None), ui::MUTED, ui::WHITE, Message::QueueSource);
        let prefix = match &self.queue_from {
            _ if self.wave_queue => return plain(name).into(),
            QueueFrom::Unknown => return plain(name).into(),
            QueueFrom::Search => return link("из поиска".to_string()).into(),
            QueueFrom::History => return link("из истории".to_string()).into(),
            QueueFrom::Downloads => return link("из скачанного".to_string()).into(),
            QueueFrom::List(list) if list.source == Source::Album => "из альбома",
            QueueFrom::List(_) | QueueFrom::Likes | QueueFrom::Dislikes => "из плейлиста",
            QueueFrom::Artist(_) => "из популярных треков исполнителя",
        };
        row![plain(prefix.to_string()), link(name)].spacing(5).into()
    }

    fn view_queue_row<'a>(&'a self, index: usize, track: &'a Track) -> Element<'a, Message> {
        let current = self.current == Some(index);
        let cover: Element<'a, Message> = if current {
            let icon = if self.is_playing() { ui::PAUSE } else { ui::PLAY };
            stack![self.cover(track, COVER), container(ui::icon(icon, 22.0, ui::WHITE)).center(COVER).style(ui::overlay)].into()
        } else {
            self.cover(track, COVER)
        };
        let (title_color, names_color) = match (current, track.unavailable) {
            (true, _) => (ui::ACCENT, ui::MUTED),
            (false, true) => (ui::MUTED, FAINT),
            (false, false) => (ui::WHITE, ui::MUTED),
        };
        let names: Element<'a, Message> = if let Some(published) = track.published.as_ref().filter(|_| track.remember) {
            fit(published).color(names_color).into()
        } else if track.artist_list.is_empty() {
            fit(&track.artists).color(names_color).into()
        } else {
            artist_links(&track.artist_list, 16.0, ui::font(), names_color)
        };
        let names = self.download_mark(track, names);
        let mut line = row![
            cover,
            column![track_title(track, 16.0, ui::font_medium(), title_color, false), names].width(Length::Fill).clip(true),
            self.like_button(track, 20.0),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        if self.main_width() >= TRACK_TIME_WIDTH {
            let time_color = if track.duration.is_zero() { iced::Color::TRANSPARENT } else { names_color };
            line = line.push(text(clock(track.duration)).color(time_color));
        }
        line = line.push(self.more_button(track, 18.0));
        if !current {
            line = line.push(glow(button(ui::icon(ui::CLOSE, 16.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(Message::QueueRemove(index)), 20.0));
            line = line.push(self.grip(DragList::Queue, index));
        }
        let press = match (current, track.unavailable) {
            (true, _) => Message::Toggle,
            (false, true) => Message::Unavailable,
            (false, false) => Message::QueuePlay(index),
        };
        let row = mouse_area(glow(button(line).width(Length::Fill).height(ROW).padding([8, 8]).style(ui::row).on_press(press), 8.0)).on_right_press(Message::MenuOpen(track.clone())).into();
        if current { row } else { self.drag_target(row, DragList::Queue, index) }
    }

    /// Эквалайзер, открытый из трекбара: тот же, что в настройках, пресеты списком.
    fn view_eq_popup(&self, x: f32, y: f32) -> Element<'_, Message> {
        let (alpha, live) = (self.eq_fade.shown(), self.eq_fade.live);
        let eq = self.eq;
        let head = row![text("Эквалайзер").size(16).font(ui::font_medium()).color(ui::TEXT.scale_alpha(alpha)).width(Length::Fill), view_switch(Some(eq.enabled))].align_y(Alignment::Center);
        let head = glow(button(head).width(Length::Fill).padding([6, 8]).style(ui::button_faded(ui::row, alpha)).on_press(Message::EqToggle), 8.0);
        let presets: Vec<Preset> = (0..equalizer::PRESETS.len()).map(Preset).collect();
        let select = pick_list(presets, Some(Preset(eq.preset)), |preset: Preset| Message::EqPreset(preset.0))
            .width(Length::Fill)
            .padding([7, 12])
            .text_size(14)
            .style(ui::select_faded(alpha))
            .menu_style(ui::select_menu);
        let body = column![head, container(select).padding([0, 8]), self.eq_bands(EQ_POPUP_SLIDER, alpha)].spacing(12);
        popup(body.into(), x, y, EQ_POPUP_WIDTH, EQ_POPUP_HEIGHT, alpha, live)
    }

    /// Ползунки эквалайзера: уровень и десять полос с подписями.
    fn eq_bands(&self, height: f32, alpha: f32) -> Element<'static, Message> {
        let eq = self.eq;
        let tint = if eq.enabled { ui::TEXT } else { ui::MUTED }.scale_alpha(alpha);
        let value_tint = ui::MUTED.scale_alpha(alpha);
        let column_of = |value: String, slider: Element<'static, Message>, label: String| column![text(value).size(12).color(value_tint), slider, text(label).size(12).color(tint)].spacing(6).align_x(Alignment::Center).width(Length::Fill);
        let preamp = vertical_slider(equalizer::PREAMP_MIN..=equalizer::PREAMP_MAX, eq.preamp, Message::EqPreamp).step(0.05).height(height).on_release(Message::EqSave).style(ui::eq_slider(eq.enabled, alpha));
        let mut bands = row![column_of(format!("{:.2}", eq.preamp), preamp.into(), "уровень".to_string())].spacing(4);
        for (band, frequency) in equalizer::BANDS.iter().enumerate() {
            let slider = vertical_slider(-equalizer::GAIN_LIMIT..=equalizer::GAIN_LIMIT, eq.gains[band], move |gain| Message::EqBand(band, gain)).step(0.5).height(height).on_release(Message::EqSave).style(ui::eq_slider(eq.enabled, alpha));
            let label = if *frequency < 1000.0 { format!("{frequency}") } else { format!("{}K", frequency / 1000.0) };
            bands = bands.push(column_of(format!("{:+}", eq.gains[band]), slider.into(), label));
        }
        container(bands).padding([0, 8]).into()
    }

    /// Качество и очередь: кнопки рядом с громкостью.
    fn view_tools<'a>(&self, size: f32, quality: bool) -> Element<'a, Message> {
        let tool = |icon, on: bool, message| glow(button(ui::icon(icon, size, if on { ui::ACCENT } else { ui::MUTED })).padding(4).style(ui::ghost).on_press(message), 20.0);
        let mut tools = row![].spacing(6).align_y(Alignment::Center);
        if quality {
            tools = tools.push(tool(ui::EQUALIZER, self.eq.enabled, Message::EqOpen));
            tools = tools.push(tool(ui::TUNE, self.quality == Quality::Lossless, Message::QualityOpen));
        }
        if self.current_track().is_some_and(|track| track.lyrics) {
            tools = tools.push(tool(ui::LYRICS, self.section == Section::Lyrics, Message::LyricsToggle));
        }
        tools.push(tool(ui::QUEUE, self.section == Section::Queue, Message::QueueToggle)).into()
    }

    fn more_button<'a>(&self, track: &Track, size: f32) -> Element<'a, Message> {
        glow(button(container(ui::icon(ui::MORE, size, ui::MUTED)).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::MenuOpen(track.clone())), 20.0).into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let opened = match self.section {
            Section::Likes | Section::Playlists | Section::Albums | Section::Artists | Section::History | Section::Dislikes | Section::Downloads => Section::Collection,
            section => section,
        };
        let item = |icon, label, section| {
            let tint = if opened == section { ui::TEXT } else { ui::MUTED };
            glow(
                // Подписи в одну строку: «Книги и подкасты» иначе переносится.
                button(row![ui::icon(icon, 24.0, tint), text(label).color(tint).wrapping(Wrapping::None)].spacing(12).align_y(Alignment::Center))
                    .width(Length::Fill)
                    .padding(Padding::new(12.0).right(12))
                    .style(ui::nav)
                    .on_press(Message::Open(section)),
                24.0,
            )
        };
        let mut account = column![fit(self.profile.as_ref().map_or("", |p| p.name.as_str())).size(14).font(ui::font_medium()).color(ui::TEXT)].clip(true).width(Length::Fill);
        if self.profile.as_ref().is_some_and(|p| p.plus) {
            account = account.push(container(text("Плюс").size(12).font(ui::font_medium())).padding([1, 8]).style(ui::plus));
        }
        let who = glow(button(row![self.avatar(AVATAR_SIZE, 18.0), account].spacing(10).align_y(Alignment::Center)).width(Length::Fill).padding(4).style(ui::row).on_press(Message::DialogOpen(Dialog::Account)), 8.0);
        let gear = if self.section == Section::Settings { ui::TEXT } else { ui::MUTED };
        let profile = row![who, glow(button(ui::icon(ui::SETTINGS, 18.0, gear)).padding(6).style(ui::ghost).on_press(Message::Open(Section::Settings)), 20.0), glow(button(ui::icon(ui::LOGOUT, 18.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(Message::DialogOpen(Dialog::Logout)), 20.0)].spacing(4).align_y(Alignment::Center);
        container(
            column![
                container(text("Tune").size(26).font(ui::font_headline()).color(ui::ACCENT)).padding(Padding::new(0.0).left(12)),
                Space::new().height(8),
                column![
                    item(ui::HOME, "Главная", Section::Home),
                    item(ui::SEARCH, "Поиск", Section::Search),
                    item(ui::WAVE, "Моя волна", Section::Wave),
                    item(ui::HEADPHONES, "Книги и подкасты", Section::Books),
                ]
                .extend(self.kids.then(|| item(ui::KIDS, "Детям", Section::Kids)))
                .push(item(ui::HEART_OUTLINE, "Коллекция", Section::Collection))
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
                let cover: Element<'_, Message> = match self.covers.get(&format!("{PIN_COVER_PREFIX}{}", pin.key())) {
                    Some(handle) => {
                        let picture = image(handle.clone()).opacity(self.cover_alpha(handle)).width(40).height(40).border_radius(radius);
                        match pin.color.as_deref().and_then(ui::hex_color) {
                            Some(color) => container(picture).style(move |_| container::Style::default().background(color).border(iced::border::rounded(radius))).into(),
                            None => picture.into(),
                        }
                    }
                    None => container(Space::new()).width(40).height(40).style(if pin.round { ui::avatar } else { ui::tile }).into(),
                };
                let mut line = row![
                    cover,
                    column![
                        fit(&pin.title).size(13).font(ui::font_medium()).color(if self.pin_active(pin) { ui::ACCENT } else { ui::TEXT }),
                        fit(&pin.subtitle).size(12).color(ui::MUTED),
                    ]
                    .width(Length::Fill)
                    .clip(true),
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                // У Моей волны нет ни страницы, ни открепления, так что и меню не нужно.
                let menu = pin.removable();
                if menu {
                    line = line.push(glow(button(ui::icon(ui::MORE, 16.0, ui::MUTED)).padding(4).style(ui::ghost).on_press(Message::PinMenu(index)), 14.0));
                }
                let row = glow(button(line).width(Length::Fill).padding(6).style(ui::row).on_press(Message::PinOpen(index)), 8.0);
                list.push(if menu { Element::from(mouse_area(row).on_right_press(Message::PinMenu(index))) } else { row.into() })
            })
            .into()
    }

    /// Заголовок блока со стрелкой, как на страницах оригинала.
    fn block_title<'a>(&self, title: &'a str, section: Section) -> Element<'a, Message> {
        glow(button(row![text(title).size(24).font(ui::font_headline()).color(ui::TEXT), ui::icon(ui::CHEVRON, 24.0, ui::MUTED)].spacing(6).align_y(Alignment::Center))
            .padding([4, 8])
            .style(ui::ghost)
            .on_press(Message::Open(section)), 20.0)
            .into()
    }

    /// Главная: играющий трек с управлением, откуда бы он ни играл.
    /// Страница волны: только управление волной. Трек и его кнопки — в нижнем плеере,
    /// середина свободна под будущую анимацию.
    fn view_wave(&self) -> Element<'_, Message> {
        if self.uid.is_none() {
            // Пока висит ошибка, волна не настраивается: обещать загрузку нечестно.
            let note = if self.error.is_some() { "Не удалось загрузить волну" } else { "Настраиваем волну…" };
            return placeholder(note, None);
        }
        let tuned = settings::wave_names(self.wave_tuning).join(", ");
        let title = match &self.wave_title {
            Some(title) if self.wave_queue => clipped(title, WAVE_LABEL),
            _ => "Моя волна".to_string(),
        };
        let playing = self.wave_queue && self.is_playing();
        // Играет волна — кнопка ставит её на паузу; играет что-то другое — включает волну.
        let press = if self.wave_queue { Message::Toggle } else { Message::WaveStart };
        // 64 + 2 × 8 = 80: при радиусе 40 у стиля кнопка ровно круглая.
        let play = shine(button(container(ui::icon(if playing { ui::PAUSE } else { ui::PLAY }, 36.0, ui::ON_ACCENT)).center(64)).padding(8).style(ui::round).on_press(press), 40.0);
        let note = if self.wave_loading {
            "Настраиваем волну…".to_string()
        } else if !tuned.is_empty() {
            tuned.clone()
        } else {
            "Музыка под ваш вкус без конца".to_string()
        };
        let tint = if tuned.is_empty() { ui::TEXT } else { ui::ACCENT };
        let mut tools = row![shine(button(row![ui::icon(ui::WAVE, 20.0, tint), text("Настроить").color(tint)].spacing(8).align_y(Alignment::Center)).padding([8, 16]).style(ui::secondary).on_press(Message::DialogOpen(Dialog::WaveTune)), 24.0)].spacing(10).align_y(Alignment::Center);
        if self.wave_queue && self.wave_base != [api::MY_WAVE] {
            let target = PinRef::Wave(self.wave_base.clone());
            let pinned = self.pins.iter().any(|pin| target.matches(pin));
            tools = tools.push(shine(button(container(ui::icon(ui::PIN, 20.0, if pinned { ui::ACCENT } else { ui::TEXT })).center(20)).padding(9).style(ui::secondary).on_press(Message::Pin(target)), 20.0));
            // Волна от исполнителя, альбома или настроения: можно вернуться к обычной.
            tools = tools.push(shine(button(text("К Моей волне").color(ui::TEXT)).padding([8, 16]).style(ui::secondary).on_press(Message::WaveReset), 24.0));
        }
        // Длинное название уменьшается под ширину, а если и так не влезает — переносится на вторую строку.
        let room = (self.main_width() - WAVE_TITLE_GAP).max(200.0);
        let size = (room / (title.chars().count().max(1) as f32 * WAVE_TITLE_CHAR)).clamp(WAVE_TITLE_MIN, WAVE_TITLE_MAX);
        let body = column![
            text(title).size(size).font(ui::font_headline()).color(ui::WHITE).wrapping(Wrapping::WordOrGlyph).align_x(Alignment::Center).width(room),
            text(note).size(16).color(if tuned.is_empty() { ui::MUTED } else { ui::ACCENT }).wrapping(Wrapping::None),
            Space::new().height(28),
            play,
            Space::new().height(28),
            tools,
        ]
        .spacing(8)
        .align_x(Alignment::Center);
        container(body).center(Length::Fill).into()
    }

    /// Большой плеер на всё окно: обложка, название, перемотка и кнопки.
    /// `back` — цвет фона, которым на миг закрывается смена трека.
    fn player_body<'a>(&'a self, track: &'a Track, art: f32, line: f32, back: iced::Color) -> Element<'a, Message> {
        let side = |icon, message| glow(button(ui::icon(icon, 32.0, ui::TEXT)).padding(8).style(ui::ghost).on_press(message), 20.0);
        let name_size = (line / (track.title.chars().count().max(1) as f32 * 0.62)).clamp(20.0, 34.0);
        let names: Element<'_, Message> = if track.artist_list.is_empty() {
            fit(&track.artists).size(18).color(ui::MUTED).marquee().into()
        } else {
            artist_links(&track.artist_list, 18.0, ui::font(), ui::MUTED)
        };
        let about = column![track_title(track, name_size, ui::font_headline(), ui::WHITE, true), names].spacing(4).align_x(Alignment::Center);
        let info = row![self.like_button(track, 24.0), container(about).center_x(Length::Fill).clip(true), self.more_button(track, 22.0)].spacing(12).align_y(Alignment::Center).width(line);
        let mode = |icon, on: bool, message| glow(button(ui::icon(icon, 24.0, if on { ui::ACCENT } else { ui::MUTED })).padding(8).style(ui::ghost).on_press(message), 20.0);
        let mut transport = row![].spacing(20).align_y(Alignment::Center);
        if !self.wave_queue {
            transport = transport.push(mode(ui::SHUFFLE, self.shuffle, Message::ShuffleToggle));
        }
        transport = transport
            .push(side(ui::PREVIOUS, Message::Previous))
            .push(shine(button(container(ui::icon(if self.is_playing() { ui::PAUSE } else { ui::PLAY }, 36.0, ui::ON_ACCENT)).center(56)).padding(4).style(ui::round).on_press(Message::Toggle), 40.0))
            .push(side(ui::NEXT, Message::Next));
        if !self.wave_queue {
            let icon = if self.repeat == Repeat::One { ui::REPEAT_ONE } else { ui::REPEAT };
            transport = transport.push(mode(icon, self.repeat != Repeat::Off, Message::RepeatNext));
        }

        let rest = 1.0 - anim::ease_out(1.0 - self.track_veil);
        let shift = 10.0 * rest;
        let veil = back.scale_alpha(rest);
        column![
            reveal(self.wave_cover(track, art)).shift(0.0, shift).veil(veil),
            Space::new().height(20),
            reveal(info).shift(0.0, shift).veil(veil),
            Space::new().height(12),
            self.seek_line(track, Length::Fixed(line)),
            Space::new().height(8),
            transport,
        ]
        .align_x(Alignment::Center)
        .into()
    }

    /// Плеер на всё окно поверх приложения: выезжает снизу, фон в цвет обложки.
    fn view_full(&self) -> Option<Element<'_, Message>> {
        self.full_fade.item?;
        let track = self.current_track()?;
        let (width, height) = *WINDOW.lock().unwrap();
        let art = (height - FULL_CHROME).min(width - 48.0).clamp(WAVE_COVER_MIN, FULL_COVER_MAX);
        let line = (width - 48.0).min(WAVE_COLUMN);
        let tint = self.full_tint;
        // Цвет обложки приглушён, чтобы белый текст читался на любой картинке.
        let shade = move |level: f32| iced::Color::from_rgb(tint.r * level, tint.g * level, tint.b * level);
        let top = shade(FULL_TINT);
        let source = if self.wave_queue { self.wave_title.clone().unwrap_or_else(|| "Моя волна".to_string()) } else { self.queue_title.clone().unwrap_or_default() };
        let collapse = glow(button(ui::icon(ui::COLLAPSE, 32.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::FullToggle), 20.0);
        let label = text(clipped(&source, WAVE_LABEL)).size(16).font(ui::font_medium()).color(ui::MUTED).wrapping(Wrapping::None);
        let head = row![collapse, container(label).center_x(Length::Fill), Space::new().width(44)].align_y(Alignment::Center);
        let tools = row![Space::new().width(Length::Fill), self.view_tools(22.0, true), self.view_volume(22.0, Some(120.0))].spacing(10).align_y(Alignment::Center);
        let page = column![head, container(self.player_body(track, art, line, top)).center(Length::Fill), tools].padding(24).width(Length::Fill).height(Length::Fill);
        let back = move |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Gradient(iced::Gradient::Linear(iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, top).add_stop(0.35, shade(FULL_TINT * 0.72)).add_stop(0.7, shade(FULL_TINT * 0.42)).add_stop(1.0, shade(FULL_TINT * 0.22))))),
            ..container::Style::default()
        };
        let shown = self.full_fade.shown();
        let sheet = anim::block(container(page).width(Length::Fill).height(Length::Fill).style(back));
        Some(reveal(sheet).shift(0.0, height * (1.0 - shown)).inert(!self.player_full).into())
    }

    fn view_volume<'a>(&self, icon: f32, width: Option<f32>) -> Element<'a, Message> {
        let shown = if self.muted { 0.0 } else { self.volume };
        let mut line = row![glow(button(ui::icon(if self.muted { ui::VOLUME_OFF } else { ui::VOLUME }, icon, ui::MUTED)).padding(4).style(ui::ghost).on_press(Message::ToggleMute), 20.0)].spacing(6).align_y(Alignment::Center);
        if let Some(width) = width {
            line = line.push(canvas(seek::Seek { value: shown, total: 1.0, on_change: Message::Volume, on_done: None }).width(width).height(SEEK_HEIGHT));
        }
        line.into()
    }

    fn view_track_page<'a>(&'a self, name: &'a str, tracks: &'a [Track], empty: &'a str, section: Section) -> Element<'a, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let title = row![back, text(name).size(36).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)].spacing(12).align_y(Alignment::Center);
        let center: Element<'a, Message> = if !tracks.is_empty() {
            self.view_list(section, None)
        } else if self.history_loading {
            skeleton_list()
        } else {
            placeholder("Пока пусто", Some(empty))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        let title = |label| text(label).size(20).font(ui::font_medium()).color(ui::TEXT);
        let choice = |label, chosen, press| shine(button(text(label)).padding([9, 18]).style(if chosen { ui::accent } else { ui::secondary }).on_press(press), 20.0);
        let link = |label, press| shine(button(text(label)).padding([9, 18]).style(ui::secondary).on_press(press), 20.0);

        let quality = Quality::ALL.into_iter().fold(row![].spacing(8), |line, quality| line.push(choice(quality.label(), quality == self.quality, Message::QualitySet(quality))));
        let mut sound = column![title("Качество звука"), quality].spacing(12);
        if let Some(playing) = &self.sound {
            sound = sound.push(text(format!("Сейчас играет: {}", playing.label)).size(13).color(ui::MUTED));
            if playing.below(self.quality.code()) {
                sound = sound.push(text("Яндекс не дал этот трек в выбранном качестве: играет лучшее, что есть для вашего аккаунта").size(13).color(ui::MUTED));
            }
        }

        let ai = row![
            column![text("Меньше ИИ").color(ui::TEXT), text("В рекомендациях будет реже встречаться музыка, созданная с помощью ИИ").size(13).color(ui::MUTED)].spacing(2).width(Length::Fill),
            view_switch(self.ai_less),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let ai = glow(button(ai).width(Length::Fill).padding(8).style(ui::row).on_press_maybe(self.ai_less.map(|enabled| Message::AiLessSet(!enabled))), 8.0);

        let cross = row![
            column![text("Плавные переходы между треками").color(ui::TEXT), text("Конец трека перетекает в начало следующего").size(13).color(ui::MUTED)].spacing(2).width(Length::Fill),
            view_switch(Some(self.crossfade)),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let cross = glow(button(cross).width(Length::Fill).padding(8).style(ui::row).on_press(Message::CrossfadeToggle), 8.0);

        let kids = row![
            column![text("Детский раздел").color(ui::TEXT), text("Пункт «Детям» в боковой панели: сказки, песни и аудиокниги для детей").size(13).color(ui::MUTED)].spacing(2).width(Length::Fill),
            view_switch(Some(self.kids)),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let kids = glow(button(kids).width(Length::Fill).padding(8).style(ui::row).on_press(Message::KidsToggle), 8.0);

        let page = column![
            text("Настройки").size(36).font(ui::font_headline()).color(ui::TEXT),
            sound,
            self.view_equalizer(),
            cross,
            column![title("Рекомендации"), ai, kids].spacing(8),
            self.view_stored(),
            view_hotkeys(),
            column![title("Аккаунт"), row![link("Управление аккаунтом", Message::DialogOpen(Dialog::Account)), link("Выйти", Message::DialogOpen(Dialog::Logout))].spacing(8)].spacing(12),
            column![
                title("О приложении"),
                text(format!("Tune Native, версия {}", env!("CARGO_PKG_VERSION"))).color(ui::MUTED),
                row![link("Помощь", Message::OpenLink("https://yandex.ru/support/music/")), link("Условия использования", Message::OpenLink("https://yandex.ru/legal/music_termsofuse/"))].spacing(8),
            ]
            .spacing(12),
        ]
        .spacing(28)
        .max_width(700)
        .padding(Padding::ZERO.right(20));
        ui::scroll(page).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
    }

    /// Сколько скачано и сколько места занимает, с переходом к списку и очисткой.
    fn view_stored(&self) -> Element<'_, Message> {
        let count = self.downloads.len();
        let summary = if count == 0 {
            "Пока ничего не скачано. Скачать трек, альбом или плейлист можно через меню «…»".to_string()
        } else {
            format!("{count} {} · {}", tracks_word(count), offline::size_label(self.downloads.iter().map(|saved| saved.bytes).sum()))
        };
        let mut block = column![text("Скачанное").size(20).font(ui::font_medium()).color(ui::TEXT), text(summary).color(ui::MUTED)].spacing(12);
        if count > 0 {
            let link = |label, press| shine(button(text(label)).padding([9, 18]).style(ui::secondary).on_press(press), 20.0);
            block = block.push(row![link("Открыть", Message::Open(Section::Downloads)), link("Удалить всё", Message::DialogOpen(Dialog::ClearDownloads))].spacing(8));
        }
        block.into()
    }

    /// Эквалайзер как в оригинале: уровень слева, десять полос, пресеты под ними.
    fn view_equalizer(&self) -> Element<'_, Message> {
        let eq = self.eq;
        let head = row![
            column![text("Эквалайзер").size(20).font(ui::font_medium()).color(ui::TEXT), text(equalizer::PRESETS[eq.preset].0).size(13).color(ui::MUTED)].spacing(2).width(Length::Fill),
            view_switch(Some(eq.enabled)),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let head = glow(button(head).width(Length::Fill).padding(8).style(ui::row).on_press(Message::EqToggle), 8.0);
        let presets = equalizer::PRESETS.iter().enumerate().fold(row![].spacing(8), |line, (index, (name, _))| {
            line.push(shine(button(text(*name).size(14)).padding([6, 14]).style(ui::chip(eq.enabled && index == eq.preset)).on_press(Message::EqPreset(index)), 16.0))
        });
        column![head, self.eq_bands(EQ_HEIGHT, 1.0), presets.wrap().vertical_spacing(8)].spacing(16).into()
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
        .padding(8)
        .style(ui::card)
        .on_press(Message::Open(Section::Likes));
        let likes = glow(likes, 12.0);

        // Первые восемь треков в две колонки.
        let column_of = |range: std::ops::Range<usize>| {
            range.fold(column![].spacing(0).width(Length::FillPortion(1)), |list, index| match self.liked.get(index) {
                Some(track) => list.push(self.view_track(Section::Likes, index, track)),
                None => list,
            })
        };
        let mut page = column![
            row![text("Коллекция").size(36).font(ui::font_headline()).color(ui::TEXT), Space::new().width(Length::Fill), self.block_title("Скачанное", Section::Downloads), self.block_title("История", Section::History), self.block_title("Дизлайки", Section::Dislikes)].align_y(Alignment::Center),
            Space::new().height(16),
            likes,
            if self.main_width() >= COLLECTION_COLUMNS { Element::from(row![column_of(0..4), column_of(4..8)].spacing(24)) } else { column_of(0..8).into() },
        ]
        .spacing(16)
        .padding(Padding::ZERO.right(SCROLLBAR));
        if !self.playlists.is_empty() {
            page = page.push(Space::new().height(8)).push(self.block_title("Плейлисты", Section::Playlists)).push(self.view_playlist_cards(&self.playlists));
        }
        if !self.liked_lists.is_empty() {
            page = page.push(Space::new().height(8)).push(text("Понравившиеся плейлисты").size(24).font(ui::font_headline()).color(ui::TEXT)).push(self.view_playlist_cards(&self.liked_lists));
        }
        if !self.liked_albums.is_empty() {
            let shown = &self.liked_albums[..self.liked_albums.len().min(COLLECTION_CARDS)];
            page = page.push(Space::new().height(8)).push(self.block_title("Альбомы", Section::Albums)).push(self.view_cards(shown, ALBUM_COVER, false, open_album));
        }
        if !self.liked_artists.is_empty() {
            let shown = &self.liked_artists[..self.liked_artists.len().min(COLLECTION_CARDS)];
            page = page.push(Space::new().height(8)).push(self.block_title("Исполнители", Section::Artists)).push(self.view_cards(shown, ARTIST_COVER, true, open_artist));
        }
        ui::scroll(page).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
    }

    fn view_card_page<'a>(&'a self, title: &'a str, cards: &'a [Card], prefix: &str, round: bool, empty: &'a str) -> Element<'a, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let head = row![back, text(title).size(36).font(ui::font_headline()).color(ui::TEXT)].spacing(12).align_y(Alignment::Center);
        let center: Element<'a, Message> = if cards.is_empty() {
            placeholder("Пока пусто", Some(empty))
        } else {
            let open = if round { open_artist } else { open_album };
            ui::scroll(container(self.view_cards(cards, prefix, round, open)).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
        };
        column![head, center].spacing(16).height(Length::Fill).into()
    }

    fn view_playlist_cards<'a>(&self, playlists: &'a [Playlist]) -> Element<'a, Message> {
        let cards = playlists.iter().fold(row![].spacing(4), |cards, playlist| {
            let cover: Element<'a, Message> = match self.covers.get(&playlist_cover_key(playlist)) {
                Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).width(CARD).height(CARD).border_radius(8).into(),
                None => container(ui::icon(ui::PLAY, 48.0, ui::MUTED)).center(CARD).style(ui::tile).into(),
            };
            let card = column![
                cover,
                fit(&playlist.title).font(ui::font_medium()).color(ui::TEXT).lines(2),
                text(format!("{} {}", playlist.count, tracks_word(playlist.count as usize))).size(13).color(ui::MUTED),
            ]
            .spacing(6)
            .width(CARD)
            .clip(true);
            cards.push(glow(button(card).padding(8).style(ui::card).on_press(Message::ListOpen(playlist.clone())), 12.0))
        });
        cards.wrap().into()
    }

    /// Крестик лежит внутри поля, чтобы при вводе оно не сдвигалось.
    fn view_likes_pin<'a>(&self) -> Element<'a, Message> {
        let Some(uid) = self.uid.clone() else { return Space::new().into() };
        let target = PinRef::Playlist(uid, FAVORITES_KIND.to_string());
        let pinned = self.pins.iter().any(|pin| target.matches(pin));
        shine(button(container(ui::icon(ui::PIN, 20.0, if pinned { ui::ACCENT } else { ui::TEXT })).center(20)).padding(7).style(ui::secondary).on_press(Message::Pin(target)), 20.0).into()
    }

    /// Скачать «Мне нравится» целиком. Всё уже скачано — значок отмечен и не нажимается.
    fn view_likes_download<'a>(&self) -> Element<'a, Message> {
        let all = !self.liked_ids.is_empty() && self.liked_ids.iter().all(|id| self.download_ids.contains(id));
        let busy = self.download_now.is_some();
        let (icon, tint) = if all { (ui::DOWNLOAD_DONE, ui::ACCENT) } else { (ui::DOWNLOAD, ui::TEXT) };
        let press = (!all && !busy && !self.offline && !self.liked_ids.is_empty()).then_some(Message::DialogOpen(Dialog::DownloadLikes));
        shine(button(container(ui::icon(icon, 20.0, tint)).center(20)).padding(7).style(ui::secondary).on_press_maybe(press), 20.0).into()
    }

    fn view_filter<'a>(&self) -> Element<'a, Message> {
        let width = if self.main_width() < FILTER_WIDE { FILTER_WIDTH_SMALL } else { FILTER_WIDTH };
        let input = text_input("Поиск по трекам", &self.filter).id(FILTER_INPUT).on_input(Message::Filter).padding(Padding { top: 8.0, right: 34.0, bottom: 8.0, left: 16.0 }).size(14).width(width).style(ui::input);
        // Поле всегда внутри одной и той же стопки: если бы с первой буквой вокруг него
        // появлялась обёртка, iced пересоздал бы поле и фокус пропал бы.
        let mut field = stack![input].width(width);
        if !self.filter.is_empty() {
            let clear = glow(button(ui::icon(ui::CLOSE, 16.0, ui::MUTED)).padding(4).style(ui::ghost).on_press(Message::Filter(String::new())), 14.0);
            field = field.push(container(clear).padding(Padding::ZERO.right(8)).align_right(width).height(Length::Fill).align_y(Alignment::Center));
        }
        field.into()
    }

    fn view_filter_list<'a>(&'a self, section: Section) -> Element<'a, Message> {
        if !self.filter_tracks.is_empty() {
            self.view_list(section, None)
        } else if self.filter_loading {
            skeleton_list()
        } else {
            placeholder("Ничего не нашлось", Some("Попробуйте другое название или исполнителя"))
        }
    }

    fn view_likes(&self) -> Element<'_, Message> {
        let count = match self.liked_ids.len() {
            0 => String::new(),
            n => format!("{n} {}", tracks_word(n)),
        };
        let name = row![fit("Мне нравится").size(36).font(ui::font_headline()).color(ui::TEXT).reserve(PLAYLIST_COUNT_RESERVE).no_tip(), text(count).color(ui::MUTED).wrapping(Wrapping::None)].spacing(12).align_y(Alignment::Center);
        let title = row![
            glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0),
            container(name).width(Length::Fill).clip(true),
            self.view_likes_pin(),
            self.view_likes_download(),
            self.view_filter(),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let center: Element<'_, Message> = if self.filtering() {
            self.view_filter_list(Section::Likes)
        } else if !self.liked.is_empty() {
            self.view_list(Section::Likes, None)
        } else if self.likes_loading || self.uid.is_none() {
            skeleton_list()
        } else {
            placeholder("Пока пусто", Some("Нажмите на сердце у трека, и он появится здесь"))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_playlists(&self) -> Element<'_, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        if let Some(playlist) = &self.playlist {
            if playlist.source == Source::Album {
                return self.view_album(playlist);
            }
            let name = row![
                fit(&playlist.title).size(36).font(ui::font_headline()).color(ui::TEXT).reserve(PLAYLIST_COUNT_RESERVE),
                text(if self.playlist_tracks.is_empty() { String::new() } else { format!("{} {}", playlist.count, tracks_word(playlist.count as usize)) }).color(ui::MUTED).wrapping(Wrapping::None),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            let mut title = row![back, container(name).width(Length::Fill).clip(true)].spacing(12).align_y(Alignment::Center);
            if playlist.source == Source::Playlist && !self.playlist_tracks.is_empty() {
                title = title.push(self.view_filter());
            }
            let mut head = column![title].spacing(10);
            if playlist.source == Source::Playlist {
                let meta = self.list_meta.as_ref();
                if let Some(description) = meta.map(|m| m.1.as_str()).filter(|d| !d.is_empty()) {
                    head = head.push(text(description).color(ui::MUTED));
                }
                let mut tools = self.page_actions(Section::Playlists, format!("playlist:{}_{}", playlist.owner, playlist.kind), &playlist.title);
                if !self.is_own(playlist) && playlist.kind != FAVORITES_KIND {
                    let liked = self.liked_lists.iter().any(|l| l.kind == playlist.kind && l.owner == playlist.owner);
                    tools = tools.push(self.page_like(liked, Message::ListLike(playlist.clone())));
                }
                tools = tools.push(self.page_more());
                head = head.push(tools.wrap().vertical_spacing(8));
            }
            let title = head;
            let center: Element<'_, Message> = if self.filtering() {
                self.view_filter_list(Section::Playlists)
            } else if !self.playlist_tracks.is_empty() {
                self.view_list(Section::Playlists, None)
            } else if playlist.count == 0 {
                placeholder("В этом плейлисте пока нет треков", None)
            } else {
                skeleton_list()
            };
            return column![title, center].spacing(16).height(Length::Fill).into();
        }
        let create = shine(button(row![ui::icon(ui::PLUS, 20.0, ui::TEXT), text("Новый плейлист").color(ui::TEXT)].spacing(6).align_y(Alignment::Center)).padding([6, 14]).style(ui::secondary).on_press(Message::DialogOpen(Dialog::Create)), 24.0);
        let title = row![back, text("Плейлисты").size(36).font(ui::font_headline()).color(ui::TEXT), Space::new().width(Length::Fill), create].spacing(12).align_y(Alignment::Center);
        let center: Element<'_, Message> = if !self.playlists.is_empty() {
            ui::scroll(container(self.view_playlist_cards(&self.playlists)).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
        } else if self.playlists_loading || self.uid.is_none() {
            skeleton_cards()
        } else {
            placeholder("Плейлистов пока нет", Some("Созданные вами плейлисты появятся здесь"))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_search(&self) -> Element<'_, Message> {
        let input = text_input("Трек, исполнитель или ссылка", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .padding([11, 20])
            .size(15)
            .style(ui::input);
        let mut page = column![input].spacing(16).height(Length::Fill);
        if !self.searched && !self.searching {
            if self.searches.is_empty() {
                return page.push(placeholder("Что послушаем?", Some("Начните вводить название трека, альбома или исполнителя"))).into();
            }
            let clear = shine(button(text("Очистить историю").size(14)).padding([6, 14]).style(ui::secondary).on_press(Message::SearchHistoryClear), 18.0);
            let head = row![text("Недавние запросы").size(24).font(ui::font_headline()).color(ui::TEXT), Space::new().width(Length::Fill), clear].align_y(Alignment::Center);
            let rows = self.searches.iter().fold(column![].spacing(2), |rows, query| {
                let line = row![ui::icon(ui::SEARCH, 20.0, ui::MUTED), fit(query).color(ui::TEXT)].spacing(12).align_y(Alignment::Center);
                rows.push(glow(button(line).width(Length::Fill).padding([10, 12]).style(ui::row).on_press(Message::SearchAgain(query.clone())), 8.0))
            });
            let history = column![container(head).padding(Padding::ZERO.right(SCROLLBAR)), ui::scroll(container(rows).padding(Padding::ZERO.right(SCROLLBAR))).width(Length::Fill).height(Length::Fill)].spacing(12);
            return page.push(history).into();
        }
        let tabs = SearchTab::ALL.into_iter().fold(row![].spacing(8), |tabs, tab| {
            tabs.push(shine(button(text(tab.label())).padding([7, 16]).style(ui::chip(tab == self.search_tab)).on_press(Message::SearchTabSet(tab)), 18.0))
        });
        page = page.push(tabs.wrap().vertical_spacing(8));
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
            ui::scroll(blocks).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
        };
        page.push(self.area(center)).into()
    }

    fn view_home(&self) -> Element<'_, Message> {
        let mut tabs = self.home.tabs.iter().enumerate().fold(row![].spacing(8), |tabs, (index, title)| {
            tabs.push(shine(button(text(title)).padding([7, 16]).style(ui::chip(index == self.home_tab)).on_press(Message::HomeTab(index)), 18.0))
        });
        if self.home_loading && self.home.tabs.is_empty() {
            tabs = SKELETON_TABS.iter().fold(tabs, |tabs, &width| tabs.push(container(Space::new()).width(width).height(SKELETON_TAB).style(skeleton_round(SKELETON_TAB / 2.0))));
        }
        let title = landing_title(self.section);
        let mut page = column![text(title).size(36).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)].spacing(16).height(Length::Fill);
        // Одна вкладка («Детям») только повторила бы заголовок.
        if self.home.tabs.len() != 1 {
            page = page.push(tabs);
        }
        if self.home_loading || self.home.blocks.is_empty() {
            return page.push(self.area(if self.home_loading { skeleton_home(self.cards_in_row()) } else { placeholder("Пока здесь пусто", None) })).into();
        }
        let in_row = self.cards_in_row();
        let blocks = self.home.blocks.iter().enumerate().fold(column![].spacing(12).padding(Padding::ZERO.right(SCROLLBAR)), |blocks, (index, block)| {
            let mut head = row![text(&block.title).size(24).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None), Space::new().width(Length::Fill)].spacing(4).align_y(Alignment::Center);
            let shown = self.home_shown(index, block);
            if block.items.len() > in_row {
                let arrow = |icon, active: bool, forward| glow(button(ui::icon(icon, 22.0, if active { ui::TEXT } else { ui::CONTROL_HOVER })).padding(5).style(ui::ghost).on_press_maybe(active.then_some(Message::HomePage(index, forward))), 20.0);
                head = head.push(arrow(ui::LEFT, shown.start > 0, false)).push(arrow(ui::RIGHT, shown.end < block.items.len(), true));
            }
            let mut body = if block.title.is_empty() { column![] } else { column![head] }.spacing(8);
            if !block.tabs.is_empty() {
                let chosen = self.home_block_tabs.get(&index).copied().unwrap_or(0);
                let chips = block.tabs.iter().enumerate().fold(row![].spacing(8), |chips, (at, (title, _))| {
                    chips.push(shine(button(text(title).size(14)).padding([5, 14]).style(ui::chip(at == chosen)).on_press(Message::HomeBlockTab(index, at)), 16.0))
                });
                body = body.push(chips.wrap());
            }
            if !shown.is_empty() {
                body = body.push(block.items[shown].iter().fold(row![].spacing(4), |line, item| line.push(self.view_home_item(item))));
            }
            if !block.tracks.is_empty() {
                let rows = block.tracks.iter().take(HOME_TRACKS).enumerate();
                body = body.push(rows.fold(column![], |list, (at, track)| list.push(self.view_track_row(Message::HomePlay(index, at), track))));
            }
            blocks.push(body).push(Space::new().height(8))
        });
        page.push(self.area(ui::scroll(blocks).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill))).into()
    }

    fn view_home_item<'a>(&self, item: &'a HomeItem) -> Element<'a, Message> {
        let (title, subtitle, round): (&str, String, bool) = match item {
            HomeItem::Album(card) => (&card.title, card.subtitle.clone(), false),
            HomeItem::Artist(card) => (&card.title, card.subtitle.clone(), true),
            HomeItem::Playlist(list) => (&list.title, if list.count > 0 { format!("{} {}", list.count, tracks_word(list.count as usize)) } else { "Плейлист".to_string() }, false),
            HomeItem::Wave { title, .. } => (title, "Волна".to_string(), true),
            HomeItem::Mix { title, tag, .. } => (title, if tag.starts_with(api::CATEGORY) { "Категория" } else { "Микс" }.to_string(), false),
            HomeItem::Likes { count, .. } => ("Мне нравится", format!("{count} {}", tracks_word(*count as usize)), false),
            HomeItem::History { subtitle, .. } => ("История", if subtitle.is_empty() { "Недавно прослушанное".to_string() } else { subtitle.clone() }, false),
        };
        let radius = if round { CARD / 2.0 } else { 8.0 };
        let color = match item {
            HomeItem::Wave { color, .. } => color.as_deref().and_then(ui::hex_color),
            _ => None,
        };
        let cover: Element<'a, Message> = match self.covers.get(&home_item_key(item)) {
            Some(handle) => {
                let picture = image(handle.clone()).opacity(self.cover_alpha(handle)).width(CARD).height(CARD).border_radius(radius);
                match color {
                    Some(color) => container(picture).style(move |_| container::Style::default().background(color).border(iced::border::rounded(radius))).into(),
                    None => picture.into(),
                }
            }
            None => container(Space::new()).width(CARD).height(CARD).style(if round { ui::avatar } else { ui::tile }).into(),
        };
        let body = column![cover, fit(title).font(ui::font_medium()).color(ui::TEXT).lines(2), fit(subtitle).size(13).color(ui::MUTED)].spacing(6).width(CARD).clip(true);
        glow(button(body).padding(8).style(ui::card).on_press(Message::HomeOpen(item.clone())), 12.0)
    }

    /// Загружается альбом или исполнитель: шапка стоит там же, где будет настоящая.
    fn skeleton_page<'a>(&self, round: bool) -> Element<'a, Message> {
        let side = if self.main_width() < PAGE_COVER_WIDTH { PAGE_COVER_SMALL } else { PAGE_COVER };
        let cover = container(Space::new()).width(side).height(side).style(skeleton_round(if round { side / 2.0 } else { 8.0 }));
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let info = column![skeleton_bar(70.0, 12.0), skeleton_bar(280.0, 34.0), skeleton_bar(160.0, 12.0)].spacing(14);
        let head = container(row![container(back).height(PAGE_COVER), cover, container(info).center_y(PAGE_HEADER - 8.0)].spacing(16)).height(PAGE_HEADER).width(Length::Fill);
        column![head, skeleton_list()].spacing(16).height(Length::Fill).into()
    }

    /// Шапка альбома и исполнителя: обложка, название, подписи и кнопки.
    fn page_header<'a>(&self, kicker: &'a str, title: String, cover_key: String, round: bool, links: Option<Element<'a, Message>>, about: String, actions: iced::widget::Row<'a, Message>) -> Element<'a, Message> {
        let side = if self.main_width() < PAGE_COVER_WIDTH { PAGE_COVER_SMALL } else { PAGE_COVER };
        let radius = if round { side / 2.0 } else { 8.0 };
        let cover: Element<'a, Message> = match self.covers.get(&cover_key) {
            Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).width(side).height(side).border_radius(radius).into(),
            None => container(Space::new()).width(side).height(side).style(if round { ui::avatar } else { ui::tile }).into(),
        };
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let mut info = column![text(kicker).size(14).color(ui::MUTED), fit(title).size(40).font(ui::font_headline()).color(ui::WHITE)].spacing(6).width(Length::Fill).clip(true);
        if let Some(links) = links {
            info = info.push(links);
        }
        let info = info.push(fit(about).color(ui::MUTED)).push(Space::new().height(8)).push(actions.wrap().vertical_spacing(8));
        container(row![container(back).height(PAGE_COVER), cover, container(info).center_y(PAGE_HEADER - 8.0)].spacing(16)).height(PAGE_HEADER).width(Length::Fill).into()
    }

    /// Кнопки шапки: запуск списка и волна по нему.
    fn page_actions<'a>(&self, section: Section, seed: String, name: &str) -> iced::widget::Row<'a, Message> {
        let title = wave_label(&seed, name);
        let play = shine(button(row![ui::icon(ui::PLAY, 22.0, ui::ON_ACCENT), text("Слушать").font(ui::font_medium()).color(ui::ON_ACCENT)].spacing(6).align_y(Alignment::Center))
            .padding([8, 18])
            .style(ui::accent)
            .on_press_maybe((!self.list(section).is_empty()).then_some(Message::Play(section, 0))), 24.0);
        let wave = shine(button(row![ui::icon(ui::WAVE, 20.0, ui::TEXT), text("Моя волна").color(ui::TEXT)].spacing(6).align_y(Alignment::Center))
            .padding([8, 18])
            .style(ui::secondary)
            .on_press(Message::WaveStartSeeds(vec![seed], Some(title))), 24.0);
        row![play, wave].spacing(8).align_y(Alignment::Center)
    }

    fn page_like<'a>(&self, liked: bool, message: Message) -> Element<'a, Message> {
        let icon = if liked { ui::HEART } else { ui::HEART_OUTLINE };
        shine(button(container(ui::icon(icon, 22.0, ui::TEXT)).center(22)).padding(9).style(ui::secondary).on_press(message), 24.0).into()
    }

    fn view_album<'a>(&'a self, playlist: &'a Playlist) -> Element<'a, Message> {
        let Some(album) = &self.album else {
            return self.skeleton_page(false);
        };
        if album.kind != AlbumKind::Music {
            return self.view_book(playlist, album);
        }
        let count = self.playlist_tracks.len();
        let mut about: Vec<String> = album.year.map(|year| year.to_string()).into_iter().collect();
        about.push(format!("{count} {}", tracks_word(count)));
        if album.volumes.len() > 1 {
            about.push(format!("{} {}", album.volumes.len(), disks_word(album.volumes.len())));
        }
        if !album.version.is_empty() {
            about.push(album.version.clone());
        }
        let actions = self.page_actions(Section::Playlists, format!("album:{}", album.id), &album.title);
        let liked = self.liked_albums.iter().any(|c| c.id == album.id);
        let actions = actions.push(self.page_like(liked, Message::AlbumLike)).push(self.page_more());
        let links = artist_links(&album.artists, 16.0, ui::font(), ui::MUTED);
        let header = self.page_header("Альбом", playlist.title.clone(), format!("{ALBUM_COVER}{}", album.id), false, Some(links), about.join(" · "), actions);
        self.view_list(Section::Playlists, Some(header))
    }

    /// Подкаст или аудиокнига: «Продолжить» с начатого выпуска, сезоны и порядок выпусков.
    fn view_book<'a>(&'a self, playlist: &'a Playlist, album: &'a Album) -> Element<'a, Message> {
        let podcast = album.kind == AlbumKind::Podcast;
        let count = if podcast { self.episodes.len() } else { self.playlist_tracks.len() };
        let mut about: Vec<String> = album.year.map(|year| year.to_string()).into_iter().collect();
        about.push(format!("{count} {}", if podcast { plural(count, ["выпуск", "выпуска", "выпусков"]) } else { plural(count, ["глава", "главы", "глав"]) }));
        if podcast && album.volumes.len() > 1 {
            about.push(format!("{} {}", album.volumes.len(), plural(album.volumes.len(), ["сезон", "сезона", "сезонов"])));
        }
        if album.explicit {
            about.push("18+".to_string());
        }
        if album.finished {
            about.push("прослушано".to_string());
        }
        let (label, start) = match self.resume_index() {
            Some(index) => ("Продолжить", index),
            None => ("Слушать", 0),
        };
        let play = shine(button(row![ui::icon(ui::PLAY, 22.0, ui::ON_ACCENT), text(label).font(ui::font_medium()).color(ui::ON_ACCENT)].spacing(6).align_y(Alignment::Center))
            .padding([8, 18])
            .style(ui::accent)
            .on_press_maybe((!self.playlist_tracks.is_empty()).then_some(Message::Play(Section::Playlists, start))), 24.0);
        let mut actions = row![play].spacing(8).align_y(Alignment::Center);
        if let Some((label, url)) = &album.action {
            actions = actions.push(shine(button(text(label).color(ui::TEXT)).padding([8, 18]).style(ui::secondary).on_press(Message::OpenUrl(url.clone())), 24.0));
        }
        let liked = self.liked_albums.iter().any(|c| c.id == album.id);
        let actions = actions.push(self.page_like(liked, Message::AlbumLike)).push(self.page_more());
        let links = artist_links(&album.artists, 16.0, ui::font(), ui::MUTED);
        let kicker = if podcast { "Подкаст" } else { "Аудиокнига" };
        let header = self.page_header(kicker, playlist.title.clone(), format!("{ALBUM_COVER}{}", album.id), false, Some(links), about.join(" · "), actions);
        if !podcast || self.episodes.len() < 2 || self.filtering() {
            return self.view_list(Section::Playlists, Some(header));
        }
        let chip = |label, old_first: bool| {
            let chosen = old_first == self.episodes_old_first;
            shine(button(text(label)).padding([7, 16]).style(if chosen { ui::accent } else { ui::secondary }).on_press(Message::EpisodesOrder(old_first)), 20.0)
        };
        let mut tools = row![chip("Сначала новые", false), chip("Сначала старые", true)].spacing(8).align_y(Alignment::Center);
        if album.volumes.len() > 1 {
            let seasons: Vec<Season> = std::iter::once(Season(None)).chain((0..album.volumes.len()).map(|season| Season(Some(season)))).collect();
            let select = pick_list(seasons, Some(Season(self.season)), |season: Season| Message::SeasonPick(season.0)).padding([7, 14]).text_size(14).style(ui::select_faded(1.0)).menu_style(ui::select_menu);
            tools = tools.push(select);
        }
        let header = column![header, container(tools).padding(Padding::new(8.0).top(4).bottom(12))].into();
        self.view_list(Section::Playlists, Some(header))
    }

    fn view_cards<'a>(&self, cards: &'a [Card], prefix: &str, round: bool, open: fn(&Card) -> Message) -> Element<'a, Message> {
        cards
            .iter()
            .fold(row![].spacing(4), |line, card| {
                let cover: Element<'a, Message> = match self.covers.get(&format!("{prefix}{}", card.id)) {
                    Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).width(CARD).height(CARD).border_radius(if round { CARD / 2.0 } else { 8.0 }).into(),
                    None => container(Space::new()).width(CARD).height(CARD).style(if round { ui::avatar } else { ui::tile }).into(),
                };
                let body = column![cover, fit(&card.title).font(ui::font_medium()).color(ui::TEXT).lines(2), fit(&card.subtitle).size(13).color(ui::MUTED)].spacing(6).width(CARD).clip(true);
                line.push(glow(button(body).padding(8).style(ui::card).on_press(open(card)), 12.0))
            })
            .wrap()
            .into()
    }

    fn view_artist(&self) -> Element<'_, Message> {
        let Some(artist) = &self.artist else {
            return self.skeleton_page(true);
        };
        let about = if artist.listeners > 0 { format!("Слушателей в месяц: {}", grouped(artist.listeners)) } else { String::new() };
        let actions = self.page_actions(Section::Artist, format!("artist:{}", artist.id), &artist.name).push(shine(button(text("Все треки").color(ui::TEXT)).padding([8, 18]).style(ui::secondary).on_press(Message::ArtistTracks), 24.0));
        let liked = self.liked_artists.iter().any(|c| c.id == artist.id);
        let actions = actions.push(self.page_like(liked, Message::ArtistLike)).push(self.page_more());
        let block = |title| text(title).size(24).font(ui::font_headline()).color(ui::TEXT);
        let mut page = column![self.page_header("Исполнитель", artist.name.clone(), format!("{ARTIST_COVER}{}", artist.id), true, None, about, actions)].spacing(12).padding(Padding::ZERO.right(SCROLLBAR));
        if !artist.popular.is_empty() {
            page = page.push(block("Популярные треки"));
            page = page.push(artist.popular.iter().enumerate().fold(column![], |list, (index, track)| list.push(self.view_track(Section::Artist, index, track))));
        }
        let albums = |cards: &'_ [Card]| !cards.is_empty();
        if albums(&artist.albums) {
            page = page.push(Space::new().height(8)).push(more_title("Альбомы", Message::DiscographyOpen(false))).push(self.view_cards(&artist.albums, ALBUM_COVER, false, open_album));
        }
        if albums(&artist.also) {
            page = page.push(Space::new().height(8)).push(more_title("Сборники", Message::DiscographyOpen(true))).push(self.view_cards(&artist.also, ALBUM_COVER, false, open_album));
        }
        if albums(&artist.similar) {
            page = page.push(Space::new().height(8)).push(block("Похожие исполнители")).push(self.view_cards(&artist.similar, ARTIST_COVER, true, open_artist));
        }
        if !artist.about.is_empty() || !artist.links.is_empty() {
            page = page.push(Space::new().height(8)).push(block("Об исполнителе"));
            if !artist.about.is_empty() {
                page = page.push(container(text(&artist.about).size(15).line_height(1.5).color(ui::TEXT)).max_width(760));
            }
            let links = artist.links.iter().fold(row![].spacing(8), |line, link| {
                let mut label = column![text(&link.title).color(ui::TEXT)];
                if !link.subtitle.is_empty() {
                    label = label.push(text(&link.subtitle).size(12).color(ui::MUTED));
                }
                line.push(shine(button(label).padding([8, 16]).style(ui::secondary).on_press(Message::OpenUrl(link.url.clone())), 20.0))
            });
            page = page.push(links.wrap());
        }
        ui::scroll(page).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
    }

    fn view_tag(&self) -> Element<'_, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let title = self.tag.as_ref().map_or(String::new(), |(_, title)| title.clone());
        let head = row![back, text(title).size(36).font(ui::font_headline()).color(ui::TEXT)].spacing(12).align_y(Alignment::Center);
        let center: Element<'_, Message> = if self.tag_loading {
            skeleton_home(self.cards_in_row())
        } else if self.tag_lists.is_empty() {
            placeholder("Пока пусто", Some("В этом миксе нет плейлистов"))
        } else {
            let cards = self.tag_lists.iter().fold(row![].spacing(4), |cards, item| cards.push(self.view_home_item(item)));
            ui::scroll(container(cards.wrap()).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
        };
        column![head, center].spacing(16).height(Length::Fill).into()
    }

    fn view_discography(&self) -> Element<'_, Message> {
        let back = glow(button(ui::icon(ui::BACK, 24.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::Back), 20.0);
        let title = if self.discography_also { "Сборники" } else { "Альбомы и синглы" };
        let name = self.artist.as_ref().map_or(String::new(), |artist| artist.name.clone());
        let head = row![back, column![text(title).size(36).font(ui::font_headline()).color(ui::TEXT), text(name).color(ui::MUTED)]].spacing(12).align_y(Alignment::Center);
        let chip = |label, by_year: bool| {
            let chosen = by_year == self.discography_by_year;
            shine(button(text(label)).padding([7, 16]).style(if chosen { ui::accent } else { ui::secondary }).on_press(Message::DiscographySort(by_year)), 20.0)
        };
        let sort = row![chip("По популярности", false), chip("По дате выхода", true)].spacing(8);
        let mut list = column![self.view_cards(&self.discography, ALBUM_COVER, false, open_album)].spacing(16);
        if (self.discography.len() as u64) < self.discography_total && self.discography_key.is_none() {
            list = list.push(container(shine(button(text("Показать ещё")).padding([9, 18]).style(ui::secondary).on_press(Message::DiscographyMore), 20.0)).center_x(Length::Fill));
        }
        let center: Element<'_, Message> = if self.discography.is_empty() && self.discography_key.is_none() {
            placeholder("Пока пусто", Some("У исполнителя нет таких релизов"))
        } else {
            ui::scroll(container(list).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
        };
        column![head, sort, center].spacing(16).height(Length::Fill).into()
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
        let disks = self.album.as_ref().map_or(0, |album| album.volumes.len());
        if disks > 1 && self.album_place(section, 0).is_some() && tracks.len() == total {
            // Альбом с дисками короткий: строки идут целиком, с заголовком перед каждым диском.
            for (index, track) in tracks.iter().enumerate() {
                if let Some((disk, 1)) = self.album_place(section, index) {
                    let part = if self.album.as_ref().is_some_and(|album| album.kind == AlbumKind::Audiobook) { "Часть" } else { "Диск" };
                    list = list.push(container(text(format!("{part} {disk}")).size(18).font(ui::font_medium()).color(ui::TEXT)).padding(Padding::new(8.0).top(if disk > 1 { 20.0 } else { 8.0 })));
                }
                list = list.push(self.view_track(section, index, track));
            }
        } else {
            list = list.push(Space::new().height(range.start as f32 * ROW));
            for index in range.clone() {
                list = list.push(match tracks.get(index) {
                    Some(track) => self.view_track(section, index, track),
                    None => skeleton_row(),
                });
            }
            list = list.push(Space::new().height((total - range.end) as f32 * ROW));
        }
        if let Some(album) = self.album.as_ref().filter(|_| section == Section::Playlists && !self.filtering()) {
            let mut facts = column![].spacing(2);
            if let Some(released) = &album.released {
                facts = facts.push(text(format!("Дата выхода: {released}")).size(13).color(ui::MUTED));
            }
            let music = album.kind == AlbumKind::Music;
            if !album.labels.is_empty() {
                facts = facts.push(text(format!("{}: {}", if music { "Лейбл" } else { "Издатель" }, album.labels.join(", "))).size(13).color(ui::MUTED));
            }
            if !music && !album.description.is_empty() {
                let title = if album.kind == AlbumKind::Podcast { "О подкасте" } else { "О книге" };
                list = list.push(Space::new().height(24)).push(text(title).size(24).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)).push(Space::new().height(8));
                list = list.push(container(text(&album.description).size(15).line_height(1.5).color(ui::TEXT)).max_width(760).padding([0, 8]));
            }
            list = list.push(container(facts).padding(Padding::new(8.0).top(16)));
            let others = if music { "Другие альбомы исполнителя" } else { "Ещё у автора" };
            for (title, cards) in [("Другие версии", &album.versions), (others, &self.album_others)] {
                if !cards.is_empty() {
                    list = list.push(Space::new().height(24)).push(text(title).size(24).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)).push(Space::new().height(8)).push(self.view_cards(cards, ALBUM_COVER, false, open_album));
                }
            }
        }
        if section == Section::Playlists && !self.filtering() && !self.similar.is_empty() {
            let cards = self.similar.iter().fold(row![].spacing(4), |cards, item| cards.push(self.view_home_item(item)));
            list = list.push(Space::new().height(24)).push(text("Похожее").size(24).font(ui::font_headline()).color(ui::TEXT).wrapping(Wrapping::None)).push(Space::new().height(8)).push(cards.wrap()).push(Space::new().height(16));
        }
        ui::scroll(list).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).height(Length::Fill).into()
    }

    fn view_track<'a>(&'a self, section: Section, index: usize, track: &'a Track) -> Element<'a, Message> {
        let number = self.album_place(section, index).map(|(_, number)| number);
        // Свой плейлист без поиска внутри можно переставлять.
        if section == Section::Playlists && !self.filtering() && self.playlist.as_ref().is_some_and(|list| self.is_own(list)) {
            let row = self.view_numbered_row(Message::Play(section, index), track, number, Some(self.grip(DragList::Playlist, index)));
            return self.drag_target(row, DragList::Playlist, index);
        }
        self.view_numbered_row(Message::Play(section, index), track, number, None)
    }

    fn view_track_row<'a>(&'a self, play: Message, track: &'a Track) -> Element<'a, Message> {
        self.view_numbered_row(play, track, None, None)
    }

    /// Открыт альбом без поиска внутри: у трека есть диск и номер на диске, с единицы.
    fn album_place(&self, section: Section, index: usize) -> Option<(usize, usize)> {
        let album = self.album.as_ref().filter(|album| section == Section::Playlists && !self.filtering() && album.kind != AlbumKind::Podcast)?;
        let mut left = index;
        for (disk, &size) in album.volumes.iter().enumerate() {
            if left < size {
                return Some((disk + 1, left + 1));
            }
            left -= size;
        }
        None
    }

    /// Строка трека. В альбоме вместо одинаковых обложек стоят номера, как в оригинале.
    fn view_numbered_row<'a>(&'a self, play: Message, track: &'a Track, number: Option<usize>, grip: Option<Element<'a, Message>>) -> Element<'a, Message> {
        let current = self.current_track().is_some_and(|t| t.id == track.id);
        let cover: Element<'_, Message> = if let Some(number) = number {
            let mark: Element<'_, Message> = if current {
                ui::icon(if self.is_playing() { ui::PAUSE } else { ui::PLAY }, 22.0, ui::ACCENT)
            } else {
                text(number.to_string()).color(if track.unavailable { FAINT } else { ui::MUTED }).into()
            };
            container(mark).center(COVER).into()
        } else if current {
            let icon = if self.is_playing() { ui::PAUSE } else { ui::PLAY };
            stack![self.cover(track, COVER), container(ui::icon(icon, 22.0, ui::WHITE)).center(COVER).style(ui::overlay)].into()
        } else {
            self.cover(track, COVER)
        };
        let (title_color, names_color) = if track.unavailable { (ui::MUTED, FAINT) } else { (ui::WHITE, ui::MUTED) };
        let names: Element<'a, Message> = if let Some(published) = track.published.as_ref().filter(|_| track.remember) {
            fit(published).color(names_color).into()
        } else if track.artist_list.is_empty() {
            fit(&track.artists).color(names_color).into()
        } else {
            artist_links(&track.artist_list, 16.0, ui::font(), names_color)
        };
        let names = self.download_mark(track, names);
        let play = if track.unavailable { Message::Unavailable } else { play };
        let mut line = row![
            cover,
            column![track_title(track, 16.0, ui::font_medium(), title_color, false), names].width(Length::Fill).clip(true),
            self.like_button(track, 20.0),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        // У недоступных треков Яндекс длительности не присылает: вместо «00:00» пустое место той же ширины.
        if self.main_width() >= TRACK_TIME_WIDTH {
            let time_color = if track.duration.is_zero() { iced::Color::TRANSPARENT } else { names_color };
            // Колонка одной ширины: сердечки в строках стоят ровно.
            let width = if track.remember && self.wide_times() { TIME_WIDTH_EPISODE } else { TIME_WIDTH };
            line = line.push(container(text(self.track_time(track)).color(time_color)).width(width).align_x(Alignment::End));
        }
        let mut line = line.push(self.more_button(track, 18.0));
        if let Some(grip) = grip {
            line = line.push(grip);
        }
        mouse_area(glow(button(line).width(Length::Fill).height(ROW).padding([8, 8]).style(ui::row).on_press(play), 8.0)).on_right_press(Message::MenuOpen(track.clone())).into()
    }

    /// Длительность трека, а у начатого выпуска или главы — сколько осталось.
    fn track_time(&self, track: &Track) -> String {
        let end = track.duration.as_secs_f32();
        match self.place(track) {
            _ if self.listened(track) => "прослушано".to_string(),
            Some(place) if place > PROGRESS_MIN => format!("осталось {}", clock(Duration::from_secs_f32(end - place))),
            _ => clock(track.duration),
        }
    }

    /// В списке есть «осталось …» или «прослушано»: колонке времени нужна большая ширина.
    fn wide_times(&self) -> bool {
        self.list(self.section).iter().any(|track| self.listened(track) || self.place(track).is_some_and(|place| place > PROGRESS_MIN))
    }

    /// Где остановились в выпуске или главе: свежее место этого запуска или ответ сервера.
    fn place(&self, track: &Track) -> Option<f32> {
        if track.remember { self.progress.get(&track.id).copied().or(track.progress) } else { None }
    }

    /// Дослушан ли выпуск. Свежая отметка этого запуска главнее ответа сервера; там дослушанный
    /// помечен `everFinished`, пока его не начали заново.
    fn listened(&self, track: &Track) -> bool {
        let end = track.duration.as_secs_f32();
        let near_end = |place: f32| end > 0.0 && place >= end - PROGRESS_TAIL;
        match self.progress.get(&track.id) {
            Some(&place) => track.remember && near_end(place),
            None => track.remember && (near_end(track.progress.unwrap_or(0.0)) || (track.finished && track.progress.unwrap_or(0.0) <= PROGRESS_MIN)),
        }
    }

    /// Начатый и не дослушанный выпуск или глава на странице: с него «Продолжить».
    fn resume_index(&self) -> Option<usize> {
        self.playlist_tracks.iter().position(|track| !self.listened(track) && self.place(track).is_some_and(|place| place > PROGRESS_MIN))
    }

    /// Выпуски подкаста для страницы: выбранный сезон, сначала новые или старые.
    fn episodes_arranged(&self) -> Vec<Track> {
        let sizes = self.album.as_ref().map_or(&[][..], |album| album.volumes.as_slice());
        let mut start = 0;
        let mut chosen: Vec<Track> = Vec::new();
        for (season, &size) in sizes.iter().enumerate() {
            let end = (start + size).min(self.episodes.len());
            if self.season.is_none_or(|wanted| wanted == season) {
                chosen.extend_from_slice(&self.episodes[start.min(end)..end]);
            }
            start = end;
        }
        if sizes.is_empty() {
            chosen = self.episodes.clone();
        }
        if !self.episodes_old_first {
            chosen.reverse();
        }
        chosen
    }

    fn like_button<'a>(&self, track: &Track, size: f32) -> Element<'a, Message> {
        let liked = self.liked_set.contains(&track.id);
        let icon = if liked { ui::icon(ui::HEART, size, ui::MUTED) } else { ui::icon(ui::HEART_OUTLINE, size, ui::MUTED) };
        glow(button(container(icon).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::Like(track.clone())), 20.0).into()
    }

    fn cover_alpha(&self, handle: &image::Handle) -> f32 {
        self.cover_fades.get(&handle.id()).map_or(1.0, |level| anim::ease_out(*level))
    }

    /// Обложка играющего трека в волне: пока нет крупной, показывает обычную.
    fn wave_cover<'a>(&'a self, track: &Track, size: f32) -> Element<'a, Message> {
        match self.covers.get(&format!("{WAVE_BIG}{}", track.id)).or_else(|| self.covers.get(&track.id)) {
            Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).width(size).height(size).border_radius(12).into(),
            None => container(Space::new()).width(size).height(size).style(ui::tile).into(),
        }
    }

    /// Полоска перемотки с подписями времени по краям. Подписи одной ширины, чтобы полоска не дёргалась.
    fn seek_line<'a>(&self, track: &Track, width: Length) -> Element<'a, Message> {
        let total = track.duration.as_secs_f32().max(1.0);
        let shown = self.seeking.unwrap_or(self.position.as_secs_f32()).min(total);
        let time = |value: Duration| text(clock(value)).size(12).color(ui::MUTED).wrapping(Wrapping::None);
        row![
            container(time(Duration::from_secs_f32(shown))).width(SEEK_TIME),
            canvas(seek::Seek { value: shown, total, on_change: Message::Seek, on_done: Some(Message::SeekDone) }).width(Length::Fill).height(SEEK_HEIGHT),
            container(time(track.duration)).align_right(SEEK_TIME),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(width)
        .into()
    }

    fn cover<'a>(&'a self, track: &Track, size: f32) -> Element<'a, Message> {
        let dim = if track.unavailable { UNAVAILABLE_COVER } else { 1.0 };
        match self.covers.get(&track.id) {
            Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle) * dim).width(size).height(size).border_radius(6).into(),
            None => container(Space::new()).width(size).height(size).style(ui::tile).into(),
        }
    }

    fn is_playing(&self) -> bool {
        self.starting || (self.active && !self.player.as_ref().is_none_or(Player::is_paused))
    }

    /// Ширина боковых частей трекбара и нужна ли строка с названием сверху.
    /// Кнопки стоят посередине, края занимают равные части.
    fn bar_layout(&self) -> (f32, bool) {
        let inner = self.main_width() - 16.0;
        let core = 2.0 * BAR_SIDE + BAR_TOGGLE + 2.0 * BAR_GAP;
        let transport = if self.wave_queue { core } else { core + 2.0 * (BAR_MODE + BAR_GAP) };
        let zone = ((inner - transport) / 2.0 - 16.0).max(0.0);
        let room = zone - BAR_COVER - 12.0 - 2.0 * (LIKE + 12.0);
        (zone, room < BAR_TITLE_ROOM)
    }

    fn bar_top_target(&self) -> f32 {
        if self.bar_layout().1 {
            1.0
        } else {
            0.0
        }
    }

    /// Все кнопки есть всегда. Когда окно сужается, сжимается громкость, а потом название
    /// и громкость плавно переезжают в тонкую строку сверху; не поместится и там — многоточие.
    fn view_bar(&self) -> Option<Element<'_, Message>> {
        let track = self.current_track()?;
        let side = |icon, message| glow(button(ui::icon(icon, 26.0, ui::MUTED)).padding(6).style(ui::ghost).on_press(message), 20.0);
        let toggle = shine(button(container(ui::icon(if self.is_playing() { ui::PAUSE } else { ui::PLAY }, 26.0, ui::ON_ACCENT)).center(26)).padding(7).style(ui::round).on_press(Message::Toggle), 40.0);
        let mode = |icon, on: bool, message| glow(button(ui::icon(icon, 22.0, if on { ui::ACCENT } else { ui::MUTED })).padding(8).style(ui::ghost).on_press(message), 20.0);
        let (zone, _) = self.bar_layout();

        let names = |size: f32, alpha: f32| -> Element<'_, Message> {
            if self.starting {
                text("Загрузка…").size(size).color(ui::MUTED.scale_alpha(alpha)).wrapping(Wrapping::None).into()
            } else if track.artist_list.is_empty() {
                fit(&track.artists).size(size).color(ui::MUTED.scale_alpha(alpha)).marquee().into()
            } else {
                artist_links(&track.artist_list, size, ui::font(), ui::MUTED.scale_alpha(alpha)).into()
            }
        };
        // Насколько название и громкость уже переехали в верхнюю строку: всё движется вместе с ней.
        let shown = anim::ease_out(self.bar_top);
        let cover = button(self.cover(track, BAR_COVER)).padding(0).style(ui::ghost).on_press(Message::FullToggle);
        let mut about = row![cover].spacing(12).align_y(Alignment::Center);
        if shown < 1.0 {
            // Пока строка сверху выезжает, название внизу сужается и гаснет, а не исчезает разом.
            let stay = 1.0 - shown;
            let room = (zone - BAR_COVER - 12.0 - 2.0 * (LIKE + 12.0)).max(0.0);
            let width = if shown > 0.0 { Length::Fixed(room * stay) } else { Length::Fill };
            about = about.push(column![track_title(track, 16.0, ui::font_medium(), ui::WHITE.scale_alpha(stay), true), names(16.0, stay)].width(width).clip(true));
        }
        about = about.push(self.like_button(track, 22.0)).push(self.more_button(track, 20.0));
        let about = mouse_area(container(about).width(zone).clip(true)).on_right_press(Message::MenuOpen(track.clone()));

        // В волне порядок задаёт она сама, поэтому этих кнопок там нет.
        let mut buttons = row![].spacing(BAR_GAP).align_y(Alignment::Center);
        // У выпусков и книг вместо перемешивания и повтора — шаг на 15 секунд, как в оригинале.
        let jump = |icon, step: f32| {
            let face = stack![ui::icon(icon, 26.0, ui::MUTED), container(text("15").size(8).font(ui::font_medium()).color(ui::MUTED)).center(26.0).padding(Padding::ZERO.top(3))];
            glow(button(face).padding(6).style(ui::ghost).on_press(Message::SeekBy(step)), 20.0)
        };
        if track.remember {
            buttons = buttons.push(jump(ui::REPLAY, -PODCAST_STEP));
        } else if !self.wave_queue {
            buttons = buttons.push(mode(ui::SHUFFLE, self.shuffle, Message::ShuffleToggle));
        }
        buttons = buttons.push(side(ui::PREVIOUS, Message::Previous)).push(toggle).push(side(ui::NEXT, Message::Next));
        if track.remember {
            buttons = buttons.push(jump(ui::FORWARD, PODCAST_STEP));
        } else if !self.wave_queue {
            let icon = if self.repeat == Repeat::One { ui::REPEAT_ONE } else { ui::REPEAT };
            buttons = buttons.push(mode(icon, self.repeat != Repeat::Off, Message::RepeatNext));
        }

        let lyrics = if track.lyrics { BAR_TOOL + 6.0 } else { 0.0 };
        let tools = lyrics + 3.0 * BAR_TOOL + 12.0;
        let slider_room = (zone - tools - BAR_VOLUME_ICON - 14.0).clamp(BAR_SLIDER_MIN, BAR_SLIDER_MAX);
        let mut right = row![self.view_tools(22.0, true)].spacing(8).align_y(Alignment::Center);
        // В узком окне громкость уезжает наверх вместе с названием: внизу ей не хватает места.
        if shown < 1.0 {
            let volume = container(self.view_volume(24.0, Some(slider_room))).width((BAR_VOLUME_ICON + 6.0 + slider_room) * (1.0 - shown)).clip(true);
            right = right.push(volume);
        }

        let controls = row![about, container(buttons).center_x(Length::Fill), container(right).align_right(zone)].spacing(16).align_y(Alignment::Center);

        // Строка сверху выезжает плавно и обрезается по краю окна: название слева, громкость справа.
        let top: Element<'_, Message> = if self.bar_top > 0.0 {
            let about = row![
                track_title(track, BAR_TOP_SIZE, ui::font_medium(), ui::WHITE.scale_alpha(shown), true),
                text("·").size(BAR_TOP_SIZE).color(ui::MUTED.scale_alpha(shown)),
                names(BAR_TOP_SIZE, shown),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            let volume = container(self.view_volume(BAR_TOP_VOLUME_ICON, Some(BAR_TOP_SLIDER))).width((BAR_TOP_VOLUME_ICON + 14.0 + BAR_TOP_SLIDER) * shown).clip(true);
            let line = row![container(about).width(Length::Fill).clip(true), volume].spacing(12).align_y(Alignment::Center);
            mouse_area(container(line).width(Length::Fill).height(BAR_TOP_LINE * shown).padding([0, 8]).clip(true).align_y(Alignment::Center)).on_right_press(Message::MenuOpen(track.clone())).into()
        } else {
            Space::new().height(0).into()
        };
        Some(
            container(column![
                top,
                container(self.seek_line(track, Length::Fill)).padding([0, 8]).height(SEEK_HEIGHT),
                container(controls).padding([0, 8]).center_y(BAR_HEIGHT - 4.0).width(Length::Fill),
            ])
            .padding(Padding::ZERO.top(4))
            .width(Length::Fill)
            .style(ui::player)
            .into(),
        )
    }
}

/// Всплывающая панель у курсора. Не выходит за край окна.
fn popup<'a>(content: Element<'a, Message>, x: f32, y: f32, width: f32, height: f32, alpha: f32, live: bool) -> Element<'a, Message> {
    let (window_width, window_height) = *WINDOW.lock().unwrap();
    let placed_x = x.min(window_width - width - 8.0).max(8.0);
    let placed_y = y.min(window_height - height - 24.0).max(8.0);
    // Меню раскрывается от точки клика: снизу вверх, если не поместилось вниз.
    let origin_x = if placed_x < x { 1.0 } else { 0.0 };
    let origin_y = if placed_y < y { 1.0 } else { 0.0 };
    let body = container(content).width(width).padding(8).style(ui::popover_faded(alpha));
    container(reveal(anim::block(body)).scale(0.92 + 0.08 * alpha, origin_x, origin_y).inert(!live))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding::new(0.0).left(placed_x).top(placed_y))
        .into()
}

/// Куда встанет трек, взятый с места `from`, если отпустить его над строкой `at` (выше или
/// ниже её середины). `None` — трек остаётся на месте.
fn drop_index(from: usize, at: usize, below: bool) -> Option<usize> {
    let slot = at + usize::from(below);
    let to = if slot > from { slot - 1 } else { slot };
    (to != from).then_some(to)
}

/// Страница из блоков: главная и книги с подкастами устроены одинаково.
fn is_landing(section: Section) -> bool {
    matches!(section, Section::Home | Section::Books | Section::Kids)
}

/// Номер страницы из блоков для запоминания её вкладки.
fn landing_index(section: Section) -> usize {
    match section {
        Section::Books => 1,
        Section::Kids => 2,
        _ => 0,
    }
}

/// Скелет страницы из блоков у Яндекса.
fn landing_skeleton(section: Section) -> &'static str {
    match section {
        Section::Books => "non_music",
        Section::Kids => "kids",
        _ => "main",
    }
}

/// Заголовок страницы из блоков.
fn landing_title(section: Section) -> &'static str {
    match section {
        Section::Books => "Книги и подкасты",
        Section::Kids => "Детям",
        _ => "Главная",
    }
}

/// Ошибка словами для человека. Внутренний текст с кодами и сообщениями системы
/// остаётся в журнале, а на экране — что случилось и что делать.
fn human_error(error: &str) -> String {
    const OFFLINE: &str = "Нет связи с Яндексом. Проверьте подключение к интернету";
    if api::network_error(error) {
        return OFFLINE.to_string();
    }
    match error.strip_prefix("Яндекс ответил ") {
        Some(code) if code.starts_with('5') => "Яндекс сейчас не отвечает. Попробуйте чуть позже".to_string(),
        Some(_) => "Яндекс не смог выполнить запрос. Попробуйте ещё раз".to_string(),
        None => error.to_string(),
    }
}

/// Подпись волны, запущенной от трека, альбома, исполнителя или плейлиста.
fn wave_label(seed: &str, name: &str) -> String {
    let kind = match seed.split(':').next() {
        Some("track") => "Моя волна по треку",
        Some("album") => "Моя волна по альбому",
        Some("artist") => "Моя волна по артисту",
        Some("playlist") => "Моя волна по плейлисту",
        _ => "Моя волна",
    };
    format!("{kind} · {name}")
}

/// Длинное название обрезается, чтобы не вылезать за край.
fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit - 1).collect::<String>().trim_end())
}

/// Отпечаток очереди: менялась ли она с прошлой записи.
fn queue_key(tracks: &[Track], title: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    title.hash(&mut hasher);
    tracks.iter().for_each(|track| track.id.hash(&mut hasher));
    hasher.finish()
}

/// Название трека. Есть альбом — ведёт к нему.
/// Название трека: не влезет — многоточие и полный текст в подсказке, а у играющего
/// (`marquee`) бегущая строка при наведении.
fn track_title<'a>(track: &'a Track, size: f32, font: iced::Font, color: iced::Color, marquee: bool) -> Element<'a, Message> {
    let label = fit(&track.title).size(size).font(font).reserve(if track.explicit { EXPLICIT_RESERVE } else { 0.0 });
    let label = if marquee { label.marquee() } else { label };
    let title: Element<'a, Message> = if track.album.is_some() {
        hover_line(label, color, if color == ui::ACCENT { ui::WHITE } else { ui::ACCENT }, Message::OpenAlbum(track.clone())).into()
    } else {
        label.color(color).into()
    };
    if !track.explicit {
        return title;
    }
    let badge = container(text("E").size(11).font(ui::font_medium()).color(ui::WHITE)).center(16).style(ui::explicit);
    row![title, badge].spacing(6).align_y(Alignment::Center).into()
}

/// Страницы, которые возвращаются на прежнее место. У текста песни, очереди и волны своя прокрутка.
fn remembers_scroll(section: Section) -> bool {
    !matches!(section, Section::Lyrics | Section::Queue | Section::Wave)
}

fn home_item_key(item: &HomeItem) -> String {
    match item {
        HomeItem::Album(card) => format!("{ALBUM_COVER}{}", card.id),
        HomeItem::Artist(card) => format!("{ARTIST_COVER}{}", card.id),
        HomeItem::Playlist(list) => playlist_cover_key(list),
        HomeItem::Wave { title, .. } => format!("{WAVE_CARD}{title}"),
        HomeItem::Likes { .. } => format!("{WAVE_CARD}:likes"),
        HomeItem::Mix { tag, .. } => format!("{WAVE_CARD}:mix:{tag}"),
        HomeItem::History { .. } => format!("{WAVE_CARD}:history"),
    }
}

fn home_item_cover(item: &HomeItem) -> Option<String> {
    match item {
        HomeItem::Album(card) | HomeItem::Artist(card) => card.cover.clone(),
        HomeItem::Playlist(list) => list.cover.clone(),
        HomeItem::Wave { cover, .. } | HomeItem::Likes { cover, .. } | HomeItem::History { cover, .. } | HomeItem::Mix { cover, .. } => cover.clone(),
    }
}

fn open_album(card: &Card) -> Message {
    Message::AlbumOpen(card.id.clone(), card.title.clone())
}

/// Заголовок блока со стрелкой, ведущий на полный список.
fn more_title<'a>(title: &'a str, press: Message) -> Element<'a, Message> {
    glow(button(row![text(title).size(24).font(ui::font_headline()).color(ui::TEXT), ui::icon(ui::CHEVRON, 24.0, ui::MUTED)].spacing(6).align_y(Alignment::Center))
        .padding([4, 8])
        .style(ui::ghost)
        .on_press(press), 20.0)
        .into()
}

fn disks_word(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, rest) if rest != 11 => "диск",
        (2..=4, rest) if !(12..=14).contains(&rest) => "диска",
        _ => "дисков",
    }
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

/// Без номера страницу исполнителя придётся искать по имени.
fn artist_message(name: &str, id: &str) -> Message {
    if id.is_empty() {
        Message::ArtistByName(name.to_string())
    } else {
        Message::ArtistOpen(id.to_string())
    }
}

/// Сначала точное совпадение имени, иначе первый результат поиска.
fn pick_artist(artists: &[Card], name: &str) -> Option<String> {
    artists.iter().find(|a| a.title.to_lowercase() == name.to_lowercase()).or(artists.first()).map(|a| a.id.clone())
}

/// Все исполнители через запятую, каждый ведёт на свою страницу.
/// Исполнители ссылками через запятую. Кто не влез, не показывается: в конце многоточие.
fn artist_links<'a>(artists: &[(String, String)], size: f32, font: iced::Font, comma: iced::Color) -> Element<'a, Message> {
    let items = artists
        .iter()
        .enumerate()
        .map(|(index, (name, id))| {
            // Пробел после запятой — часть ссылки: так ряд режется только целыми именами.
            let label = if index + 1 < artists.len() { format!("{name}, ") } else { name.clone() };
            hover_line(text(label).size(size).font(font).wrapping(Wrapping::None), comma, ui::WHITE, artist_message(name, id)).into()
        })
        .collect();
    fit_row(items, size, font, comma).into()
}

/// Заглушка строки, пока трек не загружен.
/// Движение с плавным началом и концом: t от 0 до 1.
fn smooth_step(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Подходит ли трек под поиск: по названию, исполнителям или альбому, без учёта регистра.
fn track_matches(track: &Track, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    [&track.title, &track.artists, &track.album_title].iter().any(|field| field.to_lowercase().contains(&filter))
}

fn skeleton_bar<'a>(width: f32, height: f32) -> iced::widget::Container<'a, Message> {
    container(Space::new()).width(width).height(height).style(ui::tile)
}

fn skeleton_round(radius: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |theme| container::Style { border: iced::border::rounded(radius), ..ui::tile(theme) }
}

fn skeleton_row<'a>() -> Element<'a, Message> {
    container(row![skeleton_bar(COVER, COVER), column![skeleton_bar(180.0, 12.0), skeleton_bar(110.0, 10.0)].spacing(8)].spacing(12).align_y(Alignment::Center))
        .width(Length::Fill)
        .height(ROW)
        .padding([8, 8])
        .into()
}

/// Рамка скелетона: обрезает лишнее по краю панели. Обычный контейнер сжал бы последние
/// строки под оставшуюся высоту, а прокручиваемая область без полосы отдаёт им полный размер.
fn skeleton_frame<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    let bar = iced::widget::scrollable::Scrollbar::new().width(0).scroller_width(0).margin(0);
    iced::widget::Scrollable::new(container(content).padding(Padding::ZERO.right(SCROLLBAR)).width(Length::Fill))
        .direction(iced::widget::scrollable::Direction::Vertical(bar))
        .height(Length::Fill)
        .into()
}

fn skeleton_list<'a>() -> Element<'a, Message> {
    skeleton_frame((0..SKELETON_ROWS).fold(column![], |list, _| list.push(skeleton_row())))
}

/// Те же отступы и размеры, что у настоящей карточки: обложка и две строки текста.
fn skeleton_card<'a>() -> Element<'a, Message> {
    let line = |width, height, room| container(skeleton_bar(width, height)).center_y(room);
    container(column![skeleton_bar(CARD, CARD).style(skeleton_round(8.0)), line(130.0, 13.0, SKELETON_TITLE), line(90.0, 10.0, SKELETON_SUBTITLE)].spacing(6).width(CARD)).padding(8).into()
}

fn skeleton_cards<'a>() -> Element<'a, Message> {
    skeleton_frame((0..SKELETON_ROWS).fold(row![].spacing(4), |cards, _| cards.push(skeleton_card())).wrap())
}

fn skeleton_home<'a>(in_row: usize) -> Element<'a, Message> {
    skeleton_frame((0..SKELETON_BLOCKS).fold(column![].spacing(12), |blocks, _| {
        let head = container(skeleton_bar(220.0, 20.0)).center_y(SKELETON_HEAD);
        let cards = (0..in_row).fold(row![].spacing(4), |cards, _| cards.push(skeleton_card()));
        blocks.push(column![head, cards].spacing(8)).push(Space::new().height(8))
    }))
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
        let back = event::listen_with(|event, status, _| match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Back)) => Some(Message::Back),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if DRAGGING.load(Ordering::Acquire) => Some(Message::DragEnd),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Forward)) => Some(Message::Forward),
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                *CURSOR.lock().unwrap() = (position.x, position.y);
                None
            }
            Event::Window(window::Event::Resized(size)) => Some(Message::WindowSize(size)),
            Event::Window(window::Event::CloseRequested) => Some(Message::Quit),
            Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Escape), .. }) => Some(Message::MenuClose),
            Event::Keyboard(keyboard::Event::KeyPressed { key, physical_key, modifiers, .. }) if status == event::Status::Ignored => hotkey(&key, physical_key, modifiers),
            _ => None,
        });
        Subscription::batch([self, back, Subscription::run(media::events).map(Message::Media)])
    }
}

/// Цвета неотличимы на глаз: переход можно закончить.
fn same_color(a: iced::Color, b: iced::Color) -> bool {
    (a.r - b.r).abs().max((a.g - b.g).abs()).max((a.b - b.b).abs()) < 0.004
}

/// Основной цвет обложки по каждой десятой точке. Яркие точки весят больше серых:
/// простое среднее смешало бы цвета в бурый, а серая обложка и так останется серой.
fn average_color(handle: &image::Handle) -> Option<iced::Color> {
    let image::Handle::Rgba { pixels, .. } = handle else { return None };
    let (mut sum, mut weights) = ([0f32; 3], 0f32);
    for pixel in pixels.chunks_exact(4).step_by(10) {
        let (high, low) = (pixel[..3].iter().max()?, pixel[..3].iter().min()?);
        let weight = (high - low) as f32 / 255.0 + 0.08;
        for (total, value) in sum.iter_mut().zip(pixel) {
            *total += *value as f32 * weight;
        }
        weights += weight;
    }
    let part = |total: f32| total / weights / 255.0;
    (weights > 0.0).then(|| iced::Color::from_rgb(part(sum[0]), part(sum[1]), part(sum[2])))
}

/// Готовые пиксели обложки. Сжатую картинку iced распаковал бы прямо во время кадра,
/// и при смене страницы с десятком новых обложек анимация спотыкалась бы.
fn decoded(bytes: &[u8]) -> Option<image::Handle> {
    let picture = ::image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = picture.dimensions();
    Some(image::Handle::from_rgba(width, height, picture.into_raw()))
}

/// Клавиши как в официальном клиенте. Буквы берутся по месту на клавиатуре,
/// поэтому работают и в русской раскладке.
fn hotkey(key: &keyboard::Key, physical: keyboard::key::Physical, modifiers: keyboard::Modifiers) -> Option<Message> {
    use keyboard::key::{Code, Named, Physical};
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    if let keyboard::Key::Named(named) = key {
        let message = match named {
            Named::Space => Message::Toggle,
            Named::ArrowRight => Message::SeekBy(SEEK_STEP),
            Named::ArrowLeft => Message::SeekBy(-SEEK_STEP),
            Named::ArrowUp => Message::VolumeBy(VOLUME_STEP),
            Named::ArrowDown => Message::VolumeBy(-VOLUME_STEP),
            _ => return None,
        };
        return Some(message);
    }
    let Physical::Code(code) = physical else { return None };
    Some(match code {
        Code::KeyK => Message::Toggle,
        Code::KeyL => Message::SeekBy(SEEK_STEP),
        Code::KeyJ => Message::SeekBy(-SEEK_STEP),
        Code::KeyN => Message::Next,
        Code::KeyP => Message::Previous,
        Code::KeyM => Message::ToggleMute,
        Code::KeyF => Message::LikeCurrent,
        Code::KeyD => Message::DislikeCurrent,
        Code::KeyR => Message::RepeatNext,
        Code::KeyS => Message::ShuffleToggle,
        Code::KeyW => Message::FullToggle,
        _ => return None,
    })
}

/// Список для настроек: группа, действие и клавиши.
const HOTKEYS: [(&str, &[(&str, &[&str])]); 4] = [
    ("Воспроизведение", &[
        ("Пауза и продолжение", &["Пробел", "K"]),
        ("Следующий трек", &["N"]),
        ("Предыдущий трек", &["P"]),
        ("Перемотать вперёд на 5 секунд", &["→", "L"]),
        ("Перемотать назад на 5 секунд", &["←", "J"]),
        ("Режим повтора", &["R"]),
        ("Случайный порядок", &["S"]),
    ]),
    ("Громкость", &[("Громче", &["↑"]), ("Тише", &["↓"]), ("Выключить или включить звук", &["M"])]),
    ("Оценки", &[("Лайк играющему треку", &["F"]), ("Дизлайк играющему треку", &["D"])]),
    ("Окна и страницы", &[
        ("Предыдущая страница", &["Мышь: боковая «назад»"]),
        ("Следующая страница", &["Мышь: боковая «вперёд»"]),
        ("Закрыть меню или окно", &["Esc"]),
    ]),
];

fn keycap<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(13).font(ui::font_medium()).color(ui::TEXT))
        .padding([3, 9])
        .style(|_| container::Style::default().background(ui::TEXT.scale_alpha(0.08)).border(iced::border::rounded(6.0).width(1.0).color(ui::TEXT.scale_alpha(0.14))))
        .into()
}

fn view_hotkeys<'a>() -> Element<'a, Message> {
    let line = || container(Space::new()).width(Length::Fill).height(1).style(|_| container::Style::default().background(ui::TEXT.scale_alpha(0.06)));
    HOTKEYS
        .iter()
        .fold(column![text("Горячие клавиши").size(20).font(ui::font_medium()).color(ui::TEXT)].spacing(12), |page, (group, items)| {
            let rows = items.iter().enumerate().fold(column![], |rows, (index, (action, keys))| {
                let caps = keys.iter().enumerate().fold(row![].spacing(6).align_y(Alignment::Center), |caps, (at, key)| {
                    let caps = if at > 0 { caps.push(text("или").size(12).color(ui::MUTED)) } else { caps };
                    caps.push(keycap(key))
                });
                let rows = if index > 0 { rows.push(line()) } else { rows };
                rows.push(container(row![text(*action).color(ui::TEXT).width(Length::Fill), caps].spacing(16).align_y(Alignment::Center)).padding([9, 16]))
            });
            let card = container(rows).width(Length::Fill).style(|_| container::Style::default().background(ui::TEXT.scale_alpha(0.035)).border(iced::border::rounded(12.0)));
            page.push(column![text(*group).size(13).font(ui::font_medium()).color(ui::MUTED), card].spacing(6))
        })
        .into()
}

/// Переключатель. Пока значение неизвестно, он блёклый и стоит в положении «выключено».
/// Эквалайзер встаёт над трекбаром: низ окна над кнопкой, а не поверх неё.
fn eq_place((x, y): (f32, f32)) -> (f32, f32) {
    (x, (y - EQ_POPUP_LIFT - EQ_POPUP_HEIGHT).max(8.0))
}

/// Пресет эквалайзера в выпадающем списке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Preset(usize);

/// Сезон подкаста в списке выбора; `None` — все сезоны.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Season(Option<usize>);

impl std::fmt::Display for Season {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(season) => write!(f, "Сезон {}", season + 1),
            None => f.write_str("Все сезоны"),
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(equalizer::PRESETS[self.0].0)
    }
}

fn view_switch<'a>(state: Option<bool>) -> Element<'a, Message> {
    let on = state == Some(true);
    let (back, front) = if on { (ui::ACCENT, ui::ON_ACCENT) } else { (ui::CONTROL, ui::MUTED) };
    let alpha = if state.is_some() { 1.0 } else { 0.4 };
    let knob = container(Space::new()).width(18).height(18).style(move |_| container::Style::default().background(front.scale_alpha(alpha)).border(iced::border::rounded(9.0)));
    container(knob)
        .width(40)
        .height(22)
        .padding(2)
        .align_x(if on { Alignment::End } else { Alignment::Start })
        .style(move |_| container::Style::default().background(back.scale_alpha(alpha)).border(iced::border::rounded(11.0)))
        .into()
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

/// «03:25», а у выпусков и глав длиннее часа — «1:47:20».
fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        return format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60);
    }
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Слово в нужном числе: один выпуск, два выпуска, пять выпусков.
fn plural(n: usize, forms: [&'static str; 3]) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=14) => forms[2],
        (1, _) => forms[0],
        (2..=4, _) => forms[1],
        _ => forms[2],
    }
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
    println!("Файл: {}, зашифрован: {}", info.label(), info.key.is_some());
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

/// Проверка волн: что сервер отдаёт на каждое зерно и сколько треков совпало с Моей волной.
fn smoke_wave(seeds: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let mine: HashSet<String> = api::wave_new(&[api::MY_WAVE.to_string()], &token)?.tracks.into_iter().map(|t| t.id).collect();
    println!("{}: {} треков", api::MY_WAVE, mine.len());
    for seed in seeds.split(',') {
        // Плюс объединяет зёрна в одну волну.
        let asked: Vec<String> = seed.split('+').map(str::to_string).collect();
        match api::wave_new(&asked, &token) {
            Ok(wave) => {
                let same = wave.tracks.iter().filter(|t| mine.contains(&t.id)).count();
                println!("{seed}: {} треков, совпало с Моей волной: {same}", wave.tracks.len());
                for track in wave.tracks.iter().take(4) {
                    println!("  {} — {}", track.artists, track.title);
                }
            }
            Err(e) => println!("{seed}: ошибка {e}"),
        }
    }
    Ok(())
}

/// Проверка текста: только число строк, сам текст не печатаем.
fn smoke_lyrics(ids: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    for id in ids.split(',') {
        match api::lyrics(id, &token)? {
            Some(lyrics) => println!("{id}: строк {}, синхронный: {}, авторов {}", lyrics.lines.len(), lyrics.synced(), lyrics.authors.len()),
            None => println!("{id}: текста нет"),
        }
    }
    Ok(())
}

/// Проверка без окна и без звука: каждое качество запускается и перематывается.
/// Переключает «Меньше ИИ» туда и обратно и каждый раз читает, что запомнил сервер.
fn smoke_settings() -> Result<(), String> {
    let token = api::load_token().ok_or("нет токена")?;
    let before = api::ai_reduction(&token)?;
    println!("Было: {before}");
    api::set_ai_reduction(!before, &token)?;
    println!("После переключения: {}", api::ai_reduction(&token)?);
    api::set_ai_reduction(before, &token)?;
    println!("После возврата: {}", api::ai_reduction(&token)?);
    Ok(())
}

/// Печатает блоки обеих вкладок главной: что из ответа Яндекса дошло до экрана.
fn smoke_home() -> Result<(), String> {
    let token = api::load_token().ok_or("нет токена")?;
    for (skeleton, tab) in [("main", 0), ("main", 1), ("non_music", 0), ("non_music", 1), ("kids", 0)] {
        let home = api::landing(skeleton, tab, &token)?;
        println!("=== {}", home.tabs.get(tab).cloned().unwrap_or_default());
        for block in &home.blocks {
            let kinds: Vec<String> = block.items.iter().take(4).map(|item| format!("{item:?}").split(['{', '(', ' ']).next().unwrap_or_default().to_string()).collect();
            println!("{} | карточек {} | треков {} | вкладок {} | {}", block.title, block.items.len(), block.tracks.len(), block.tabs.len(), kinds.join(","));
        }
    }
    let items = api::category("category_non-music_comedy", &token)?;
    println!("=== Категория «Юмор»: {} карточек, первые: {}", items.len(), items.iter().take(3).map(|item| format!("{item:?}").chars().take(60).collect::<String>()).collect::<Vec<_>>().join(" | "));
    Ok(())
}

/// Шлёт стартовый отчёт о прослушивании для трека, на котором остановилась прошлая очередь.
fn smoke_plays() -> Result<(), String> {
    let token = api::load_token().ok_or("нет токена")?;
    let saved = session::load().ok_or("нет сохранённой очереди")?;
    let track = saved.tracks.get(saved.current).ok_or("нет трека")?;
    let mut listen = api::Listen::new(track, "desktop-main_screen-default".into(), "other", None);
    listen.started = Some(api::now_stamp());
    listen.restored = true;
    api::plays(vec![listen.report(0.0, None, true)], &token)?;
    println!("Яндекс принял отчёт: {} — {}", track.artists, track.title);
    Ok(())
}

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
        println!("{}: {}, позиция {played:?}, после перемотки {:?}", quality.code(), info.label(), player.position());
    }
    let info = api::file_info(&track.id, Quality::Normal.code(), &token)?;
    player.preload("проверка", &info)?;
    let generation = player.stop();
    let started = std::time::Instant::now();
    player.play_prepared(generation, "проверка").ok_or("Подготовленный файл потерялся")??;
    println!("Запуск подготовленного файла: {:?}", started.elapsed());
    std::thread::sleep(Duration::from_secs(1));
    player.hold_next(true);
    let generation = player.stop();
    player.play(generation, &info)?;
    std::thread::sleep(Duration::from_secs(1));
    println!("Запуск на паузе: пауза {}, позиция {:?}", player.is_paused(), player.position());
    player.seek(Duration::from_secs(60))?;
    println!("Перемотка на паузе: позиция {:?}", player.position());
    player.toggle();
    std::thread::sleep(Duration::from_secs(1));
    println!("После продолжения: пауза {}, позиция {:?}", player.is_paused(), player.position());
    std::thread::sleep(Duration::from_secs(2));
    println!("Позиция: {:?}", player.position());
    Ok(())
}

/// Обложка плейлиста: временный плейлист получает картинку 3000×3000 (уходит уменьшенной),
/// потом удаляется.
fn smoke_cover() -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let uid = api::profile(&token)?.uid;
    let path = std::env::temp_dir().join("tune-native-cover-check.png");
    let picture = ::image::RgbImage::from_fn(3000, 3000, |x, y| ::image::Rgb([(x / 12) as u8, (y / 12) as u8, 160]));
    picture.save(&path).map_err(|e| e.to_string())?;
    let (bytes, mime) = api::cover_file(&path)?;
    let side = ::image::load_from_memory(&bytes).map_err(|e| e.to_string())?.width();
    println!("К загрузке: {mime}, {} КБ, сторона {side}", bytes.len() / 1024);
    let list = api::create_playlist(&uid, &format!("Проверка Tune {}", random_seed()), &token)?;
    let result = api::upload_cover(&uid, &list.kind, bytes, mime, &token);
    let shown = api::playlists(&uid, &token).ok().and_then(|lists| lists.into_iter().find(|p| p.kind == list.kind)).and_then(|p| p.cover);
    api::delete_playlist(&uid, &list.kind, &token)?;
    let _ = std::fs::remove_file(path);
    println!("Яндекс вернул обложку: {:?}", result?);
    println!("В списке плейлистов: {shown:?}");
    println!("Временный плейлист удалён");
    Ok(())
}

/// Перестановка трека в плейлисте: временный плейлист из трёх треков, первый уезжает в конец,
/// потом плейлист удаляется.
fn smoke_move(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let uid = api::profile(&token)?.uid;
    let tracks: Vec<Track> = api::search(query, &token)?.into_iter().take(3).collect();
    if tracks.len() < 3 {
        return Err("Нужно три трека".into());
    }
    let list = api::create_playlist(&uid, &format!("Проверка Tune {}", random_seed()), &token)?;
    let result = (|| {
        for track in &tracks {
            api::add_to_playlist(&uid, &list.kind, track, &token)?;
        }
        let titles = |tracks: Vec<Track>| tracks.into_iter().map(|t| t.title).collect::<Vec<_>>().join(" | ");
        let before = api::playlist_tracks(&list, &token)?;
        println!("До:    {}", titles(before.clone()));
        api::move_in_playlist(&uid, &list.kind, &before[0], 0, 2, &token)?;
        let after = api::playlist_tracks(&list, &token)?;
        println!("После: {}", titles(after.clone()));
        println!("Первый уехал в конец: {}", after.last().map(|t| &t.id) == Some(&before[0].id) && after[0].id == before[1].id);
        Ok::<(), String>(())
    })();
    api::delete_playlist(&uid, &list.kind, &token)?;
    println!("Временный плейлист удалён");
    result
}

/// Переход между двумя найденными треками: старый должен затихнуть и кончиться на своей
/// точке, новый — играть дальше. Звук выключен.
fn smoke_crossfade(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let tracks: Vec<Track> = api::search(query, &token)?.into_iter().filter(|track| track.fade.is_some()).take(2).collect();
    let [first, second] = &tracks[..] else { return Err("Нужны два трека с точками перехода".into()) };
    let (out, fade_in) = (first.fade.unwrap(), second.fade.unwrap());
    println!("{} — {}: затухание {:.1}–{:.1} с", first.artists, first.title, out.out_start, out.out_stop);
    println!("{} — {}: нарастание {:.1}–{:.1} с", second.artists, second.title, fade_in.in_start, fade_in.in_stop);
    let player = Player::new()?;
    player.set_volume(0.0);
    let generation = player.stop();
    player.play(generation, &api::file_info(&first.id, Quality::Normal.code(), &token)?)?;
    player.preload("следующий", &api::file_info(&second.id, Quality::Normal.code(), &token)?)?;
    player.seek(Duration::from_secs_f32((out.out_start - 1.0).max(0.0)))?;
    std::thread::sleep(Duration::from_secs(1));
    let generation = player.crossfade((out.out_start, out.out_stop), (fade_in.in_start, fade_in.in_stop));
    player.play_prepared(generation, "следующий").ok_or("Подготовленный файл потерялся")??;
    let length = out.out_stop - out.out_start;
    for step in 0..=((length + 2.0) * 2.0) as usize {
        println!("{:4.1} с: старый звучит {}, новый на {:?}", step as f32 / 2.0, player.fading(), player.position());
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// Проверка без окна: скачать трек, найти его в списке, сыграть с диска и удалить.
fn smoke_download(query: &str) -> Result<(), String> {
    // «keep:запрос» оставляет трек скачанным, чтобы посмотреть страницу; «clear» убирает всё.
    if query == "clear" {
        offline::load().iter().for_each(offline::remove);
        offline::save(&[]);
        println!("Скачанное очищено");
        return Ok(());
    }
    let (query, keep) = match query.strip_prefix("keep:") {
        Some(rest) => (rest, true),
        None => (query, false),
    };
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let track = api::search(query, &token)?.into_iter().find(|track| !track.unavailable).ok_or("Ничего не нашлось")?;
    let started = std::time::Instant::now();
    let saved = offline::download(&track, Quality::Normal.code(), &token)?;
    println!("{} — {}: {} за {:.1} с, {}, файл есть: {}", track.artists, track.title, offline::size_label(saved.bytes), started.elapsed().as_secs_f32(), saved.sound().label, saved.path().exists());
    let mut list = offline::load();
    list.retain(|old| old.track.id != saved.track.id);
    list.insert(0, saved.clone());
    offline::save(&list);
    println!("В списке после перечитывания: {}", offline::load().iter().any(|item| item.track.id == track.id));
    println!("Обложка с диска: {}", offline::cover(&track.id).map_or(0, |bytes| bytes.len()));
    let player = Player::new()?;
    player.set_volume(0.0);
    let generation = player.stop();
    player.play_file(generation, &saved.path(), &saved.codec)?;
    std::thread::sleep(Duration::from_secs(2));
    println!("Играет с диска: место {:?}", player.position());
    player.seek(Duration::from_secs(30))?;
    std::thread::sleep(Duration::from_millis(500));
    println!("Перемотка по файлу: {:?}", player.position());
    player.stop();
    println!("Файл после остановки на месте: {}", saved.path().exists());
    if keep {
        return Ok(());
    }
    offline::remove(&saved);
    list.retain(|old| old.track.id != saved.track.id);
    offline::save(&list);
    println!("Удалён: файл {}, обложка {}", !saved.path().exists(), offline::cover(&track.id).is_none());
    Ok(())
}

/// Проверка отложенных отчётов: `keep` — без сети отчёт о скачанном треке ложится на диск,
/// `flush` — с сетью уходит.
fn smoke_offline(step: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    if step == "flush" {
        println!("Ждут отправки: {}", offline::plays_waiting());
        offline::flush_plays(token);
        std::thread::sleep(Duration::from_secs(4));
        println!("После отправки ждут: {}", offline::plays_waiting());
        return Ok(());
    }
    let saved = offline::load().into_iter().next().ok_or("Сначала скачайте трек")?;
    let mut listen = api::Listen::new(&saved.track, "desktop-downloads_screen-default".into(), "other", None);
    listen.started = Some(api::now_stamp());
    let report = listen.report(0.0, None, true);
    match api::plays(vec![report.clone()], &token) {
        Err(e) if api::network_error(&e) => {
            offline::keep_plays(vec![report]);
            println!("Сети нет, отчёт отложен. Ждут отправки: {}", offline::plays_waiting());
        }
        other => println!("Отчёт ушёл сразу: {other:?}"),
    }
    Ok(())
}

/// Проверка без окна: перемотка на паузе сдвигает место и не снимает паузу.
fn smoke_pause_seek(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let track = api::search(query, &token)?.into_iter().next().ok_or("Ничего не нашлось")?;
    let player = Player::new()?;
    player.set_volume(0.0);
    let generation = player.stop();
    player.play(generation, &api::file_info(&track.id, Quality::Normal.code(), &token)?)?;
    std::thread::sleep(Duration::from_secs(2));
    player.toggle();
    println!("Пауза: {}, место {:?}", player.is_paused(), player.position());
    for step in [15.0, 30.0, 10.0] {
        player.seek(Duration::from_secs_f32(step))?;
        std::thread::sleep(Duration::from_millis(500));
        println!("Перемотка на {step} с: место {:?}, пауза {}", player.position(), player.is_paused());
    }
    player.toggle();
    std::thread::sleep(Duration::from_secs(1));
    println!("После снятия паузы: место {:?}", player.position());
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
    let name = format!("Проверка Tune {}", random_seed());
    let list = api::create_playlist(&uid, &name, &token)?;
    let renamed = format!("{name} 2");
    api::rename_playlist(&uid, &list.kind, &renamed, &token)?;
    let checked = api::playlists(&uid, &token)?.iter().any(|p| p.kind == list.kind && p.title == renamed);
    api::add_to_playlist(&uid, &list.kind, &track, &token)?;
    let added = api::playlist_tracks(&list, &token)?.iter().any(|t| t.id == track.id);
    api::remove_from_playlist(&uid, &list.kind, &track, &token)?;
    let removed = api::playlist_tracks(&list, &token)?.is_empty();
    api::set_description(&uid, &list.kind, "описание", &token)?;
    api::set_visibility(&uid, &list.kind, "public", &token)?;
    println!("Описание и приватность: {:?}", api::playlist_meta(&uid, &list.kind, &token)?);
    let target = PinRef::Playlist(uid.clone(), list.kind.clone());
    api::set_pinned(&target, true, &token)?;
    let pinned = api::pins(&token)?.iter().any(|pin| target.matches(pin));
    api::set_pinned(&target, false, &token)?;
    println!("Закрепление: появилось {pinned}, убралось {}", !api::pins(&token)?.iter().any(|pin| target.matches(pin)));
    api::delete_playlist(&uid, &list.kind, &token)?;
    let gone = !api::playlists(&uid, &token)?.iter().any(|p| p.kind == list.kind);
    println!("Плейлист: название сменилось {checked}, трек добавился {added}, убрался {removed}, плейлист удалён {gone}");
    println!("История: {} треков", api::history(HISTORY_LIMIT, &token)?.len());
    println!("Дизлайков показать: {}", api::disliked_tracks(&uid, HISTORY_LIMIT, &token)?.len());
    let liked_before = api::liked_playlists(&uid, &token)?;
    if let Some(other) = found.playlists.iter().find(|p| p.owner != uid && !liked_before.iter().any(|l| l.kind == p.kind && l.owner == p.owner)) {
        api::set_playlist_liked(&uid, &other.owner, &other.kind, true, &token)?;
        let added = api::liked_playlists(&uid, &token)?.iter().any(|l| l.kind == other.kind && l.owner == other.owner);
        api::set_playlist_liked(&uid, &other.owner, &other.kind, false, &token)?;
        println!("Лайк плейлиста: появился {added}, снялся {}", api::liked_playlists(&uid, &token)?.len() == liked_before.len());
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
        [_, flag, query] if flag == "--smoke" || flag == "--smoke-library" || flag == "--smoke-quality" || flag == "--smoke-wave" || flag == "--smoke-lyrics" || flag == "--smoke-settings" || flag == "--smoke-plays" || flag == "--smoke-home" || flag == "--smoke-crossfade" || flag == "--smoke-move" || flag == "--smoke-cover" || flag == "--smoke-pause-seek" || flag == "--smoke-download" || flag == "--smoke-offline" => {
            let result = match flag.as_str() {
                "--smoke" => smoke(query),
                "--smoke-library" => smoke_library(query),
                "--smoke-wave" => smoke_wave(query),
                "--smoke-lyrics" => smoke_lyrics(query),
                "--smoke-settings" => smoke_settings(),
                "--smoke-plays" => smoke_plays(),
                "--smoke-home" => smoke_home(),
            "--smoke-crossfade" => smoke_crossfade(query),
            "--smoke-move" => smoke_move(query),
            "--smoke-cover" => smoke_cover(),
            "--smoke-pause-seek" => smoke_pause_seek(query),
            "--smoke-download" => smoke_download(query),
            "--smoke-offline" => smoke_offline(query),
                _ => smoke_quality(query),
            };
            if let Err(e) = result {
                println!("Ошибка: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        // Ссылка в командной строке открывается как и в поиске.
        [_, link] if api::parse_link(link).is_some() => Some(Preview::Search(link.clone())),
        [_, flag, what] if flag == "--preview" => Some(match what.as_str() {
            "wave" => Preview::Wave,
            "home" => Preview::Home,
            "home-trends" => Preview::HomeTrends,
            "history" => Preview::History,
            "dislikes" => Preview::Dislikes,
            "dialog" => Preview::Dialog,
            "wave-seek" => Preview::WaveSeek,
            "wave-seeded" => Preview::WaveSeeded,
            "wave-tune" => Preview::WaveTune,
            "lyrics" => Preview::Lyrics,
            "likes" => Preview::Likes,
            "likes-end" => Preview::LikesEnd,
            "panels" => Preview::Panels,
            "queue-jump" => Preview::QueueJump,
            "menu" => Preview::Menu,
            "track-info" => Preview::TrackInfo,
            "account" => Preview::Account,
            "settings" => Preview::Settings,
            "restored" => Preview::Restored,
            "logout-ask" => Preview::LogoutAsk,
            "pin-menu" => Preview::PinMenu,
            "pin-play-0" => Preview::PinPlay(0),
            "pin-play-1" => Preview::PinPlay(1),
            "pin-play-2" => Preview::PinPlay(2),
            "collection" => Preview::Collection,
            "playlists" => Preview::Playlists,
            "playlist" => Preview::Playlist,
            "page-menu" => Preview::PageMenu,
            "playlist-home" => Preview::PlaylistHome,
            what if what.starts_with("filter:") => Preview::Filter(what[7..].to_string()),
            what if what.starts_with("similar-album:") => Preview::SimilarOf(Playlist::album(&what[14..], "")),
            what if what.starts_with("similar-list:") => {
                let (owner, kind) = what[13..].split_once('_').unwrap_or_default();
                Preview::SimilarOf(Playlist { source: Source::Playlist, owner: owner.to_string(), kind: kind.to_string(), title: "Плейлист".to_string(), count: 1, cover: None })
            }
            what if what.starts_with("album:") => Preview::Album(what[6..].to_string()),
            what if what.starts_with("artist:") => Preview::Artist(what[7..].to_string()),
            what if what.starts_with("artist-about:") => Preview::ArtistAbout(what[13..].to_string()),
            what if what.starts_with("discography:") => Preview::Discography(what[12..].to_string()),
            what if what.starts_with("tag:") => Preview::Tag(what[4..].to_string()),
            "search-history" => Preview::SearchHistory,
            "search-tabs" => Preview::SearchTabs(1),
            "search-focus" => Preview::SearchFocus,
            "scroll-return" => Preview::ScrollReturn,
            "full" => Preview::Full,
            "eq-bar" => Preview::EqBar,
            "books" => Preview::Books,
            "downloads" => Preview::Downloads,
            "kids" => Preview::Kids,
            "wave-restored" => Preview::WaveRestored,
            "filter-focus" => Preview::FilterFocus,
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
        .window(window::Settings { size: Size::new(1280.0, 820.0), min_size: Some(Size::new(900.0, 600.0)), exit_on_close_request: false, ..window::Settings::default() })
        .run()
}

#[cfg(test)]
mod lyrics_tests {
    use super::*;

    #[test]
    fn counts_disks() {
        assert_eq!(disks_word(1), "диск");
        assert_eq!(disks_word(3), "диска");
        assert_eq!(disks_word(5), "дисков");
        assert_eq!(disks_word(11), "дисков");
        assert_eq!(disks_word(22), "диска");
    }

    #[test]
    fn return_starts_and_ends_softly() {
        assert_eq!(smooth_step(0.0), 0.0);
        assert_eq!(smooth_step(1.0), 1.0);
        assert_eq!(smooth_step(0.5), 0.5);
        assert!(smooth_step(0.05) < 0.05);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_times_show_hours() {
        assert_eq!(clock(Duration::from_secs(205)), "03:25");
        assert_eq!(clock(Duration::from_secs(6440)), "1:47:20");
    }

    #[test]
    fn words_agree_with_numbers() {
        let forms = ["выпуск", "выпуска", "выпусков"];
        assert_eq!([1, 2, 5, 11, 21, 22, 112].map(|n| plural(n, forms)), ["выпуск", "выпуска", "выпусков", "выпусков", "выпуск", "выпуска", "выпусков"]);
    }

    #[test]
    fn drop_lands_where_the_line_is() {
        // Вниз: над нижней половиной третьей строки — встаёт третьей.
        assert_eq!(drop_index(0, 2, true), Some(2));
        assert_eq!(drop_index(0, 2, false), Some(1));
        // Вверх: над верхней половиной первой строки — встаёт первой.
        assert_eq!(drop_index(3, 0, false), Some(0));
        assert_eq!(drop_index(3, 0, true), Some(1));
        // Рядом с собой — без перестановки.
        assert_eq!(drop_index(2, 2, true), None);
        assert_eq!(drop_index(2, 1, true), None);
        assert_eq!(drop_index(2, 3, false), None);
    }

    #[test]
    fn errors_read_like_words() {
        let offline = "Не удалось связаться с Яндексом: io: Подключение не установлено (os error 10061)";
        assert_eq!(human_error(offline), "Нет связи с Яндексом. Проверьте подключение к интернету");
        assert_eq!(human_error("Яндекс ответил 503: ошибка"), "Яндекс сейчас не отвечает. Попробуйте чуть позже");
        assert_eq!(human_error("Яндекс ответил 400: validate"), "Яндекс не смог выполнить запрос. Попробуйте ещё раз");
        assert_eq!(human_error("Здесь пока нет треков"), "Здесь пока нет треков");
    }

    #[test]
    fn picks_artist_by_name() {
        let card = |id: &str, title: &str| Card { id: id.to_string(), title: title.to_string(), subtitle: String::new(), cover: None };
        let found = [card("1", "Lida Band"), card("2", "LIDA")];
        assert_eq!(pick_artist(&found, "lida").as_deref(), Some("2"));
        assert_eq!(pick_artist(&found, "другой").as_deref(), Some("1"));
        assert_eq!(pick_artist(&[], "lida"), None);
    }

    #[test]
    fn labels_wave_by_source() {
        assert_eq!(wave_label("track:5", "Песня"), "Моя волна по треку · Песня");
        assert_eq!(wave_label("playlist:1_2", "Лист"), "Моя волна по плейлисту · Лист");
        assert_eq!(clipped("короткое", 56), "короткое");
        assert_eq!(clipped("один два три", 6), "один…");
    }

    #[test]
    fn groups_digits() {
        assert_eq!(grouped(4733421), "4 733 421");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1 000");
    }

    #[test]
    fn names_codecs() {
        assert_eq!(api::codec_name("flac-mp4"), "FLAC");
        assert_eq!(api::codec_name("he-aac-mp4"), "HE-AAC");
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
