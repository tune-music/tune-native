#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod player;
mod ui;

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, image, operation, row, scrollable, slider, text, text_input, Id, Space};
use iced::{time, window, Alignment, Element, Length, Size, Subscription, Task};

use api::{DeviceCode, Track, NEED_LOGIN};
use player::Player;

const LOGIN_POLL: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(500);
const COVER: f32 = 40.0;
const BAR_COVER: f32 = 56.0;
const SIDEBAR: f32 = 200.0;
const NUMBER: f32 = 28.0;
/// Место под полосу прокрутки, чтобы колонка времени в заголовке совпала со строками.
const LIST_GUTTER: f32 = 5.0;
const SEARCH_INPUT: &str = "search";

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
    /// Запуск с готовым поиском: первый трек показывается в плеере, но не играет.
    preview: bool,
}

#[derive(Clone, Debug)]
enum Message {
    LoginStarted(Result<DeviceCode, String>),
    OpenLogin,
    LoginPoll,
    LoginChecked(Result<Option<String>, String>),
    LoginAgain,
    Logout,
    FocusSearch,
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
    fn new(preview: Option<String>) -> (Self, Task<Message>) {
        let (player, error) = match Player::new() {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        let token = api::load_token();
        let task = if token.is_none() { start_login() } else { Task::none() };
        let mut app = Self {
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
            preview: preview.is_some(),
        };
        if let Some(p) = &app.player {
            p.set_volume(app.volume);
        }
        if let Some(query) = preview {
            app.query = query;
            let search = app.update(Message::Search);
            return (app, search);
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
            Message::FocusSearch => return operation::focus(Id::new(SEARCH_INPUT)),
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
                        // Обложка играющего трека нужна нижней панели и после нового поиска.
                        let queue = &self.queue;
                        self.covers.retain(|id, _| queue.iter().any(|t| t.id == *id));
                        if std::mem::take(&mut self.preview) {
                            self.queue = self.tracks.clone();
                            self.current = Some(0).filter(|_| !self.queue.is_empty());
                        }
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
        let mut content = column![text("Tune").size(32).font(ui::FONT_BOLD)].spacing(14).align_x(Alignment::Center);
        match code {
            Some(code) => {
                content = content
                    .push(text("Введите код на странице Яндекса. Он уже скопирован.").color(ui::MUTED))
                    .push(text(&code.user_code).size(40).font(ui::FONT_BOLD))
                    .push(button(text("Открыть страницу входа").font(ui::FONT_BOLD)).padding([10, 20]).style(ui::accent).on_press(Message::OpenLogin))
                    .push(text("После подтверждения приложение откроется само").size(13).color(ui::MUTED));
            }
            None if self.error.is_some() => {
                content = content.push(button(text("Попробовать ещё раз").font(ui::FONT_BOLD)).padding([10, 20]).style(ui::accent).on_press(Message::LoginAgain));
            }
            None => content = content.push(text("Получаем код входа…").color(ui::MUTED)),
        }
        if let Some(error) = &self.error {
            content = content.push(text(error).color(ui::DANGER));
        }
        container(content).center(Length::Fill).style(ui::page).into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let body = row![self.view_sidebar(), self.view_search()];
        let mut layout = column![container(body).height(Length::Fill)];
        if let Some(bar) = self.view_bar() {
            layout = layout.push(container(Space::new()).width(Length::Fill).height(1).style(ui::line)).push(bar);
        }
        layout.into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let search = button(row![ui::icon(ui::SEARCH, 20.0, ui::TEXT), text("Поиск").font(ui::FONT_BOLD)].spacing(12).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([9, 12])
            .style(ui::selected)
            .on_press(Message::FocusSearch);
        let logout = button(row![ui::icon(ui::LOGOUT, 20.0, ui::MUTED), text("Выйти").color(ui::MUTED)].spacing(12).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([9, 12])
            .style(ui::ghost)
            .on_press(Message::Logout);
        container(
            column![
                container(text("Tune").size(22).font(ui::FONT_BOLD)).padding([6, 12]),
                search,
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

    fn view_search(&self) -> Element<'_, Message> {
        let input = text_input("Трек или исполнитель", &self.query)
            .id(SEARCH_INPUT)
            .on_input(Message::Query)
            .on_submit(Message::Search)
            .padding([10, 14])
            .size(15)
            .style(ui::input);

        let center: Element<'_, Message> = if self.searching {
            container(text("Ищем…").color(ui::MUTED)).center(Length::Fill).into()
        } else if self.tracks.is_empty() {
            container(
                column![
                    text("Что послушаем?").size(22).font(ui::FONT_BOLD),
                    text("Введите название трека или исполнителя и нажмите Enter").color(ui::MUTED),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        } else {
            let list = self.tracks.iter().enumerate().fold(column![].spacing(2), |list, (index, track)| list.push(self.view_track(index, track)));
            column![self.view_header(), scrollable(list).spacing(8).height(Length::Fill)].spacing(8).into()
        };

        let mut page = column![text("Поиск").size(26).font(ui::FONT_BOLD), input, center].spacing(16);
        if let Some(error) = &self.error {
            page = page.push(text(error).color(ui::DANGER));
        }
        container(page).padding([20, 24]).width(Length::Fill).height(Length::Fill).style(ui::page).into()
    }

    fn view_header(&self) -> Element<'_, Message> {
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

    fn view_track<'a>(&'a self, index: usize, track: &'a Track) -> Element<'a, Message> {
        let playing = self.current.and_then(|i| self.queue.get(i)).is_some_and(|t| t.id == track.id);
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
            text(clock(track.duration)).size(13).color(ui::MUTED),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        button(line).width(Length::Fill).padding([6, 8]).style(ui::ghost).on_press(Message::Play(index)).into()
    }

    fn cover<'a>(&'a self, track: &Track, size: f32) -> Element<'a, Message> {
        match self.covers.get(&track.id) {
            Some(handle) => image(handle.clone()).width(size).height(size).border_radius(4).into(),
            None => container(Space::new()).width(size).height(size).style(ui::line).into(),
        }
    }

    fn view_bar(&self) -> Option<Element<'_, Message>> {
        let track = self.queue.get(self.current?)?;
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
    let preview = match args.as_slice() {
        [_, flag, query] if flag == "--preview" => Some(query.clone()),
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
