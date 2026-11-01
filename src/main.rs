#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod login;
mod media;
mod player;
mod ui;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::channel::{mpsc, oneshot};
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, image, operation, row, scrollable, slider, text, text_input, Id, Space};
use iced::{time, window, Alignment, Element, Length, Padding, Size, Subscription, Task};

use api::{Playlist, Track, Wave, NEED_LOGIN};
use media::Media;
use player::Player;
use raw_window_handle::RawWindowHandle;

const TICK: Duration = Duration::from_millis(500);
const COVER: f32 = 40.0;
const BAR_COVER: f32 = 56.0;
const SIDEBAR: f32 = 200.0;
const NUMBER: f32 = 28.0;
const LIKE: f32 = 30.0;
/// Постоянное место под полосу прокрутки: колонка времени не прыгает на коротких списках.
const SCROLLBAR: f32 = 14.0;
/// Сдвиг заголовка, чтобы «Время» совпало с колонкой в строках.
const LIST_GUTTER: f32 = 2.0;
const SEARCH_INPUT: &str = "search";
const COVER_WORKERS: usize = 6;
const LIKES_PAGE: usize = 100;
/// Сколько треков до конца очереди оставить, прежде чем просить у волны новые.
const WAVE_MARGIN: usize = 2;
const WAVE_HISTORY: usize = 50;
/// Сколько сбойных треков подряд пропустить, прежде чем остановиться: сеть могла пропасть.
const MAX_FAILURES: u32 = 3;
const PLAYLIST_PAGE: usize = 100;
const PLAYLIST_COVER: &str = "playlist:";

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
    Likes,
    Playlists,
    Search,
}

/// Запуск с готовым разделом: первый трек показывается в плеере, но не играет.
#[derive(Clone)]
enum Preview {
    Wave,
    Likes,
    Playlists,
    /// Сразу открыть первый плейлист.
    Playlist,
    Search(String),
}

struct App {
    screen: Screen,
    section: Section,
    query: String,
    results: Vec<Track>,
    searching: bool,
    search_id: u64,
    uid: Option<String>,
    liked_ids: Vec<String>,
    liked_set: HashSet<String>,
    liked: Vec<Track>,
    likes_loading: bool,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    /// Открытый плейлист и его треки; показывается только начало списка.
    playlist: Option<Playlist>,
    playlist_tracks: Vec<Track>,
    playlist_shown: usize,
    covers: HashMap<String, image::Handle>,
    media: Option<Media>,
    failures: u32,
    player: Option<Player>,
    /// Что играет: смена раздела и новый поиск не прерывают музыку.
    queue: Vec<Track>,
    /// Сессия волны, если очередь — это волна.
    wave: Option<String>,
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
    error: Option<String>,
    preview: Option<Preview>,
}

