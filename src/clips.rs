//! Клипы: страница «Клипы» (свои и «Вам понравится»), полка на странице исполнителя и
//! окно просмотра поверх Tune со своим управлением. Видео играет libmpv (`video.rs`).

use std::time::Instant;

use crate::*;
use api::Clip;
use iced::widget::{column, row};

/// Кнопки на видео прячутся, если мышь не двигалась столько секунд.
const CONTROLS_HIDE: f32 = 2.5;
/// Скорости, как у подкастов в оригинале.
pub const CLIP_SPEEDS: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// Ширина колонки «Далее» справа от видео.
const NEXT_WIDTH: f32 = 340.0;
/// Колонка «Далее» появляется, только если видео при ней остаётся не уже этого.
const NEXT_ROOM: f32 = 640.0;
const CLIP_COVER: &str = "clip:";
/// Ширина карточки клипа, к которой подгоняется ряд.
const CLIP_CARD: f32 = 290.0;
const CLIP_POSTER: &str = "clip-poster:";

/// Меню на видео: скорость или качество.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipMenu {
    Speed,
    Quality,
}

/// Открытый клип.
pub struct Watch {
    pub list: Vec<Clip>,
    pub at: usize,
    pub video: Option<video::Video>,
    pub state: video::State,
    pub error: Option<String>,
    pub credits: Vec<(String, String)>,
    pub speed: f32,
    /// Окно Tune развёрнуто на весь экран ради клипа.
    pub full: bool,
    pub moved: Instant,
    pub menu: Option<ClipMenu>,
    /// Куда тянут полоску перемотки, пока её не отпустили.
    pub drag: Option<f32>,
    /// Качества: высота кадра и поток, от лучшего к худшему.
    pub qualities: Vec<(u32, String)>,
    pub quality: usize,
    /// Музыка играла до клипа: после него продолжится.
    pub resume: bool,
    /// Номер открытия: ответы для прошлого клипа не подменят текущий.
    pub ticket: u64,
    /// Лента «Время клипов»: сессия и идёт ли догрузка.
    pub feed: Option<(String, bool)>,
}

impl Watch {
    pub fn clip(&self) -> &Clip {
        &self.list[self.at]
    }

    /// Нужны ли кадры: видео идёт, грузится, или только что что-то нажали (перемотка на паузе,
    /// исчезновение кнопок).
    pub fn moving(&self) -> bool {
        self.video.is_some() && (!self.state.paused || self.state.buffering || self.moved.elapsed().as_secs_f32() < 1.0)
    }

    fn controls_shown(&self) -> f32 {
        if self.state.paused || self.menu.is_some() || self.drag.is_some() || self.video.is_none() || self.state.ended {
            return 1.0;
        }
        let idle = self.moved.elapsed().as_secs_f32();
        (1.0 - (idle - CONTROLS_HIDE) / 0.3).clamp(0.0, 1.0)
    }
}

fn clip_clock(seconds: f32) -> String {
    clock(Duration::from_secs_f32(seconds.max(0.0)))
}

impl App {
    /// Клипы, которые понравились, и «Вам понравится».
    pub(crate) fn clips_load(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let uid = self.uid.clone();
        self.clips_loading = true;
        Task::perform(
            background(move || {
                let liked = match &uid {
                    Some(uid) => api::liked_clips(uid, &token).unwrap_or_default(),
                    None => Vec::new(),
                };
                let like = api::clips_will_like(&token)?;
                Ok((liked, like))
            }),
            Message::ClipsLoaded,
        )
    }

    pub(crate) fn clip_feed_load(&self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        Task::perform(background(move || api::clip_feed(&token)), Message::ClipFeed)
    }

    pub(crate) fn clip_covers(&self, clips: &[Clip]) -> Task<Message> {
        // Кадры с видеохостинга (`get-vh`) знают только точные размеры 16:9.
        let (vh, music): (Vec<(String, String)>, Vec<(String, String)>) = clips.iter().filter_map(|clip| Some((clip_cover_key(clip), clip.cover.clone()?))).partition(|(_, uri)| uri.contains("/get-vh/"));
        Task::batch([self.fetch_cover_jobs(music, "m400x400"), self.fetch_cover_jobs(vh, "400x225")])
    }

    pub(crate) fn clip_liked(&self, clip: &Clip) -> bool {
        self.clips_liked.iter().any(|liked| liked.id == clip.id)
    }

    /// Открывает клип из списка: музыка встаёт на паузу, видео грузится.
    pub(crate) fn clip_open(&mut self, list: Vec<Clip>, at: usize) -> Task<Message> {
        if list.is_empty() {
            return Task::none();
        }
        let at = at.min(list.len() - 1);
        let (mut resume, mut full) = (false, false);
        // Другой клип в открытом окне: музыка и полный экран остаются как были.
        if let Some(watch) = self.clip.take() {
            resume = watch.resume;
            full = watch.full;
        } else if let Some(p) = self.player.clone().filter(|_| self.is_playing()) {
            p.toggle();
            self.progress_save(false);
            resume = true;
        }
        self.player_full = false;
        self.clip_ticket += 1;
        self.clip = Some(Watch {
            list,
            at,
            video: None,
            state: video::State::default(),
            error: None,
            credits: Vec::new(),
            speed: 1.0,
            full,
            moved: Instant::now(),
            menu: None,
            drag: None,
            qualities: Vec::new(),
            quality: 0,
            resume,
            ticket: self.clip_ticket,
            feed: None,
        });
        self.clip_start()
    }

