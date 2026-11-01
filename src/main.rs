#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod player;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::widget::{button, column, container, image, row, scrollable, slider, text, text_input, Space};
use iced::{time, Alignment, Element, Length, Subscription, Task, Theme};

use api::{DeviceCode, Track, NEED_LOGIN};
use player::Player;

const LOGIN_POLL: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(500);
const COVER: f32 = 44.0;

/// Блокирующая работа в отдельном потоке, чтобы не останавливать интерфейс.
fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl Future<Output = T> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("фоновая задача завершилась") }
}

enum Screen {
    Login(Option<DeviceCode>),
    Main(String),
}

struct App {
    screen: Screen,
    query: String,
    tracks: Vec<Track>,
    /// Что играет: новый поиск не прерывает музыку.
    queue: Vec<Track>,
    play_id: u64,
    covers: HashMap<String, image::Handle>,
    searching: bool,
    search_id: u64,
    player: Option<Player>,
    current: Option<usize>,
    /// Трек запущен и ещё не доиграл.
    active: bool,
    starting: bool,
    position: Duration,
    /// Куда тянут ползунок: пока его держат, ход песни его не двигает.
    seeking: Option<f32>,
    volume: f32,
    error: Option<String>,
}

#[derive(Clone, Debug)]
enum Message {
    LoginStarted(Result<DeviceCode, String>),
    OpenLogin,
    LoginPoll,
    LoginChecked(Result<Option<String>, String>),
    LoginAgain,
    Logout,
    Query(String),
    Search,
    Found(u64, Result<Vec<Track>, String>),
    Cover(String, Option<Vec<u8>>),
    Play(usize),
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
    fn new() -> (Self, Task<Message>) {
        let (player, error) = match Player::new() {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        let token = api::load_token();
        let task = if token.is_none() { start_login() } else { Task::none() };
        let app = Self {
            screen: token.map_or(Screen::Login(None), Screen::Main),
            query: String::new(),
            tracks: Vec::new(),
            queue: Vec::new(),
            play_id: 0,
            covers: HashMap::new(),
            searching: false,
            search_id: 0,
            player,
            current: None,
            active: false,
            starting: false,
            position: Duration::ZERO,
            seeking: None,
            volume: 0.8,
            error,
        };
        if let Some(p) = &app.player {
            p.set_volume(app.volume);
        }
        (app, task)
    }

    fn token(&self) -> Option<String> {
        match &self.screen {
            Screen::Main(token) => Some(token.clone()),
            Screen::Login(_) => None,
        }
    }

    fn fail(&mut self, error: String) -> Task<Message> {
        if error == NEED_LOGIN {
            api::logout();
            self.screen = Screen::Login(None);
            self.error = Some("Вход устарел, войдите ещё раз".into());
            return start_login();
        }
        self.error = Some(error);
        Task::none()
    }

    fn play(&mut self, index: usize) -> Task<Message> {
        let (Some(player), Some(token), Some(track)) = (self.player.clone(), self.token(), self.queue.get(index)) else {
            return Task::none();
        };
        let id = track.id.clone();
        let generation = player.stop();
        self.play_id = generation;
        self.current = Some(index);
        self.active = false;
        self.starting = true;
        self.position = Duration::ZERO;
        self.seeking = None;
        self.error = None;
        Task::perform(
            background(move || {
                let info = api::file_info(&id, &token)?;
                player.play(generation, &info)
            }),
            move |result| Message::Started(generation, result),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::LoginStarted(Ok(code)) => {
                let clipboard = iced::clipboard::write(code.user_code.clone());
                self.screen = Screen::Login(Some(code));
                return clipboard;
            }
            Message::LoginStarted(Err(e)) => self.error = Some(e),
            Message::OpenLogin => {
                if let Screen::Login(Some(code)) = &self.screen {
                    if code.url.starts_with("https://") {
                        let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", &code.url]).spawn();
                    }
                }
            }
            Message::LoginPoll => {
                if let Screen::Login(Some(code)) = &self.screen {
                    let device_code = code.device_code.clone();
                    return Task::perform(background(move || api::login_poll(&device_code)), Message::LoginChecked);
                }
            }
            Message::LoginChecked(Ok(Some(token))) => {
                self.error = None;
                self.screen = Screen::Main(token);
            }
            Message::LoginChecked(Ok(None)) => {}
            Message::LoginChecked(Err(e)) => {
                self.screen = Screen::Login(None);
                self.error = Some(e);
            }
            Message::LoginAgain => {
                self.error = None;
                return start_login();
            }
            Message::Logout => {
                if let Some(p) = &self.player {
                    p.stop();
                }
                api::logout();
                self.current = None;
                self.active = false;
                self.tracks.clear();
                self.queue.clear();
                self.covers.clear();
                self.screen = Screen::Login(None);
                return start_login();
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
                        self.tracks = tracks;
                        self.covers.clear();
                        return Task::batch(self.tracks.iter().filter_map(|t| {
                            let id = t.id.clone();
                            let uri = t.cover.clone()?;
                            Some(Task::perform(background(move || api::cover(&uri)), move |bytes| Message::Cover(id.clone(), bytes)))
                        }));
                    }
                    Err(e) => return self.fail(e),
                }
            }
            Message::Cover(id, Some(bytes)) => {
                self.covers.insert(id, image::Handle::from_bytes(bytes));
            }
            Message::Cover(_, None) => {}
            Message::Play(index) => {
                self.queue = self.tracks.clone();
                return self.play(index);
            }
            Message::Started(generation, result) => {
                // Пока трек запускался, могли включить другой.
                if generation != self.play_id {
                    return Task::none();
                }
                self.starting = false;
                match result {
                    Ok(()) => self.active = true,
                    Err(e) => return self.fail(e),
                }
            }
            Message::Toggle => {
                if let Some(p) = &self.player {
                    p.toggle();
                }
            }
            Message::Next => {
                if let Some(i) = self.current.filter(|i| i + 1 < self.queue.len()) {
                    return self.play(i + 1);
                }
            }
            Message::Previous => {
                if let Some(i) = self.current.filter(|i| *i > 0) {
                    return self.play(i - 1);
                }
            }
            Message::Tick => {
                if let (true, Some(p)) = (self.active, &self.player) {
                    self.position = p.position();
                    if p.is_empty() {
                        self.active = false;
                        return self.update(Message::Next);
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

    fn subscription(&self) -> Subscription<Message> {
        match &self.screen {
            Screen::Login(Some(_)) => time::every(LOGIN_POLL).map(|_| Message::LoginPoll),
            Screen::Main(_) if self.active => time::every(TICK).map(|_| Message::Tick),
            _ => Subscription::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Login(code) => self.view_login(code.as_ref()),
            Screen::Main(_) => self.view_main(),
        }
    }

    fn view_login<'a>(&'a self, code: Option<&'a DeviceCode>) -> Element<'a, Message> {
        let mut content = column![text("Tune Native").size(28)].spacing(14).align_x(Alignment::Center);
        match code {
            Some(code) => {
                content = content
                    .push(text("Введите этот код на странице Яндекса. Он уже скопирован."))
                    .push(text(&code.user_code).size(36))
                    .push(button("Открыть страницу входа").on_press(Message::OpenLogin))
                    .push(text("После подтверждения окно обновится само.").size(13));
            }
            None if self.error.is_some() => content = content.push(button("Попробовать ещё раз").on_press(Message::LoginAgain)),
            None => content = content.push(text("Получаем код входа…")),
        }
        if let Some(error) = &self.error {
            content = content.push(text(error).style(text::danger));
        }
        container(content).center(Length::Fill).into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let top = row![
            text_input("Поиск треков", &self.query).on_input(Message::Query).on_submit(Message::Search).padding(10),
            button("Найти").on_press(Message::Search).padding(10),
            button("Выйти").on_press(Message::Logout).padding(10).style(button::secondary),
        ]
        .spacing(8);

        let list = self.tracks.iter().enumerate().fold(column![].spacing(2), |list, (index, track)| {
            let cover: Element<'_, Message> = match self.covers.get(&track.id) {
                Some(handle) => image(handle.clone()).width(COVER).height(COVER).into(),
                None => Space::new().width(COVER).height(COVER).into(),
            };
            let line = row![
                cover,
                column![text(&track.title), text(&track.artists).size(13)].width(Length::Fill),
                text(clock(track.duration)).size(13),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            let playing = self.current.and_then(|i| self.queue.get(i)).is_some_and(|t| t.id == track.id);
            let style = if playing { button::primary } else { button::text };
            list.push(button(line).width(Length::Fill).padding(6).style(style).on_press(Message::Play(index)))
        });

        let center: Element<'_, Message> = if self.searching {
            container(text("Ищем…")).center(Length::Fill).into()
        } else if self.tracks.is_empty() {
            container(text("Найдите трек, чтобы начать").style(text::secondary)).center(Length::Fill).into()
        } else {
            scrollable(list).spacing(8).height(Length::Fill).into()
        };

        let mut page = column![top, center].spacing(12).padding(12);
        if let Some(error) = &self.error {
            page = page.push(text(error).style(text::danger));
        }
        if let Some(bar) = self.view_bar() {
            page = page.push(bar);
        }
        page.into()
    }

    fn view_bar(&self) -> Option<Element<'_, Message>> {
        let track = self.queue.get(self.current?)?;
        let total = track.duration.as_secs_f32().max(1.0);
        let shown = self.seeking.unwrap_or(self.position.as_secs_f32()).min(total);
        let paused = self.player.as_ref().is_none_or(Player::is_paused);
        let status = if self.starting { "Загрузка…".to_string() } else { track.artists.clone() };
        Some(
            column![
                row![
                    text(clock(Duration::from_secs_f32(shown))).size(13),
                    slider(0.0..=total, shown, Message::Seek).on_release(Message::SeekDone),
                    text(clock(track.duration)).size(13),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                row![
                    button("⏮").on_press(Message::Previous).style(button::secondary),
                    button(if paused { "▶" } else { "⏸" }).on_press(Message::Toggle),
                    button("⏭").on_press(Message::Next).style(button::secondary),
                    column![text(&track.title), text(status).size(13).style(text::secondary)].width(Length::Fill),
                    text("Громкость").size(13),
                    slider(0.0..=1.0, self.volume, Message::Volume).step(0.01).width(120),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            ]
            .spacing(6)
            .into(),
        )
    }
}

fn start_login() -> Task<Message> {
    Task::perform(background(api::login_start), Message::LoginStarted)
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
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

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, query] = args.as_slice() {
        if flag == "--smoke" {
            if let Err(e) = smoke(query) {
                println!("Ошибка: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
    }
    iced::application(App::new, App::update, App::view)
        .title("Tune Native")
        .theme(Theme::Dark)
        .subscription(App::subscription)
        .window_size((900.0, 640.0))
        .run()
}