#[derive(Clone, Debug)]
enum Message {
    LoginOpen,
    LoginFinished,
    Logout,
    Account(Result<(String, Vec<String>), String>),
    Open(Section),
    Query(String),
    Search,
    Found(u64, Result<Vec<Track>, String>),
    LikesMore,
    LikesPage(Result<Vec<Track>, String>),
    Like(Track),
    LikeSaved(Track, bool, Result<(), String>),
    WaveStart,
    WaveStarted(Result<Wave, String>),
    WaveMore(Result<Vec<Track>, String>),
    Cover(String, Vec<u8>),
    PlaylistsLoaded(Result<Vec<Playlist>, String>),
    PlaylistOpen(usize),
    PlaylistClose,
    PlaylistLoaded(String, Result<Vec<Track>, String>),
    PlaylistMore,
    Dislike,
    Saved(Result<(), String>),
    WindowReady(Option<isize>),
    Media(media::Event),
    Play(Section, usize),
    Started(u64, Result<(), String>),
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
        let token = api::load_token();
        let task = match &token {
            Some(token) => load_account(token.clone()),
            None => Task::done(Message::LoginOpen),
        };
        let app = Self {
            screen: token.map_or(Screen::Login(false), Screen::Main),
            section: Section::Wave,
            query: String::new(),
            results: Vec::new(),
            searching: false,
            search_id: 0,
            uid: None,
            liked_ids: Vec::new(),
            liked_set: HashSet::new(),
            liked: Vec::new(),
            likes_loading: false,
            playlists: Vec::new(),
            playlists_loading: false,
            playlist: None,
            playlist_tracks: Vec::new(),
            playlist_shown: 0,
            covers: HashMap::new(),
            media: None,
            failures: 0,
            player,
            queue: Vec::new(),
            wave: None,
            wave_queue: false,
            wave_loading: false,
            wave_fetching: false,
            play_id: 0,
            current: None,
            active: false,
            starting: false,
            position: Duration::ZERO,
            seeking: None,
            volume: 0.8,
            error,
            preview,
        };
        if let Some(p) = &app.player {
            p.set_volume(app.volume);
        }
        (app, Task::batch([task, window_handle()]))
    }

    fn token(&self) -> Option<String> {
        match &self.screen {
            Screen::Main(token) => Some(token.clone()),
            Screen::Login(_) => None,
        }
    }

    fn fail(&mut self, error: String) -> Task<Message> {
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
        self.results.clear();
        self.queue.clear();
        self.liked.clear();
        self.liked_ids.clear();
        self.liked_set.clear();
        self.covers.clear();
        self.playlists.clear();
        self.playlist = None;
        self.playlist_tracks.clear();
        self.uid = None;
        self.wave = None;
        self.wave_queue = false;
        self.current = None;
        self.active = false;
        self.starting = false;
    }

    /// Загружает обложки в несколько потоков, а не по потоку на трек: лайков бывают сотни.
    fn fetch_covers(&self, tracks: &[Track]) -> Task<Message> {
        self.fetch_cover_jobs(tracks.iter().filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect())
    }

    fn fetch_cover_jobs(&self, jobs: Vec<(String, String)>) -> Task<Message> {
        let jobs: Vec<(String, String)> = jobs.into_iter().filter(|(id, _)| !self.covers.contains_key(id)).collect();
        if jobs.is_empty() {
            return Task::none();
        }
        let (tx, rx) = mpsc::unbounded();
        let jobs = Arc::new(Mutex::new(jobs));
        for _ in 0..COVER_WORKERS {
            let (tx, jobs) = (tx.clone(), jobs.clone());
            std::thread::spawn(move || loop {
                let Some((id, uri)) = jobs.lock().unwrap().pop() else { break };
                if let Some(bytes) = api::cover(&uri) {
                    if tx.unbounded_send((id, bytes)).is_err() {
                        break;
                    }
                }
            });
        }
        Task::stream(rx).map(|(id, bytes)| Message::Cover(id, bytes))
    }

    /// Обложки остаются только у треков, которые видны или в очереди.
    fn prune_covers(&mut self) {
        let keep: HashSet<&str> = self.queue.iter().chain(&self.liked).chain(&self.results).chain(&self.playlist_tracks).map(|t| t.id.as_str()).collect();
        self.covers.retain(|id, _| id.starts_with(PLAYLIST_COVER) || keep.contains(id.as_str()));
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
        let start = Task::perform(
            background(move || {
                let info = api::file_info(&id, &token)?;
                player.play(generation, &info)
            }),
            move |result| Message::Started(generation, result),
        );
        Task::batch([start, self.wave_refill()])
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

    fn likes_more(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let from = self.liked.len();
        if self.likes_loading || from >= self.liked_ids.len() {
            return Task::none();
        }
        self.likes_loading = true;
        let page = self.liked_ids[from..(from + LIKES_PAGE).min(self.liked_ids.len())].to_vec();
        Task::perform(background(move || api::tracks_by_ids(&page, &token)), Message::LikesPage)
    }

    fn apply_like(&mut self, track: &Track, liked: bool) {
        if liked {
            self.liked_set.insert(track.id.clone());
            self.liked_ids.insert(0, track.id.clone());
            // Пока список не загружали, трек придёт вместе с первой страницей.
            if !self.liked.is_empty() {
                self.liked.insert(0, track.clone());
            }
        } else {
            self.liked_set.remove(&track.id);
            self.liked_ids.retain(|id| *id != track.id);
            self.liked.retain(|t| t.id != track.id);
        }
    }

    fn list(&self, section: Section) -> &[Track] {
        match section {
            Section::Wave => &self.queue,
            Section::Likes => &self.liked,
            Section::Playlists => &self.playlist_tracks,
            Section::Search => &self.results,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        let track = self.current_track().cloned();
        let playing = self.active && !self.player.as_ref().is_none_or(Player::is_paused);
        let position = self.player.as_ref().filter(|_| self.active).map_or(Duration::ZERO, Player::position);
        if let Some(media) = &mut self.media {
            media.sync(track.as_ref(), playing, position);
        }
        task
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
            Message::Account(Ok((uid, ids))) => {
                self.uid = Some(uid);
                self.liked_set = ids.iter().cloned().collect();
                self.liked_ids = ids;
                return match self.preview.clone() {
                    Some(Preview::Wave) => self.update(Message::WaveStart),
                    Some(Preview::Likes) => self.update(Message::Open(Section::Likes)),
                    Some(Preview::Playlists | Preview::Playlist) => self.update(Message::Open(Section::Playlists)),
                    Some(Preview::Search(query)) => {
                        self.section = Section::Search;
                        self.query = query;
                        self.update(Message::Search)
                    }
                    None if self.section == Section::Likes => self.likes_more(),
                    None => Task::none(),
                };
            }
            Message::Account(Err(e)) => return self.fail(e),
            Message::Open(section) => {
                self.section = section;
                self.error = None;
                return match section {
                    Section::Likes if self.liked.is_empty() => self.likes_more(),
                    Section::Playlists if self.playlists.is_empty() => self.playlists_load(),
                    Section::Search => operation::focus(Id::new(SEARCH_INPUT)),
                    _ => Task::none(),
                };
            }
            Message::Query(query) => self.query = query,
            Message::Search => {
                let Some(token) = self.token() else { return Task::none() };
                let text = self.query.trim().to_string();
                if text.is_empty() {
                    return Task::none();
                }
                self.searching = true;
                self.error = None;
                self.search_id += 1;
                let id = self.search_id;
                return Task::perform(background(move || api::search(&text, &token)), move |r| Message::Found(id, r));
            }
            Message::Found(id, result) => {
                if id != self.search_id {
                    return Task::none();
                }
                self.searching = false;
                match result {
                    Ok(tracks) => {
                        self.results = tracks;
                        self.prune_covers();
                        if matches!(self.preview.take(), Some(Preview::Search(_))) {
                            self.queue = self.results.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        return self.fetch_covers(&self.results);
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::LikesMore => return self.likes_more(),
            Message::LikesPage(result) => {
                self.likes_loading = false;
                match result {
                    Ok(page) => {
                        let task = self.fetch_covers(&page);
                        self.liked.extend(page);
                        if matches!(self.preview.take(), Some(Preview::Likes)) {
                            self.queue = self.liked.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        return task;
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::Like(track) => {
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
            Message::WaveStart => {
                let Some(token) = self.token() else { return Task::none() };
                self.section = Section::Wave;
                self.wave_loading = true;
                self.error = None;
                return Task::perform(background(move || api::wave_new(&token)), Message::WaveStarted);
            }
            Message::WaveStarted(result) => {
                self.wave_loading = false;
                match result {
                    Ok(wave) => {
                        self.wave = Some(wave.session);
                        self.queue = wave.tracks;
                        self.wave_queue = true;
                        self.wave_fetching = false;
                        self.prune_covers();
                        let covers = self.fetch_covers(&self.queue);
                        if matches!(self.preview.take(), Some(Preview::Wave)) {
                            // Просмотр без звука: волна не должна считать трек услышанным.
                            self.wave = None;
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                            return covers;
                        }
                        self.feedback("radioStarted", None, None);
                        return Task::batch([covers, self.play(0)]);
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::WaveMore(result) => {
                self.wave_fetching = false;
                match result {
                    Ok(tracks) if self.wave.is_some() => {
                        let known: HashSet<String> = self.queue.iter().map(|t| t.id.clone()).collect();
                        let fresh: Vec<Track> = tracks.into_iter().filter(|t| !known.contains(&t.id)).collect();
                        let task = self.fetch_covers(&fresh);
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
                        let jobs = self.playlists.iter().filter_map(|p| Some((format!("{PLAYLIST_COVER}{}", p.kind), p.cover.clone()?))).collect();
                        let covers = self.fetch_cover_jobs(jobs);
                        if matches!(self.preview, Some(Preview::Playlists)) {
                            self.preview = None;
                        }
                        if matches!(self.preview, Some(Preview::Playlist)) {
                            return Task::batch([covers, self.update(Message::PlaylistOpen(0))]);
                        }
                        return covers;
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::PlaylistOpen(index) => {
                let (Some(playlist), Some(token)) = (self.playlists.get(index).cloned(), self.token()) else { return Task::none() };
                self.playlist = Some(playlist.clone());
                self.playlist_tracks.clear();
                self.playlist_shown = 0;
                let kind = playlist.kind.clone();
                return Task::perform(background(move || api::playlist_tracks(&playlist, &token)), move |r| Message::PlaylistLoaded(kind.clone(), r));
            }
            Message::PlaylistClose => {
                self.playlist = None;
                self.playlist_tracks.clear();
                self.prune_covers();
            }
            Message::PlaylistLoaded(kind, result) => {
                // Пока грузился, могли открыть другой.
                if self.playlist.as_ref().is_none_or(|p| p.kind != kind) {
                    return Task::none();
                }
                match result {
                    Ok(tracks) => {
                        self.playlist_tracks = tracks;
                        self.playlist_shown = PLAYLIST_PAGE.min(self.playlist_tracks.len());
                        if matches!(self.preview.take(), Some(Preview::Playlist)) {
                            self.queue = self.playlist_tracks.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
                        return self.fetch_covers(&self.playlist_tracks[..self.playlist_shown]);
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::PlaylistMore => {
                let from = self.playlist_shown;
                self.playlist_shown = (from + PLAYLIST_PAGE).min(self.playlist_tracks.len());
                return self.fetch_covers(&self.playlist_tracks[from..self.playlist_shown]);
            }
            Message::Dislike => {
                let (Some(uid), Some(token), Some(track)) = (self.uid.clone(), self.token(), self.current_track().cloned()) else { return Task::none() };
                self.feedback("dislike", Some(&track), None);
                let id = track.id.clone();
                let save = Task::perform(background(move || api::set_disliked(&uid, &id, true, &token)), Message::Saved);
                let next = self.current.filter(|i| i + 1 < self.queue.len()).map_or(Task::none(), |i| self.play(i + 1));
                return Task::batch([save, next]);
            }
            Message::Saved(Ok(())) => {}
            Message::Saved(Err(e)) => return self.fail(e),
            Message::WindowReady(hwnd) => self.media = hwnd.map(Media::new),
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
                    self.wave_queue = false;
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
                    Ok(()) => {
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
                if let Some(i) = self.current.filter(|i| i + 1 < self.queue.len()) {
                    self.skip_feedback();
                    return self.play(i + 1);
                }
            }
            Message::Previous => {
                if let Some(i) = self.current.filter(|i| *i > 0) {
                    self.skip_feedback();
                    return self.play(i - 1);
                }
            }
            Message::Tick => {
                if let (true, Some(p)) = (self.active, &self.player) {
                    self.position = p.position();
                    if p.is_empty() {
                        self.active = false;
                        if let Some(track) = self.current_track().cloned() {
                            self.feedback("trackFinished", Some(&track), Some(track.duration.as_secs_f32()));
                        }
                        if let Some(i) = self.current.filter(|i| i + 1 < self.queue.len()) {
                            return self.play(i + 1);
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
                if let Some(p) = &self.player {
                    p.set_volume(volume);
                }
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
        let mut content = column![text("Tune").size(32).font(ui::FONT_BOLD)].spacing(14).align_x(Alignment::Center);
        if opened {
            content = content.push(text("Войдите в аккаунт Яндекса в открывшемся окне").color(ui::MUTED));
        } else {
            content = content
                .push(text("Для музыки нужен аккаунт Яндекса").color(ui::MUTED))
                .push(button(text("Войти через Яндекс").font(ui::FONT_BOLD)).padding([10, 20]).style(ui::accent).on_press(Message::LoginOpen));
        }
        if let Some(error) = &self.error {
            content = content.push(text(error).color(ui::DANGER));
        }
        container(content).center(Length::Fill).style(ui::page).into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let page = match self.section {
            Section::Wave => self.view_wave(),
            Section::Likes => self.view_likes(),
            Section::Playlists => self.view_playlists(),
            Section::Search => self.view_search(),
        };
        let mut page = column![page].spacing(12).height(Length::Fill);
        if let Some(error) = &self.error {
            page = page.push(text(error).color(ui::DANGER));
        }
        let body = row![self.view_sidebar(), container(page).padding([20, 24]).width(Length::Fill).height(Length::Fill).style(ui::page)];
        let mut layout = column![container(body).height(Length::Fill)];
        if let Some(bar) = self.view_bar() {
            layout = layout.push(container(Space::new()).width(Length::Fill).height(1).style(ui::line)).push(bar);
        }
        layout.into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let item = |icon, label, section| {
            let open = self.section == section;
            let style = if open { ui::selected } else { ui::ghost };
            let tint = if open { ui::TEXT } else { ui::MUTED };
            button(row![ui::icon(icon, 20.0, tint), text(label).font(ui::FONT_BOLD).color(tint)].spacing(12).align_y(Alignment::Center))
                .width(Length::Fill)
                .padding([9, 12])
                .style(style)
                .on_press(Message::Open(section))
        };
        let logout = button(row![ui::icon(ui::LOGOUT, 20.0, ui::MUTED), text("Выйти").color(ui::MUTED)].spacing(12).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([9, 12])
            .style(ui::ghost)
            .on_press(Message::Logout);
        container(
            column![
                container(text("Tune").size(22).font(ui::FONT_BOLD)).padding([6, 12]),
                column![
                    item(ui::WAVE, "Моя волна", Section::Wave),
                    item(ui::HEART, "Мне нравится", Section::Likes),
                    item(ui::PLAYLIST, "Плейлисты", Section::Playlists),
                    item(ui::SEARCH, "Поиск", Section::Search),
                ]
                .spacing(4),
                Space::new().height(Length::Fill),
                logout,
            ]
            .spacing(10),
        )
        .width(SIDEBAR)
        .height(Length::Fill)
        .padding(12)
        .style(ui::panel)
        .into()
    }

    fn view_wave(&self) -> Element<'_, Message> {
        let title = text("Моя волна").size(26).font(ui::FONT_BOLD);
        let center: Element<'_, Message> = if self.wave_loading {
            placeholder("Настраиваем волну…", None)
        } else if !self.wave_queue && self.queue.is_empty() {
            container(
                column![
                    text("Музыка под ваш вкус").size(22).font(ui::FONT_BOLD),
                    text("Треки подбирает Яндекс, волна не заканчивается").color(ui::MUTED),
                    button(text("Запустить волну").font(ui::FONT_BOLD)).padding([10, 20]).style(ui::accent).on_press(Message::WaveStart),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        } else if !self.wave_queue {
            container(
                column![
                    text("Сейчас играет другой список").color(ui::MUTED),
                    button(text("Запустить волну").font(ui::FONT_BOLD)).padding([10, 20]).style(ui::accent).on_press(Message::WaveStart),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        } else {
            self.view_list(Section::Wave, None)
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_likes(&self) -> Element<'_, Message> {
        let count = match self.liked_ids.len() {
            0 => String::new(),
            n => format!("{n} {}", tracks_word(n)),
        };
        let title = row![text("Мне нравится").size(26).font(ui::FONT_BOLD), text(count).color(ui::MUTED)].spacing(12).align_y(Alignment::Center);
        let center: Element<'_, Message> = if !self.liked.is_empty() {
            let more = (self.liked.len() < self.liked_ids.len()).then(|| {
                let label = if self.likes_loading { "Загружаем…" } else { "Показать ещё" };
                let press = (!self.likes_loading).then_some(Message::LikesMore);
                container(button(text(label)).padding([8, 16]).style(ui::ghost).on_press_maybe(press)).center_x(Length::Fill).into()
            });
            self.view_list(Section::Likes, more)
        } else if self.likes_loading || self.uid.is_none() {
            placeholder("Загружаем…", None)
        } else {
            placeholder("Пока пусто", Some("Нажмите на сердце у трека, и он появится здесь"))
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_playlists(&self) -> Element<'_, Message> {
        if let Some(playlist) = &self.playlist {
            let back = button(ui::icon(ui::BACK, 22.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(Message::PlaylistClose);
            let title = row![back, text(&playlist.title).size(26).font(ui::FONT_BOLD).wrapping(Wrapping::None), text(format!("{} {}", playlist.count, tracks_word(playlist.count as usize))).color(ui::MUTED)]
                .spacing(12)
                .align_y(Alignment::Center);
            let center: Element<'_, Message> = if !self.playlist_tracks.is_empty() {
                let more = (self.playlist_shown < self.playlist_tracks.len())
                    .then(|| container(button(text("Показать ещё")).padding([8, 16]).style(ui::ghost).on_press(Message::PlaylistMore)).center_x(Length::Fill).into());
                self.view_list(Section::Playlists, more)
            } else if playlist.count == 0 {
                placeholder("В этом плейлисте пока нет треков", None)
            } else {
                placeholder("Загружаем…", None)
            };
            return column![title, center].spacing(16).height(Length::Fill).into();
        }
        let title = text("Плейлисты").size(26).font(ui::FONT_BOLD);
        let center: Element<'_, Message> = if self.playlists.is_empty() {
            if self.playlists_loading || self.uid.is_none() {
                placeholder("Загружаем…", None)
            } else {
                placeholder("Плейлистов пока нет", Some("Созданные вами плейлисты появятся здесь"))
            }
        } else {
            let list = self.playlists.iter().enumerate().fold(column![].spacing(2).padding(Padding::ZERO.right(SCROLLBAR)), |list, (index, playlist)| {
                let cover: Element<'_, Message> = match self.covers.get(&format!("{PLAYLIST_COVER}{}", playlist.kind)) {
                    Some(handle) => image(handle.clone()).width(BAR_COVER).height(BAR_COVER).border_radius(4).into(),
                    None => container(Space::new()).width(BAR_COVER).height(BAR_COVER).style(ui::line).into(),
                };
                let line = row![
                    cover,
                    column![
                        text(&playlist.title).wrapping(Wrapping::None),
                        text(format!("{} {}", playlist.count, tracks_word(playlist.count as usize))).size(13).color(ui::MUTED),
                    ]
                    .width(Length::Fill)
                    .clip(true),
                ]
                .spacing(12)
                .align_y(Alignment::Center);
                list.push(button(line).width(Length::Fill).padding([6, 8]).style(ui::ghost).on_press(Message::PlaylistOpen(index)))
            });
            scrollable(list).height(Length::Fill).into()
        };
        column![title, center].spacing(16).height(Length::Fill).into()
    }

    fn view_search(&self) -> Element<'_, Message> {
        let input = text_input("Трек или исполнитель", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .padding([10, 14])
            .size(15)
            .style(ui::input);
        let center: Element<'_, Message> = if self.searching {
            placeholder("Ищем…", None)
        } else if self.results.is_empty() {
            placeholder("Что послушаем?", Some("Введите название трека или исполнителя и нажмите Enter"))
        } else {
            self.view_list(Section::Search, None)
        };
        column![text("Поиск").size(26).font(ui::FONT_BOLD), input, center].spacing(16).height(Length::Fill).into()
    }

    fn view_list<'a>(&'a self, section: Section, footer: Option<Element<'a, Message>>) -> Element<'a, Message> {
        let shown = if section == Section::Playlists { self.playlist_shown } else { usize::MAX };
        let mut list = self.list(section).iter().take(shown).enumerate().fold(column![].spacing(2), |list, (index, track)| list.push(self.view_track(section, index, track))).padding(Padding::ZERO.right(SCROLLBAR));
        if let Some(footer) = footer {
            list = list.push(footer);
        }
        column![view_header(), scrollable(list).height(Length::Fill)].spacing(8).into()
    }

    fn view_track<'a>(&'a self, section: Section, index: usize, track: &'a Track) -> Element<'a, Message> {
        let playing = self.current_track().is_some_and(|t| t.id == track.id);
        let number: Element<'_, Message> = if playing {
            ui::icon(ui::PLAY, 16.0, ui::ACCENT)
        } else {
            text(index + 1).size(13).color(ui::MUTED).into()
        };
        let line = row![
            container(number).width(NUMBER).center_x(NUMBER),
            self.cover(track, COVER),
            column![
                text(&track.title).color(if playing { ui::ACCENT } else { ui::TEXT }).wrapping(Wrapping::None),
                text(&track.artists).size(13).color(ui::MUTED).wrapping(Wrapping::None),
            ]
            .width(Length::Fill)
            .clip(true),
            self.like_button(track, 16.0),
            text(clock(track.duration)).size(13).color(ui::MUTED),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        button(line).width(Length::Fill).padding([6, 8]).style(ui::ghost).on_press(Message::Play(section, index)).into()
    }

    fn like_button<'a>(&self, track: &Track, size: f32) -> Element<'a, Message> {
        let liked = self.liked_set.contains(&track.id);
        let icon = if liked { ui::icon(ui::HEART, size, ui::ACCENT) } else { ui::icon(ui::HEART_OUTLINE, size, ui::MUTED) };
        button(container(icon).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::Like(track.clone())).into()
    }

    fn cover<'a>(&'a self, track: &Track, size: f32) -> Element<'a, Message> {
        match self.covers.get(&track.id) {
            Some(handle) => image(handle.clone()).width(size).height(size).border_radius(4).into(),
            None => container(Space::new()).width(size).height(size).style(ui::line).into(),
        }
    }

    fn view_bar(&self) -> Option<Element<'_, Message>> {
        let track = self.current_track()?;
        let total = track.duration.as_secs_f32().max(1.0);
        let shown = self.seeking.unwrap_or(self.position.as_secs_f32()).min(total);
        let playing = self.starting || (self.active && !self.player.as_ref().is_none_or(Player::is_paused));
        let status = if self.starting { "Загрузка…" } else { track.artists.as_str() };

        let side = |icon, message| button(ui::icon(icon, 22.0, ui::TEXT)).padding(6).style(ui::ghost).on_press(message);
        let toggle = button(ui::icon(if playing { ui::PAUSE } else { ui::PLAY }, 22.0, ui::ON_ACCENT)).padding(9).style(ui::round).on_press(Message::Toggle);

        let about = row![
            self.cover(track, BAR_COVER),
            column![
                text(&track.title).font(ui::FONT_BOLD).wrapping(Wrapping::None),
                text(status).size(13).color(ui::MUTED).wrapping(Wrapping::None),
            ]
            .clip(true),
            self.like_button(track, 20.0),
            button(container(ui::icon(ui::DISLIKE, 18.0, ui::MUTED)).center(LIKE)).padding(0).style(ui::ghost).on_press(Message::Dislike),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .width(Length::FillPortion(3));

        let transport = column![
            row![side(ui::PREVIOUS, Message::Previous), toggle, side(ui::NEXT, Message::Next)].spacing(10).align_y(Alignment::Center),
            row![
                text(clock(Duration::from_secs_f32(shown))).size(12).color(ui::MUTED),
                slider(0.0..=total, shown, Message::Seek).on_release(Message::SeekDone).style(ui::seek),
                text(clock(track.duration)).size(12).color(ui::MUTED),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .align_x(Alignment::Center)
        .width(Length::FillPortion(4));

        let volume = container(
            row![ui::icon(ui::VOLUME, 20.0, ui::MUTED), slider(0.0..=1.0, self.volume, Message::Volume).step(0.01).width(110).style(ui::volume)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .align_right(Length::FillPortion(3));

        Some(
            container(row![about, transport, volume].spacing(24).align_y(Alignment::Center))
                .padding([10, 16])
                .width(Length::Fill)
                .style(ui::panel)
                .into(),
        )
    }
}

fn view_header<'a>() -> Element<'a, Message> {
    let label = |s| text(s).size(12).color(ui::MUTED);
    column![
        row![
            container(label("#")).width(NUMBER).center_x(NUMBER),
            container(label("Название")).width(Length::Fill),
            label("Время"),
            Space::new().width(LIST_GUTTER),
        ]
        .spacing(12)
        .padding([0, 8]),
        container(Space::new()).width(Length::Fill).height(1).style(ui::line),
    ]
    .spacing(8)
    .into()
}

fn placeholder<'a>(title: &'a str, hint: Option<&'a str>) -> Element<'a, Message> {
    let mut content = column![].spacing(6).align_x(Alignment::Center);
    match hint {
        Some(hint) => content = content.push(text(title).size(22).font(ui::FONT_BOLD)).push(text(hint).color(ui::MUTED)),
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

trait PipeMedia {
    fn pipe_media(self) -> Self;
}

impl PipeMedia for Subscription<Message> {
    fn pipe_media(self) -> Self {
        Subscription::batch([self, Subscription::run(media::events).map(Message::Media)])
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
            let uid = api::account_uid(&token)?;
            let ids = api::liked_ids(&uid, &token)?;
            Ok((uid, ids))
        }),
        Message::Account,
    )
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
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
    let info = api::file_info(&track.id, &token)?;
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

/// Проверка без окна: лайки, волна и её обратная связь. Лайк ставится и сразу снимается,
/// список «Мне нравится» остаётся прежним; из обратной связи волне уходит только запуск.
fn smoke_library(query: &str) -> Result<(), String> {
    let token = api::load_token().ok_or("Сначала войдите в окне приложения")?;
    let uid = api::account_uid(&token)?;
    let ids = api::liked_ids(&uid, &token)?;
    println!("Лайков: {}", ids.len());
    let page = api::tracks_by_ids(&ids[..ids.len().min(LIKES_PAGE)], &token)?;
    println!("Загружено сведений: {}, порядок совпал: {}", page.len(), page.iter().zip(&ids).all(|(t, id)| t.id == *id));
    let wave = api::wave_new(&token)?;
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
    let preview = match args.as_slice() {
        [_, flag, query] if flag == "--smoke" || flag == "--smoke-library" => {
            let result = if flag == "--smoke" { smoke(query) } else { smoke_library(query) };
            if let Err(e) = result {
                println!("Ошибка: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        [_, flag, what] if flag == "--preview" => Some(match what.as_str() {
            "wave" => Preview::Wave,
            "likes" => Preview::Likes,
            "playlists" => Preview::Playlists,
            "playlist" => Preview::Playlist,
            query => Preview::Search(query.to_string()),
        }),
        _ => None,
    };
    iced::application(move || App::new(preview.clone()), App::update, App::view)
        .title("Tune")
        .theme(ui::theme())
        .default_font(ui::FONT)
        .subscription(App::subscription)
        .window(window::Settings { size: Size::new(1100.0, 720.0), min_size: Some(Size::new(760.0, 480.0)), ..window::Settings::default() })
        .run()
}