    /// Лента близко к концу: просим ещё.
    fn clip_feed_more(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let Some(watch) = self.clip.as_mut() else { return Task::none() };
        let Some((session, busy)) = watch.feed.as_mut() else { return Task::none() };
        if *busy || watch.at + 2 < watch.list.len() {
            return Task::none();
        }
        *busy = true;
        let session = session.clone();
        let seen: Vec<String> = watch.list.iter().map(|clip| clip.id.clone()).collect();
        let ticket = watch.ticket;
        Task::perform(background(move || api::clip_feed_next(&session, &seen, &token).map(|clips| (session, clips))), move |result| Message::ClipFeedLoaded(ticket, result))
    }

    /// Грузит поток, авторов и обложку текущего клипа.
    fn clip_start(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        let Some(watch) = self.clip.as_mut() else { return Task::none() };
        let ticket = watch.ticket;
        let clip = watch.clip().clone();
        watch.video = None;
        watch.state = video::State::default();
        watch.error = None;
        watch.credits.clear();
        watch.qualities.clear();
        watch.quality = 0;
        watch.drag = None;
        watch.menu = None;
        watch.moved = Instant::now();
        if !video::available() {
            watch.error = Some(format!("Нет видеоплеера: рядом с Tune нет {}", video::LIBRARY));
            return Task::none();
        }
        let stream = {
            let (clip, token) = (clip.clone(), token.clone());
            Task::perform(background(move || api::clip_stream(&clip, &token)), move |result| Message::ClipStream(ticket, result))
        };
        let credits = if clip.id.is_empty() {
            Task::none()
        } else {
            let (id, token) = (clip.id.clone(), token.clone());
            Task::perform(background(move || api::clip_credits(&id, &token)), move |result| Message::ClipCredits(ticket, result))
        };
        let poster = clip.cover.clone().map(|uri| self.fetch_cover_jobs(vec![(format!("{CLIP_POSTER}{}", clip_cover_id(&clip)), uri)], "orig")).unwrap_or_else(Task::none);
        Task::batch([stream, credits, poster])
    }

    pub(crate) fn clip_close(&mut self) -> Task<Message> {
        let Some(watch) = self.clip.take() else { return Task::none() };
        let unfull = if watch.full { window::latest().and_then(|id| window::set_mode(id, window::Mode::Windowed)) } else { Task::none() };
        drop(watch.video);
        if watch.resume {
            if let Some(p) = self.player.clone().filter(|p| p.is_paused()) {
                p.toggle();
            }
        }
        unfull
    }

