//! Концерты, как в оригинале: лента «Топ для вас» и «Концерты для вас» по выбранному городу,
//! страница концерта с площадкой, картой и другими концертами исполнителя. Билеты продаёт
//! Афиша: оригинал показывает её у себя в окне, Tune открывает ту же страницу в браузере.

use std::fs;

use crate::*;
use api::Concert;
use iced::widget::{column, row};

pub const CONCERT_COVER: &str = "concert:";
const CONCERT_BIG: &str = "concert-big:";
const CONCERT_MAP: &str = "concert-map:";
/// Ширина карты площадки: столько отдаёт Яндекс.Карты за раз (картинка вдвое плотнее).
const MAP_WIDTH: f32 = 650.0;
const MAP_HEIGHT: f32 = 300.0;
const TOP_SHELF: &str = "concerts:top";
const OTHERS_SHELF: &str = "concert:others";
/// Сколько популярных треков исполнителя на странице концерта.
const CONCERT_TRACKS: usize = 5;
const CITY_FILE: &str = "concerts-city.json";

/// Выбранный город: номер и название. Нет файла — «Рядом».
pub fn load_city() -> Option<(u64, String)> {
    let text = fs::read(api::data_dir().join(CITY_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&text).ok()?;
    Some((v["id"].as_u64()?, v["name"].as_str()?.to_string()))
}

fn save_city(city: &Option<(u64, String)>) {
    let path = api::data_dir().join(CITY_FILE);
    match city {
        Some((id, name)) => {
            let _ = fs::write(path, serde_json::json!({ "id": id, "name": name }).to_string());
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn concert_cover_key(concert: &Concert) -> String {
    format!("{CONCERT_COVER}{}", concert.id)
}

/// Города, у которых название начинается с введённого или содержит его.
fn cities_matching<'a>(cities: &'a [(u64, String)], query: &str) -> Vec<&'a (u64, String)> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return cities.iter().collect();
    }
    let (mut first, mut rest): (Vec<_>, Vec<_>) = (Vec::new(), Vec::new());
    for city in cities {
        let name = city.1.to_lowercase();
        if name.starts_with(&query) {
            first.push(city);
        } else if name.contains(&query) {
            rest.push(city);
        }
    }
    first.extend(rest);
    first
}

impl App {
    fn concert_location(&self) -> Option<u64> {
        self.concert_city.as_ref().map(|(id, _)| *id)
    }

    /// Лента по выбранному городу и, один раз, список городов.
    pub(crate) fn concerts_load(&mut self) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.concerts_loading = true;
        self.concerts_error = false;
        let location = self.concert_location();
        let feed = {
            let token = token.clone();
            Task::perform(background(move || api::concerts_feed(location, &token).map(|list| (api::concerts_top(&token), list))), move |r| Message::ConcertsLoaded(location, r))
        };
        let cities = if self.concert_cities.is_empty() { Task::perform(background(move || api::concert_cities(&token)), Message::ConcertCities) } else { Task::none() };
        Task::batch([feed, cities])
    }

    fn concert_covers(&self, list: &[Concert]) -> Task<Message> {
        self.fetch_cover_jobs(list.iter().filter_map(|c| Some((concert_cover_key(c), c.cover.clone()?))).collect(), "400x400")
    }

    /// Обложки концертов, которые сейчас где-то показаны: остальные можно выгрузить.
    pub(crate) fn concert_cover_keys(&self) -> Vec<String> {
        let page = self.concert.iter().flat_map(|page| page.others.iter().chain(std::iter::once(&page.concert)));
        let artist = self.artist.iter().flat_map(|artist| artist.concerts.iter());
        let mut keys: Vec<String> = self.concerts.iter().chain(page).chain(artist).map(concert_cover_key).collect();
        if let Some(page) = &self.concert {
            keys.push(format!("{CONCERT_BIG}{}", page.concert.id));
            keys.push(format!("{CONCERT_MAP}{}", page.concert.id));
        }
        keys
    }

    pub(crate) fn concert_open(&mut self, id: String) -> Task<Message> {
        let Some(token) = self.token() else { return Task::none() };
        self.player_full = false;
        self.menu = None;
        self.remember();
        self.section = Section::Concert;
        self.error = None;
        self.retry = None;
        if self.concert.as_ref().is_some_and(|page| page.concert.id == id) {
            return Task::none();
        }
        self.concert = None;
        let location = self.concert_location();
        let wanted = id.clone();
        Task::perform(background(move || api::concert_page(&id, location, &token)), move |r| Message::ConcertLoaded(wanted.clone(), r))
    }

    pub(crate) fn concert_handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ConcertsLoaded(location, result) => {
                if location != self.concert_location() {
                    return Task::none();
                }
                self.concerts_loading = false;
                match result {
                    Ok((top, list)) => {
                        let covers = self.concert_covers(&list);
                        self.concerts_top = top;
                        self.concerts = list;
                        self.shelf_pages.remove(TOP_SHELF);
                        return covers;
                    }
                    Err(error) => {
                        // Ошибку показываем на месте ленты, как оригинал.
                        api::log(&format!("концерты: {error}"));
                        self.concerts_error = true;
                        if api::network_error(&error) {
                            return self.fail(error);
                        }
                    }
                }
            }
            Message::ConcertCities(result) => match result {
                Ok(cities) => self.concert_cities = cities,
                Err(error) => api::log(&format!("города концертов: {error}")),
            },
            Message::ConcertCityPick(city) => {
                self.dialog = None;
                if city == self.concert_city {
                    return Task::none();
                }
                save_city(&city);
                self.concert_city = city;
                self.concerts.clear();
                return self.concerts_load();
            }
            Message::ConcertOpen(id) => return self.concert_open(id),
            Message::ConcertLoaded(id, result) => {
                if self.section != Section::Concert || self.concert.is_some() {
                    return Task::none();
                }
                match result {
                    Ok(page) if page.concert.id == id => {
                        let mut jobs = Vec::new();
                        if let Some(cover) = page.covers.first().cloned().or(page.concert.cover.clone()) {
                            jobs.push((format!("{CONCERT_BIG}{id}"), cover));
                        }
                        let big = self.fetch_cover_jobs(jobs, "400x400");
                        let map = page.venue.map_image.clone().map(|uri| self.fetch_cover_jobs(vec![(format!("{CONCERT_MAP}{id}"), uri)], "650,300")).unwrap_or_else(Task::none);
                        let others = self.concert_covers(&page.others);
                        let tracks = self.fetch_cover_jobs(page.tracks.iter().take(CONCERT_TRACKS).filter_map(|t| Some((t.id.clone(), t.cover.clone()?))).collect(), "100x100");
                        self.concert = Some(page);
                        return Task::batch([big, map, others, tracks]);
                    }
                    Ok(_) => {}
                    Err(error) => return self.fail_retry(error, Message::ConcertOpen(id)),
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Значок даты на обложке: число и месяц, как на карточке оригинала.
    fn concert_date<'a>(&self, concert: &Concert, big: bool) -> Element<'a, Message> {
        let Some((day, month)) = concert.day_month() else { return Space::new().into() };
        let (day_size, month_size) = if big { (28, 14) } else { (20, 12) };
        let body = column![
            text(day.to_string()).size(day_size).font(ui::font_headline()).color(iced::Color::BLACK),
            text(month).size(month_size).font(ui::font_medium()).color(iced::Color::from_rgb(0.3, 0.3, 0.3)),
        ]
        .align_x(Alignment::Center)
        .spacing(0);
        container(body).padding([6, 10]).style(|_| container::Style::default().background(iced::Color::WHITE).border(iced::border::rounded(10))).into()
    }

    fn concert_picture<'a>(&self, key: &str, width: f32, height: f32) -> Element<'a, Message> {
        match self.covers.get(key) {
            Some(handle) => image(handle.clone()).opacity(self.cover_alpha(handle)).content_fit(iced::ContentFit::Cover).width(width).height(height).border_radius(8.0).into(),
            None => container(ui::icon(ui::CONCERTS, 40.0, ui::c().muted)).center(Length::Fixed(width)).height(height).style(ui::tile).into(),
        }
    }

    /// Карточка концерта: обложка с датой, название, город с площадкой и цена.
    pub(crate) fn concert_card<'a>(&self, concert: &Concert) -> Element<'a, Message> {
        let side = self.card_side();
        let picture = stack![self.concert_picture(&concert_cover_key(concert), side, side), container(self.concert_date(concert, false)).padding(8)];
        let mut body = column![picture, fit(concert.title.clone()).font(ui::font_medium()).color(ui::c().text).lines(2), fit(concert.where_()).size(13).color(ui::c().muted)].spacing(6).width(side).clip(true);
        if let Some(price) = &concert.price {
            body = body.push(text(price.clone()).size(13).color(ui::c().text));
        }
        glow(button(body).padding(CARD_PAD).style(ui::card).on_press(Message::ConcertOpen(concert.id.clone())), 12.0)
    }

    /// Большая карточка «Топ для вас»: шире обычной, с местом в топе.
    fn concert_top_card<'a>(&self, concert: &Concert, rank: usize) -> Element<'a, Message> {
        let width = self.clip_card_width();
        let height = (width * 0.6).round();
        let place = container(text(rank.to_string()).size(56).font(ui::font_headline()).color(iced::Color::WHITE)).padding(Padding::new(4.0).left(14));
        let shade = iced::gradient::Linear::new(iced::Degrees(180.0)).add_stop(0.4, iced::Color::TRANSPARENT).add_stop(1.0, iced::Color::BLACK.scale_alpha(0.6));
        let shade = container(Space::new()).width(width).height(height).style(move |_| container::Style::default().background(iced::Background::Gradient(shade.into())).border(iced::border::rounded(8)));
        let picture = stack![
            self.concert_picture(&concert_cover_key(concert), width, height),
            shade,
            container(self.concert_date(concert, true)).padding(10),
            container(place).width(width).height(height).align_bottom(height),
        ];
        let mut line = vec![concert.when(), concert.where_()];
        line.retain(|part| !part.is_empty());
        let mut body = column![picture, fit(concert.title.clone()).size(18).font(ui::font_medium()).color(ui::c().text).lines(1), fit(line.join(" · ")).size(13).color(ui::c().muted)].spacing(6).width(width).clip(true);
        if let Some(price) = &concert.price {
            body = body.push(text(price.clone()).size(13).color(ui::c().text));
        }
        glow(button(body).padding(CARD_PAD).style(ui::card).on_press(Message::ConcertOpen(concert.id.clone())), 12.0)
    }

    /// Ряд больших карточек со стрелками, как карусель топа у оригинала.
    fn concert_top<'a>(&self, list: &[Concert]) -> Element<'a, Message> {
        let in_row = self.clips_in_row();
        let total = list.len();
        let from = self.shelf_pages.get(TOP_SHELF).copied().unwrap_or(0).min(total.saturating_sub(in_row));
        let shown = from..(from + in_row).min(total);
        let mut head = row![text("Топ для вас").size(24).font(ui::font_headline()).color(ui::c().text).wrapping(Wrapping::None), Space::new().width(Length::Fill)].spacing(4).align_y(Alignment::Center);
        if total > in_row {
            let arrow = |icon, active: bool, forward| {
                let press = active.then(|| Message::ClipShelfPage(TOP_SHELF.to_string(), forward, total));
                hint(glow(button(ui::icon(icon, 22.0, if active { ui::c().text } else { ui::c().control_hover })).padding(5).style(ui::ghost).on_press_maybe(press), 20.0), if forward { "Вперёд" } else { "Назад" })
            };
            head = head.push(arrow(ui::LEFT, shown.start > 0, false)).push(arrow(ui::RIGHT, shown.end < total, true));
        }
        let line = shown.fold(row![].spacing(CARD_GAP), |line, at| line.push(self.concert_top_card(&list[at], at + 1)));
        column![head, line].spacing(8).into()
    }

    /// Кнопка города справа от заголовка: «Рядом» или выбранный город.
    fn concert_city_button<'a>(&self) -> Element<'a, Message> {
        let name = self.concert_city.as_ref().map_or("Рядом".to_string(), |(_, name)| name.clone());
        let face = row![text(name).size(15).font(ui::font_medium()).color(ui::c().text), ui::icon(ui::COLLAPSE, 18.0, ui::c().muted)].spacing(6).align_y(Alignment::Center);
        let press = (!self.concert_cities.is_empty()).then_some(Message::DialogOpen(Dialog::ConcertCity));
        shine(button(face).padding([8, 16]).style(ui::secondary).on_press_maybe(press), 24.0)
    }

    /// Раздел «Концерты».
    pub(crate) fn view_concerts(&self) -> Element<'_, Message> {
        let head = row![text("Концерты").size(36).font(ui::font_headline()).color(ui::c().text), Space::new().width(Length::Fill), self.concert_city_button()].spacing(12).align_y(Alignment::Center);
        if self.concerts_loading && self.concerts.is_empty() {
            return column![head, self.area(skeleton_cards(self.card_side()))].spacing(16).height(Length::Fill).into();
        }
        let block = |title| text(title).size(24).font(ui::font_headline()).color(ui::c().text);
        let mut body = column![].spacing(12);
        if self.concerts_error && self.concerts.is_empty() {
            let retry = shine(button(text("Повторить").color(ui::c().text)).padding([8, 18]).style(ui::secondary).on_press(Message::Open(Section::Concerts)), 24.0);
            body = body.push(column![text("При загрузке концертов произошла ошибка").size(20).font(ui::font_headline()).color(ui::c().text), retry].spacing(12));
        } else if self.concerts.is_empty() {
            let empty = column![
                text("Ничего не нашли").size(20).font(ui::font_headline()).color(ui::c().text),
                text("В этом городе пока нет концертов — попробуйте выбрать другой").color(ui::c().muted),
            ]
            .spacing(6);
            body = body.push(empty);
        } else {
            let top = self.concerts_top.min(self.concerts.len());
            if top > 0 {
                body = body.push(self.concert_top(&self.concerts[..top])).push(Space::new().height(8));
            }
            if top < self.concerts.len() {
                body = body.push(block("Концерты для вас")).push(grid(self.concerts[top..].iter().map(|concert| self.concert_card(concert)).collect()));
            }
        }
        let center = ui::scroll(container(body).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill);
        column![head, center].spacing(16).height(Length::Fill).into()
    }

    /// Окно «Найти город»: поиск, «Концерты рядом» и список городов с отметкой выбранного.
    pub(crate) fn view_concert_city(&self) -> Element<'_, Message> {
        let alpha = self.dialog_fade.shown();
        let tint = |color: iced::Color| color.scale_alpha(alpha);
        let close = glow(button(ui::icon(ui::CLOSE, 20.0, tint(ui::c().muted))).padding(4).style(ui::ghost).on_press(Message::MenuClose), 16.0);
        let head = row![text("Найти город").size(20).font(ui::font_medium()).color(tint(ui::c().text)), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
        let search = text_input("Город", &self.dialog_text).id(DIALOG_INPUT).on_input(Message::DialogInput).on_submit(Message::DialogConfirm).padding([10, 16]).style(ui::input_faded(alpha));
        let item = |label: String, chosen: bool, pick: Option<(u64, String)>| -> Element<'_, Message> {
            let mut face = row![text(label).font(ui::font_medium()).color(tint(ui::c().text)).width(Length::Fill)].align_y(Alignment::Center);
            if chosen {
                face = face.push(ui::icon(ui::CHECK, 18.0, tint(ui::c().accent)));
            }
            glow(button(face).width(Length::Fill).padding([10, 12]).style(ui::row).on_press(Message::ConcertCityPick(pick)), 8.0)
        };
        let found = cities_matching(&self.concert_cities, &self.dialog_text);
        let mut list = column![].spacing(2);
        if self.dialog_text.trim().is_empty() {
            list = list.push(item("Концерты рядом".to_string(), self.concert_city.is_none(), None));
        }
        // Выбранный город — первым, как в оригинале.
        let chosen = self.concert_location();
        let ordered = found.iter().filter(|city| Some(city.0) == chosen).chain(found.iter().filter(|city| Some(city.0) != chosen));
        for city in ordered {
            list = list.push(item(city.1.clone(), Some(city.0) == chosen, Some((*city).clone())));
        }
        let content: Element<'_, Message> = if found.is_empty() && !self.dialog_text.trim().is_empty() {
            column![text("Ничего не нашли").size(16).font(ui::font_medium()).color(tint(ui::c().text)), text("Попробуйте написать по-другому").color(tint(ui::c().muted))].spacing(4).into()
        } else {
            ui::scroll(container(list).padding(Padding::ZERO.right(SCROLLBAR))).height(360).into()
        };
        let body = column![head, search, content].spacing(16);
        let window = reveal(container(body).width(DIALOG_WIDTH).padding(24).style(ui::popover_faded(alpha))).scale(0.94 + 0.06 * alpha, 0.5, 0.5).shift(0.0, 10.0 * (1.0 - alpha));
        let dim = iced::Color::BLACK.scale_alpha(0.6 * alpha);
        reveal(mouse_area(container(mouse_area(window).on_press(Message::DialogInput(self.dialog_text.clone()))).center(Length::Fill).style(move |_| container::Style::default().background(dim))).on_press(Message::MenuClose)).inert(!self.dialog_fade.live).into()
    }

    /// Enter в поиске города выбирает первый найденный.
    pub(crate) fn concert_city_first(&self) -> Option<Message> {
        cities_matching(&self.concert_cities, &self.dialog_text).first().map(|city| Message::ConcertCityPick(Some((*city).clone())))
    }

    /// Страница концерта.
    pub(crate) fn view_concert(&self) -> Element<'_, Message> {
        let Some(page) = &self.concert else { return self.skeleton_page(false) };
        let concert = &page.concert;
        let id = &concert.id;
        let side = if self.main_width() < PAGE_COVER_WIDTH { PAGE_COVER_SMALL } else { PAGE_COVER };
        let big = format!("{CONCERT_BIG}{id}");
        let cover = stack![self.concert_picture(&big, side, side), container(self.concert_date(concert, true)).padding(10)];
        let mut kicker = vec![concert.kind_name()];
        if !concert.rating.is_empty() {
            kicker.push(concert.rating.clone());
        }
        let mut info = column![text(kicker.join(" · ")).size(14).color(ui::c().muted), fit(concert.title.clone()).size(40).font(ui::font_headline()).color(ui::c().strong)].spacing(6).width(Length::Fill).clip(true);
        info = info.push(fit(concert.when()).color(ui::c().text)).push(fit(concert.where_()).color(ui::c().muted));
        let mut actions = row![].spacing(8).align_y(Alignment::Center);
        if let Some(url) = concert.tickets_url() {
            let label = match &concert.price {
                Some(price) => format!("Билеты {price}"),
                None => "Билеты".to_string(),
            };
            actions = actions.push(shine(button(row![ui::icon(ui::TICKET, 20.0, ui::c().on_accent), text(label).font(ui::font_medium()).color(ui::c().on_accent)].spacing(6).align_y(Alignment::Center)).padding([8, 18]).style(ui::accent).on_press(Message::OpenUrl(url)), 24.0));
        }
        if let Some(artist) = &page.artist {
            let label = if page.artist_name.is_empty() { "Исполнитель".to_string() } else { page.artist_name.clone() };
            actions = actions.push(shine(button(row![ui::icon(ui::ARTIST, 20.0, ui::c().text), text(label).color(ui::c().text)].spacing(6).align_y(Alignment::Center)).padding([8, 18]).style(ui::secondary).on_press(Message::ArtistOpen(artist.clone())), 24.0));
        }
        let info = info.push(Space::new().height(8)).push(actions.wrap().vertical_spacing(8));
        let tint = self.covers.get(&big).and_then(average_color).map(|color| color.scale_alpha(if ui::is_light() { 0.35 } else { 0.5 }));
        let header = container(row![container(back_button()).height(PAGE_COVER), cover, info].spacing(16).align_y(Alignment::Center)).padding(PAGE_HEADER_PAD).width(Length::Fill).style(move |_| match tint {
            Some(color) => {
                let fade = iced::gradient::Linear::new(iced::Degrees(180.0)).add_stop(0.0, color).add_stop(1.0, color.scale_alpha(0.0));
                container::Style::default().background(iced::Background::Gradient(fade.into())).border(iced::border::rounded(12))
            }
            None => container::Style::default(),
        });

        let block = |title: String| text(title).size(24).font(ui::font_headline()).color(ui::c().text);
        let mut body = column![header].spacing(16);
        if !page.about.is_empty() {
            let about_kind = match concert.kind.as_str() {
                "festival" => "О фестивале".to_string(),
                "musical" => "О мюзикле".to_string(),
                "tribute" => "О трибьюте".to_string(),
                "" | "concert" | "unspecified" => "О концерте".to_string(),
                other => format!("О {other}"),
            };
            let mut about = column![block(about_kind), text(page.about.clone()).color(ui::c().text)].spacing(8);
            if !page.about_source.is_empty() {
                about = about.push(text(format!("Источник: {}", page.about_source)).size(13).color(ui::c().muted));
            }
            body = body.push(container(about).padding([0, 8]));
        }
        body = body.push(container(self.concert_venue(page)).padding([0, 8]));
        if !page.tracks.is_empty() {
            let tracks = page.tracks.iter().take(CONCERT_TRACKS).enumerate().fold(column![].spacing(0), |list, (index, track)| list.push(self.view_track(Section::Concert, index, track)));
            body = body.push(container(column![block("Популярные треки".to_string()), tracks].spacing(8)).padding([0, 8]));
        }
        if !page.others.is_empty() {
            let shelf = self.shelf(OTHERS_SHELF, "Другие концерты", None, page.others.len(), |shown| page.others[shown].iter().map(|other| self.concert_card(other)).collect());
            body = body.push(container(shelf).padding([0, 8]));
        }
        ui::scroll(container(body).padding(Padding::ZERO.right(SCROLLBAR))).id(LIST).on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y)).width(Length::Fill).height(Length::Fill).into()
    }

    /// Площадка: название, адрес, станции метро цветом линии и карта со ссылкой на Яндекс Карты.
    fn concert_venue<'a>(&self, page: &api::ConcertPage) -> Element<'a, Message> {
        let venue = &page.venue;
        let mut about = column![text(page.concert.place.clone()).size(18).font(ui::font_medium()).color(ui::c().text)].spacing(6);
        let address = if venue.address.is_empty() { page.concert.city.clone() } else { venue.address.clone() };
        if !address.is_empty() {
            about = about.push(text(address).color(ui::c().muted));
        }
        if !venue.metro.is_empty() {
            let stations = venue.metro.iter().fold(row![].spacing(12), |line, (name, color)| {
                let color = color.as_deref().and_then(hex_color).unwrap_or(ui::c().muted);
                let dot = container(Space::new()).width(10).height(10).style(move |_| container::Style::default().background(color).border(iced::border::rounded(5)));
                line.push(row![dot, text(name.clone()).size(14).color(ui::c().text)].spacing(6).align_y(Alignment::Center))
            });
            about = about.push(stations.wrap().vertical_spacing(6));
        }
        let mut column_ = column![text("Место проведения").size(24).font(ui::font_headline()).color(ui::c().text), about].spacing(8);
        if let Some(handle) = self.covers.get(&format!("{CONCERT_MAP}{}", page.concert.id)) {
            let width = (self.main_width() - 48.0 - SCROLLBAR - 16.0).min(MAP_WIDTH);
            let map = image(handle.clone()).opacity(self.cover_alpha(handle)).content_fit(iced::ContentFit::Cover).width(width).height(MAP_HEIGHT * width / MAP_WIDTH).border_radius(12.0);
            column_ = column_.push(match &venue.map_url {
                Some(url) => Element::from(hint(button(map).padding(0).style(ui::ghost).on_press(Message::OpenUrl(url.clone())), "Открыть в Яндекс Картах")),
                None => map.into(),
            });
        }
        column_.into()
    }
}

/// «#ff4c4c» → цвет.
fn hex_color(hex: &str) -> Option<iced::Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some(iced::Color::from_rgb8(channel(0)?, channel(2)?, channel(4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cities_by_start_then_inside() {
        let cities = vec![(1, "Москва".to_string()), (2, "Нижний Новгород".to_string()), (3, "Великий Новгород".to_string()), (4, "Новосибирск".to_string())];
        let found: Vec<u64> = cities_matching(&cities, "нов").iter().map(|city| city.0).collect();
        assert_eq!(found, vec![4, 2, 3]);
        assert_eq!(cities_matching(&cities, " ").len(), 4);
        assert_eq!(hex_color("#ff4c4c"), Some(iced::Color::from_rgb8(255, 76, 76)));
        assert_eq!(hex_color("red"), None);
    }
}