    /// Громкость клипа идёт за громкостью Tune.
    pub(crate) fn clip_volume(&self) {
        if let Some(video) = self.clip.as_ref().and_then(|watch| watch.video.as_ref()) {
            video.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    /// Каждый кадр: положение, конец клипа и переход к следующему.
    pub(crate) fn clip_step(&mut self) -> Task<Message> {
        let Some(watch) = self.clip.as_mut() else { return Task::none() };
        let Some(video) = watch.video.clone() else { return Task::none() };
        watch.state = video.state();
        if watch.error.is_none() {
            watch.error = video.failed();
        }
        if watch.state.ended && watch.at + 1 < watch.list.len() && watch.error.is_none() {
            watch.at += 1;
            let start = self.clip_start();
            return Task::batch([start, self.clip_feed_more()]);
        }
        Task::none()
    }

    /// Сообщения клипа. Пока окно клипа открыто, сюда же приходят пробел, стрелки и Esc.
    pub(crate) fn clip_handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ClipsLoaded(result) => {
                self.clips_loading = false;
                match result {
                    Ok((liked, like)) => {
                        let covers = Task::batch([self.clip_covers(&liked), self.clip_covers(&like)]);
                        self.clips_liked = liked;
                        self.clips_like = like;
                        return covers;
                    }
                    Err(error) => return self.fail(error),
                }
            }
            Message::ClipOpen(list, at) => {
                let covers = self.clip_covers(&list);
                return Task::batch([self.clip_open(list, at), covers]);
            }
            Message::ClipLink(Ok(list), at) => {
                if list.is_empty() {
                    return self.notify("Клип не найден".to_string());
                }
                return self.handle(Message::ClipOpen(list, at));
            }
            Message::ClipLink(Err(error), _) => return self.fail(error),
            Message::ClipStream(ticket, result) => {
                let volume = if self.muted { 0.0 } else { self.volume };
                let Some(watch) = self.clip.as_mut().filter(|watch| watch.ticket == ticket) else { return Task::none() };
                // По умолчанию — лучшее до Full HD.
                let qualities = match result {
                    Ok(qualities) => qualities,
                    Err(error) => {
                        watch.error = Some(error);
                        return Task::none();
                    }
                };
                let pick = qualities.iter().position(|(height, _)| *height <= 1080).unwrap_or(0);
                match qualities.get(pick).ok_or_else(|| "Яндекс не прислал видео".to_string()).and_then(|(_, url)| video::Video::open(url, volume, 0.0)) {
                    Ok(video) => {
                        video.set_speed(watch.speed);
                        watch.video = Some(video);
                        watch.quality = pick;
                        watch.qualities = qualities;
                    }
                    Err(error) => watch.error = Some(error),
                }
            }
            Message::ClipCredits(ticket, result) => {
                if let Some(watch) = self.clip.as_mut().filter(|watch| watch.ticket == ticket) {
                    watch.credits = result.unwrap_or_default();
                }
            }
            Message::ClipToggle => {
                let Some(watch) = self.clip.as_mut() else { return Task::none() };
                watch.moved = Instant::now();
                watch.menu = None;
                if let Some(video) = &watch.video {
                    if watch.state.ended {
                        video.restart();
                    } else {
                        video.set_paused(!watch.state.paused);
                    }
                    watch.state = video.state();
                }
            }
            Message::ClipSeek(at) => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.drag = Some(at);
                    watch.moved = Instant::now();
                }
            }
            Message::ClipSeekDone => {
                if let Some(watch) = self.clip.as_mut() {
                    if let (Some(at), Some(video)) = (watch.drag.take(), &watch.video) {
                        video.seek(at);
                        watch.state.time = at;
                        watch.state.ended = false;
                    }
                }
            }
            Message::ClipSeekBy(step) => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.moved = Instant::now();
                    if let Some(video) = &watch.video {
                        let at = (watch.state.time + step).clamp(0.0, watch.state.duration.max(0.0));
                        video.seek(at);
                        watch.state.time = at;
                    }
                }
            }
            Message::ClipMove => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.moved = Instant::now();
                }
            }
            Message::ClipStep(forward) => {
                let Some(watch) = self.clip.as_mut() else { return Task::none() };
                let at = if forward { watch.at + 1 } else { watch.at.wrapping_sub(1) };
                if at < watch.list.len() {
                    watch.at = at;
                    let start = self.clip_start();
                    return Task::batch([start, self.clip_feed_more()]);
                }
                // Назад у первого клипа — к его началу.
                if let Some(video) = watch.video.as_ref().filter(|_| !forward) {
                    video.seek(0.0);
                }
            }
            Message::ClipPick(at) => {
                if let Some(watch) = self.clip.as_mut().filter(|watch| at < watch.list.len() && at != watch.at) {
                    watch.at = at;
                    let start = self.clip_start();
                    return Task::batch([start, self.clip_feed_more()]);
                }
            }
            Message::ClipClose => return self.clip_close(),
            Message::ClipFull => {
                let Some(watch) = self.clip.as_mut() else { return Task::none() };
                watch.full = !watch.full;
                watch.menu = None;
                let mode = if watch.full { window::Mode::Fullscreen } else { window::Mode::Windowed };
                return window::latest().and_then(move |id| window::set_mode(id, mode));
            }
            Message::ClipMenu(menu) => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.menu = if watch.menu == menu { None } else { menu };
                    watch.moved = Instant::now();
                }
            }
            Message::ClipSpeed(speed) => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.speed = speed;
                    watch.menu = None;
                    if let Some(video) = &watch.video {
                        video.set_speed(speed);
                    }
                }
            }
            Message::ClipQuality(at) => {
                if let Some(watch) = self.clip.as_mut() {
                    watch.menu = None;
                    if let (Some((_, url)), Some(video)) = (watch.qualities.get(at), &watch.video) {
                        if at != watch.quality {
                            video.switch(url, watch.state.time);
                            watch.quality = at;
                        }
                    }
                }
            }
            Message::ClipLike(clip) => {
                let (Some(uid), Some(token)) = (self.uid.clone(), self.token()) else { return Task::none() };
                let liked = !self.clip_liked(&clip);
                if liked {
                    self.clips_liked.insert(0, clip.clone());
                } else {
                    self.clips_liked.retain(|found| found.id != clip.id);
                }
                let notice = format!("Клип «{}» {}", clip.title, if liked { "добавлен в Коллекцию" } else { "удален из Коллекции" });
                let id = clip.id.clone();
                let send = Task::perform(background(move || api::set_clip_liked(&uid, &id, liked, &token)), move |result| Message::ClipLiked(clip.clone(), liked, result));
                return Task::batch([send, self.notify(notice)]);
            }
            Message::ClipLiked(clip, liked, Err(error)) => {
                // Не вышло: возвращаем как было.
                if liked {
                    self.clips_liked.retain(|found| found.id != clip.id);
                } else if !self.clip_liked(&clip) {
                    self.clips_liked.insert(0, clip);
                }
                return self.notify(error);
            }
            Message::ClipLiked(..) => {}
            Message::ClipFeed(Ok(feed)) => {
                let covers = self.clip_covers(&feed.clips);
                self.clip_feed = Some(feed);
                return covers;
            }
            // Без ленты страница клипов всё равно открывается.
            Message::ClipFeed(Err(_)) => {}
            Message::ClipFeedStart => {
                let Some(token) = self.token() else { return Task::none() };
                // Лента открывается сразу с первыми клипами карточки, сессия догонит.
                let first = self.clip_feed.as_ref().map(|feed| feed.clips.clone()).unwrap_or_default();
                let empty = first.is_empty();
                let mut open = Task::none();
                if !empty {
                    open = self.clip_open(first, 0);
                    if let Some(watch) = self.clip.as_mut() {
                        watch.feed = Some((String::new(), true));
                    }
                }
                let ticket = self.clip_ticket + u64::from(empty);
                return Task::batch([open, Task::perform(background(move || api::clip_feed_start(&token)), move |result| Message::ClipFeedLoaded(ticket, result))]);
            }
            Message::ClipFeedLoaded(ticket, result) => {
                let current = self.clip.as_ref().is_some_and(|watch| watch.ticket == ticket);
                // Ленту открыли без карточки: первые клипы пришли с сессией.
                if !current {
                    if self.clip.is_none() && ticket == self.clip_ticket + 1 {
                        if let Ok((session, clips)) = result {
                            let open = self.handle(Message::ClipOpen(clips, 0));
                            if let Some(watch) = self.clip.as_mut() {
                                watch.feed = Some((session, false));
                            }
                            return open;
                        }
                    }
                    return Task::none();
                }
                let watch = self.clip.as_mut().unwrap();
                let Ok((session, clips)) = result else {
                    if let Some((_, busy)) = watch.feed.as_mut() {
                        *busy = false;
                    }
                    return Task::none();
                };
                let fresh: Vec<Clip> = clips.into_iter().filter(|clip| !watch.list.iter().any(|have| have.id == clip.id)).collect();
                watch.list.extend(fresh.iter().cloned());
                watch.feed = Some((session, false));
                let covers = self.clip_covers(&fresh);
                return Task::batch([covers, self.clip_feed_more()]);
            }
            Message::ClipShelfPage(key, forward, total) => {
                let step = self.clips_in_row();
                let from = self.shelf_pages.entry(key).or_insert(0);
                *from = if forward { (*from + step).min(total.saturating_sub(step)) } else { from.saturating_sub(step) };
            }
            Message::ClipShare(clip) => return Task::batch([iced::clipboard::write(clip.share_link()), self.notify("Ссылка на клип скопирована".to_string())]),
            _ => {}
        }
        Task::none()
    }

    /// Карточка клипа: кадр 16:9 с длительностью, название и исполнители. Занимает два места
    /// обычной карточки.
    pub(crate) fn clip_card<'a>(&self, list: &[Clip], at: usize) -> Element<'a, Message> {
        let clip = &list[at];
        let width = self.clip_card_width();
        let height = (width * 9.0 / 16.0).round();
        let picture: Element<'a, Message> = match self.covers.get(&clip_cover_key(clip)) {
            Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).content_fit(iced::ContentFit::Cover).width(width).height(height).border_radius(8.0).into(),
            None => container(ui::icon(ui::CLIP, 40.0, ui::c().muted)).center(Length::Fixed(width)).height(height).style(ui::tile).into(),
        };
        let badge = container(text(clip_clock(clip.duration as f32)).size(12).font(ui::font_medium()).color(iced::Color::WHITE))
            .padding([2, 6])
            .style(|_| container::Style::default().background(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6)).border(iced::border::rounded(6)));
        let picture = stack![picture, container(badge).padding(8).width(width).height(height).align_right(width).align_bottom(height)];
        let mut title = row![fit(clip.full_title()).font(ui::font_medium()).color(ui::c().text).lines(2)].spacing(6);
        if clip.explicit {
            title = row![explicit_badge(), fit(clip.full_title()).font(ui::font_medium()).color(ui::c().text).lines(2)].spacing(6).align_y(Alignment::Center);
        }
        let body = column![picture, title, fit(clip.artist_names()).size(13).color(ui::c().muted)].spacing(6).width(width).clip(true);
        glow(button(body).padding(CARD_PAD).style(ui::card).on_press(Message::ClipOpen(list.to_vec(), at)), 12.0)
    }

    /// Сколько клипов в ряду: карточки около 300 точек, как кадры клипов в оригинале.
    pub(crate) fn clips_in_row(&self) -> usize {
        let room = self.main_width() - 48.0 - SCROLLBAR;
        ((room + CARD_GAP) / (CLIP_CARD + CARD_PAD * 2.0 + CARD_GAP)).floor().max(2.0) as usize
    }

    pub(crate) fn clip_card_width(&self) -> f32 {
        let columns = self.clips_in_row() as f32;
        let room = self.main_width() - 48.0 - SCROLLBAR - CARD_GAP * (columns - 1.0);
        (room / columns - CARD_PAD * 2.0).floor()
    }

    /// Карусель клипов: свой счёт карточек в ряду, стрелки листают по ряду.
    pub(crate) fn clip_shelf<'a>(&self, key: &str, title: &'a str, all: Option<Message>, list: &[Clip]) -> Element<'a, Message> {
        let in_row = self.clips_in_row();
        let total = list.len();
        let from = self.shelf_pages.get(key).copied().unwrap_or(0).min(total.saturating_sub(in_row));
        let shown = from..(from + in_row).min(total);
        let name: Element<'a, Message> = match all {
            Some(press) => more_title(title, press),
            None => container(text(title).size(24).font(ui::font_headline()).color(ui::c().text).wrapping(Wrapping::None)).padding([4, 0]).into(),
        };
        let mut head = row![name, Space::new().width(Length::Fill)].spacing(4).align_y(Alignment::Center);
        if total > in_row {
            let arrow = |icon, active: bool, forward| {
                let press = active.then(|| Message::ClipShelfPage(key.to_string(), forward, total));
                hint(glow(button(ui::icon(icon, 22.0, if active { ui::c().text } else { ui::c().control_hover })).padding(5).style(ui::ghost).on_press_maybe(press), 20.0), if forward { "Вперёд" } else { "Назад" })
            };
            head = head.push(arrow(ui::LEFT, shown.start > 0, false)).push(arrow(ui::RIGHT, shown.end < total, true));
        }
        let line = shown.fold(row![].spacing(CARD_GAP), |line, at| line.push(self.clip_card(list, at)));
        column![head, line].spacing(8).into()
    }

    /// Клипы в поиске: во «Всё» карусель с переходом на вкладку, во вкладке — сетка.
    pub(crate) fn search_clips<'a>(&self, list: &[Clip]) -> Element<'a, Message> {
        if self.search_tab == SearchTab::All {
            let all = (list.len() > self.clips_in_row()).then_some(Message::SearchTabSet(SearchTab::Clips));
            return self.clip_shelf("search:clips", "Клипы", all, list);
        }
        column![text("Клипы").size(24).font(ui::font_headline()).color(ui::c().text), grid((0..list.len()).map(|at| self.clip_card(list, at)).collect())].spacing(12).into()
    }

    /// Обложки клипов, которые сейчас где-то показаны: остальные можно выгрузить.
    pub(crate) fn clip_cover_keys(&self) -> Vec<String> {
        let artist = self.artist.iter().flat_map(|artist| if artist.clips.is_empty() { artist_video_clips(artist) } else { artist.clips.clone() });
        let watching = self.clip.iter().flat_map(|watch| watch.list.iter().cloned());
        let mut keys: Vec<String> = self.clips_liked.iter().chain(&self.clips_like).chain(&self.found.clips).cloned().chain(artist).chain(watching).map(|clip| clip_cover_key(&clip)).collect();
        if let Some(watch) = &self.clip {
            keys.push(format!("{CLIP_POSTER}{}", clip_cover_id(watch.clip())));
        }
        keys
    }

    /// Страница «Клипы»: свои и «Вам понравится», как в коллекции оригинала.
    pub(crate) fn view_clips(&self) -> Element<'_, Message> {
        let head = row![back_button(), text("Клипы").size(36).font(ui::font_headline()).color(ui::c().text)].spacing(12).align_y(Alignment::Center);
        if self.clips_loading && self.clips_like.is_empty() && self.clips_liked.is_empty() {
            return column![head, self.area(skeleton_list())].spacing(16).height(Length::Fill).into();
        }
        let block = |title| text(title).size(24).font(ui::font_headline()).color(ui::c().text);
        let grid_of = |list: &[Clip]| grid((0..list.len()).map(|at| self.clip_card(list, at)).collect());
        let mut body = column![].spacing(12);
        if let Some(feed) = self.clip_feed.as_ref().filter(|feed| !feed.clips.is_empty()) {
            body = body.push(self.clip_feed_card(feed)).push(Space::new().height(8));
        }
        if self.clips_liked.is_empty() {
            let empty = column![
                text("Ставьте лайки клипам, и они появятся тут").size(20).font(ui::font_headline()).color(ui::c().text),
                text("А пока — посмотрите наши рекомендации").color(ui::c().muted),
            ]
            .spacing(6);
            body = body.push(container(empty).padding([8, 0]));
        } else {
            body = body.push(block("Мои клипы")).push(grid_of(&self.clips_liked));
        }
        if !self.clips_like.is_empty() {
            body = body.push(Space::new().height(8)).push(block("Вам понравится")).push(grid_of(&self.clips_like));
        }
        let center = ui::scroll(container(body).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill);
        column![head, center].spacing(16).height(Length::Fill).into()
    }

    /// Карточка «Время клипов»: название, описание, кадры первых клипов и «Смотреть».
    fn clip_feed_card<'a>(&self, feed: &api::ClipFeed) -> Element<'a, Message> {
        let side = 120.0;
        let height = (side * 9.0 / 16.0_f32).round();
        let thumbs = feed.clips.iter().take(5).fold(row![].spacing(8), |line, clip| {
            line.push(match self.covers.get(&clip_cover_key(clip)) {
                Some(handle) => Element::from(image(handle.clone()).content_fit(iced::ContentFit::Cover).width(side).height(height).border_radius(6.0)),
                None => container(Space::new()).width(side).height(height).style(ui::tile).into(),
            })
        });
        let watch = shine(button(row![ui::icon(ui::PLAY, 20.0, ui::c().on_accent), text(feed.button.clone()).font(ui::font_medium()).color(ui::c().on_accent)].spacing(6).align_y(Alignment::Center)).padding([9, 18]).style(ui::accent).on_press(Message::ClipFeedStart), 24.0);
        let words = column![
            text(feed.title.clone()).size(28).font(ui::font_headline()).color(ui::c().text),
            text(feed.description.clone()).color(ui::c().muted),
            Space::new().height(6),
            watch,
        ]
        .spacing(4)
        .width(Length::Fill);
        let content = row![words, thumbs].spacing(24).align_y(Alignment::Center);
        container(content).padding(20).width(Length::Fill).style(ui::tile).into()
    }

    /// Окно клипа поверх всего: видео со своим управлением, под ним название, исполнители,
    /// лайк и «О клипе», справа — что дальше.
    pub(crate) fn view_clip(&self) -> Option<Element<'_, Message>> {
        let watch = self.clip.as_ref()?;
        let (width, height) = *WINDOW.lock().unwrap();
        let back = |_: &iced::Theme| container::Style::default().background(iced::Color::from_rgb8(0x0d, 0x0d, 0x0d));
        if watch.full {
            let player = self.clip_player(watch, width, height, 0.0);
            return Some(anim::block(container(player).width(Length::Fill).height(Length::Fill).style(back)));
        }
        let side = watch.list.len() > 1 && width - 48.0 - NEXT_WIDTH - 24.0 >= NEXT_ROOM;
        let room = if side { width - 48.0 - NEXT_WIDTH - 24.0 } else { width - 48.0 };
        // Видео 16:9 во всю ширину, но так, чтобы под ним оставалось место для названия.
        let video_width = room.min((height - 64.0 - 150.0) * 16.0 / 9.0).max(320.0);
        let video_height = (video_width * 9.0 / 16.0).round();
        let close = hint(glow(button(ui::icon(ui::CLOSE, 26.0, iced::Color::WHITE)).padding(6).style(ui::ghost).on_press(Message::ClipClose), 20.0), "Закрыть");
        let top = row![text("Клип").size(16).font(ui::font_medium()).color(iced::Color::from_rgb8(0xb3, 0xb3, 0xb3)), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
        let player = self.clip_player(watch, video_width, video_height, 12.0);
        let mut main = column![player, self.clip_about(watch)].spacing(16).width(video_width);
        if let Some(error) = &watch.error {
            main = main.push(text(human_error(error)).color(ui::c().danger));
        }
        let main = ui::scroll(main).width(video_width).height(Length::Fill);
        let mut content = row![main].spacing(24);
        if side {
            content = content.push(self.clip_next(watch));
        }
        let page = column![top, container(content).center_x(Length::Fill).height(Length::Fill)].spacing(12).padding(Padding::from([16, 24])).width(Length::Fill).height(Length::Fill);
        let page = mouse_area(container(page).width(Length::Fill).height(Length::Fill).style(back)).on_move(|_| Message::ClipMove);
        Some(anim::block(page))
    }

    /// Видео с поверхностью управления. В полноэкранном режиме — на всё окно.
    fn clip_player<'a>(&'a self, watch: &'a Watch, width: f32, height: f32, radius: f32) -> Element<'a, Message> {
        let clip = watch.clip();
        let full = watch.full;
        let (width, height) = if full { (Length::Fill, Length::Fill) } else { (Length::Fixed(width), Length::Fixed(height)) };
        let mut layers = stack![].width(width).height(height);
        // Пока нет кадра — обложка клипа.
        let framed = watch.video.as_ref().is_some_and(|video| video.serial() > 0);
        if !framed {
            let poster: Element<'a, Message> = match self.covers.get(&format!("{CLIP_POSTER}{}", clip_cover_id(clip))).or_else(|| self.covers.get(&clip_cover_key(clip))) {
                Some(handle) => image(handle.clone()).content_fit(iced::ContentFit::Contain).width(Length::Fill).height(Length::Fill).border_radius(radius).into(),
                None => Space::new().width(Length::Fill).height(Length::Fill).into(),
            };
            layers = layers.push(container(poster).width(Length::Fill).height(Length::Fill).style(move |_| container::Style::default().background(iced::Color::BLACK).border(iced::border::rounded(radius))));
        }
        if let Some(video) = &watch.video {
            layers = layers.push(iced::widget::shader(video.view(radius)).width(Length::Fill).height(Length::Fill));
        }
        let shown = watch.controls_shown();
        // Нажатие по видео — пауза, двойное — на весь экран.
        let surface = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_press(Message::ClipToggle)
            .on_double_click(Message::ClipFull)
            .on_move(|_| Message::ClipMove)
            .interaction(if shown > 0.0 { mouse::Interaction::Pointer } else { mouse::Interaction::Hidden });
        layers = layers.push(surface);
        if watch.video.is_none() && watch.error.is_none() || watch.state.buffering && watch.video.is_some() {
            layers = layers.push(container(text("Загружаем…").size(16).color(iced::Color::WHITE)).center(Length::Fill));
        }
        if shown > 0.0 {
            layers = layers.push(container(reveal(self.clip_controls(watch)).veil(iced::Color::TRANSPARENT).shift(0.0, 8.0 * (1.0 - shown))).width(Length::Fill).height(Length::Fill).align_y(Alignment::End));
        }
        if let Some(menu) = watch.menu {
            layers = layers.push(container(self.clip_menu(watch, menu)).width(Length::Fill).height(Length::Fill).align_x(Alignment::End).align_y(Alignment::End).padding(Padding::ZERO.right(16).bottom(64)));
        }
        container(layers).width(width).height(height).into()
    }

    /// Полоса управления внизу видео.
    fn clip_controls<'a>(&'a self, watch: &'a Watch) -> Element<'a, Message> {
        let white = iced::Color::WHITE;
        let soft = iced::Color::from_rgba(1.0, 1.0, 1.0, 0.75);
        let total = watch.state.duration.max(watch.clip().duration as f32).max(1.0);
        let at = watch.drag.unwrap_or(watch.state.time).min(total);
        let seek = canvas(seek::Seek { value: at, total, on_change: Message::ClipSeek, on_done: Some(Message::ClipSeekDone) }).width(Length::Fill).height(SEEK_HEIGHT);
        let button_of = |icon, size: f32, press: Message, label: &'static str| hint(glow(button(ui::icon(icon, size, white)).padding(6).style(ui::ghost).on_press(press), 20.0), label);
        let play = if watch.state.ended {
            button_of(ui::REPLAY, 26.0, Message::ClipToggle, "Смотреть заново")
        } else if watch.state.paused || watch.video.is_none() {
            button_of(ui::PLAY, 26.0, Message::ClipToggle, "Воспроизведение")
        } else {
            button_of(ui::PAUSE, 26.0, Message::ClipToggle, "Пауза")
        };
        let mut line = row![play].spacing(4).align_y(Alignment::Center);
        if watch.list.len() > 1 {
            line = line.push(button_of(ui::PREVIOUS, 22.0, Message::ClipStep(false), "Предыдущий клип")).push(button_of(ui::NEXT, 22.0, Message::ClipStep(true), "Следующий клип"));
        }
        line = line.push(text(format!("{} / {}", clip_clock(at), clip_clock(total))).size(14).color(soft)).push(Space::new().width(Length::Fill)).push(self.view_volume(22.0, Some(100.0)));
        let speed = if watch.speed == 1.0 { "1×".to_string() } else { format!("{}×", speed_text(watch.speed)) };
        line = line.push(hint(glow(button(text(speed).size(14).font(ui::font_medium()).color(white)).padding([6, 10]).style(ui::ghost).on_press(Message::ClipMenu(Some(ClipMenu::Speed))), 16.0), "Скорость"));
        if watch.qualities.len() > 1 {
            let label = watch.qualities.get(watch.quality).map_or_else(String::new, |(height, _)| format!("{height}p"));
            line = line.push(hint(glow(button(text(label).size(14).font(ui::font_medium()).color(white)).padding([6, 10]).style(ui::ghost).on_press(Message::ClipMenu(Some(ClipMenu::Quality))), 16.0), "Качество"));
        }
        line = line.push(button_of(if watch.full { ui::FULLSCREEN_EXIT } else { ui::FULLSCREEN }, 22.0, Message::ClipFull, if watch.full { "Выйти из полноэкранного режима" } else { "Во весь экран" }));
        let shade = |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Gradient(iced::Gradient::Linear(iced::gradient::Linear::new(iced::Radians(0.0)).add_stop(0.0, iced::Color::from_rgba(0.0, 0.0, 0.0, 0.7)).add_stop(1.0, iced::Color::TRANSPARENT)))),
            ..container::Style::default()
        };
        let bar = column![seek, line].spacing(2).padding(Padding::from([24, 14]).bottom(10));
        // Под полосой — своя мышиная область, чтобы нажатие по кнопкам не ставило видео на паузу.
        mouse_area(container(bar).width(Length::Fill).style(shade)).on_press(Message::ClipMove).on_move(|_| Message::ClipMove).into()
    }

    fn clip_menu<'a>(&'a self, watch: &'a Watch, menu: ClipMenu) -> Element<'a, Message> {
        let item = |label: String, chosen: bool, press: Message| {
            let mark: Element<'a, Message> = if chosen { ui::icon(ui::CHECK, 18.0, ui::c().accent) } else { Space::new().width(18).into() };
            button(row![mark, text(label).color(ui::c().text)].spacing(10).align_y(Alignment::Center)).padding([8, 14]).width(Length::Fill).style(ui::menu_item).on_press(press)
        };
        let list = match menu {
            ClipMenu::Speed => CLIP_SPEEDS.iter().fold(column![], |list, &speed| {
                let label = if speed == 1.0 { "Обычная".to_string() } else { format!("{}×", speed_text(speed)) };
                list.push(item(label, watch.speed == speed, Message::ClipSpeed(speed)))
            }),
            ClipMenu::Quality => watch.qualities.iter().enumerate().fold(column![], |list, (at, (height, _))| list.push(item(format!("{height}p"), watch.quality == at, Message::ClipQuality(at)))),
        };
        container(list).padding(6).width(180).style(ui::popover).into()
    }

    /// Под видео: название, исполнители, лайк, ссылка и «О клипе».
    fn clip_about<'a>(&'a self, watch: &'a Watch) -> Element<'a, Message> {
        let clip = watch.clip();
        let mut title = row![text(clip.title.clone()).size(28).font(ui::font_headline()).color(iced::Color::WHITE)].spacing(10).align_y(Alignment::Center);
        if !clip.version.is_empty() {
            title = title.push(text(clip.version.clone()).size(20).color(iced::Color::from_rgb8(0x99, 0x99, 0x99)));
        }
        if clip.explicit {
            title = title.push(explicit_badge());
        }
        let links = artist_links(&clip.artists, 16.0, ui::font(), iced::Color::from_rgb8(0xb3, 0xb3, 0xb3));
        let mut actions = row![].spacing(10).align_y(Alignment::Center);
        if !clip.id.is_empty() {
            actions = actions.push(self.page_like(self.clip_liked(clip), Message::ClipLike(clip.clone())));
            actions = actions.push(hint(shine(button(container(ui::icon(ui::LINK, 22.0, ui::c().text)).center(22)).padding(9).style(ui::secondary).on_press(Message::ClipShare(clip.clone())), 24.0), "Ссылка на клип"));
        }
        let mut about = column![title, links, actions].spacing(10);
        if !watch.credits.is_empty() {
            let facts = watch.credits.iter().fold(column![text("О клипе").size(20).font(ui::font_headline()).color(iced::Color::WHITE)].spacing(6), |list, (name, value)| {
                list.push(row![text(name.clone()).width(160).color(iced::Color::from_rgb8(0x99, 0x99, 0x99)), text(value.clone()).color(iced::Color::WHITE)].spacing(12))
            });
            about = about.push(Space::new().height(4)).push(facts);
        }
        about.into()
    }

    /// Колонка «Далее»: остальные клипы списка.
    fn clip_next<'a>(&'a self, watch: &'a Watch) -> Element<'a, Message> {
        let thumb_w = 150.0;
        let thumb_h = (thumb_w * 9.0 / 16.0_f32).round();
        let items = watch.list.iter().enumerate().fold(column![].spacing(4), |list, (at, clip)| {
            let picture: Element<'a, Message> = match self.covers.get(&clip_cover_key(clip)) {
                Some(handle) => image(handle.clone()).content_fit(iced::ContentFit::Cover).width(thumb_w).height(thumb_h).border_radius(6.0).into(),
                None => container(Space::new()).width(thumb_w).height(thumb_h).style(ui::tile).into(),
            };
            let tint = if at == watch.at { ui::c().accent } else { iced::Color::WHITE };
            let words = column![fit(clip.full_title()).font(ui::font_medium()).color(tint).lines(2), fit(clip.artist_names()).size(13).color(iced::Color::from_rgb8(0x99, 0x99, 0x99))].spacing(4).width(Length::Fill);
            let item = row![picture, words].spacing(12);
            list.push(button(item).padding(6).width(Length::Fill).style(ui::menu_item).on_press(Message::ClipPick(at)))
        });
        let head = text("Далее").size(20).font(ui::font_headline()).color(iced::Color::WHITE);
        column![head, ui::scroll(items).height(Length::Fill)].spacing(10).width(NEXT_WIDTH).height(Length::Fill).into()
    }
}

/// Ключ обложки клипа: по номеру, а у клипа без номера — по номеру ролика.
pub(crate) fn clip_cover_key(clip: &Clip) -> String {
    format!("{CLIP_COVER}{}", clip_cover_id(clip))
}

fn clip_cover_id(clip: &Clip) -> String {
    if clip.id.is_empty() { clip.player.clone().unwrap_or_default() } else { clip.id.clone() }
}

fn speed_text(speed: f32) -> String {
    let text = format!("{speed:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.replace('.', ",")
}

/// Значок «E» у клипа с ненормативной лексикой.
fn explicit_badge<'a>() -> Element<'a, Message> {
    container(text("E").size(11).font(ui::font_medium()).color(ui::c().strong)).center(16).style(ui::explicit).into()
}

/// Клипы исполнителя из старого списка (`brief-info`): только у роликов Яндекса есть номер плеера.
pub(crate) fn artist_video_clips(artist: &Artist) -> Vec<Clip> {
    artist
        .videos
        .iter()
        .filter(|video| video.url.contains("frontend.vh.yandex.ru"))
        .map(|video| Clip { id: String::new(), title: video.title.clone(), cover: video.cover.clone(), player: Some(video.id.clone()), artists: vec![(artist.name.clone(), artist.id.clone())], ..Clip::default() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_reads_like_russian_number() {
        assert_eq!(speed_text(1.25), "1,25");
        assert_eq!(speed_text(0.5), "0,5");
        assert_eq!(speed_text(2.0), "2");
    }
}
