//! Ynison в окне: что отправлять о своём плеере и как выполнять команды с других устройств.
//! Устройство, на котором включили музыку, становится играющим; остальные встают на паузу,
//! но видят его очередь и могут им управлять.

use crate::*;
use iced::widget::{column, row};
use ynison::{Local, Playable, Shared};

/// Такой разрыв между ожидаемым и настоящим местом в треке — уже перемотка.
const SEEK_JUMP: Duration = Duration::from_secs(3);
const DEVICES_WIDTH: f32 = 320.0;

impl App {
    /// Подключается к Ynison после входа. Без сети и в предпросмотре не подключаемся.
    pub(crate) fn ynison_start(&mut self) {
        if self.ynison.is_some() || self.offline || (self.preview.is_some() && std::env::var_os("TUNE_YNISON").is_none()) {
            return;
        }
        let Some(token) = self.token() else { return };
        let local = self.ynison_local();
        self.ynison_sent = Some((local.clone(), Instant::now()));
        self.ynison = Some(ynison::Ynison::start(token, local));
    }

    /// Наш плеер в виде состояния Ynison.
    pub(crate) fn ynison_local(&self) -> Local {
        let queue = self
            .queue
            .iter()
            .map(|track| Playable { id: track.id.clone(), album: track.album.clone(), title: track.title.clone(), cover: track.cover.clone() })
            .collect();
        let (entity_type, entity_id) = match &self.queue_from {
            QueueFrom::List(list) if list.source == Source::Album => ("ALBUM", list.kind.clone()),
            QueueFrom::List(list) => ("PLAYLIST", format!("{}:{}", list.owner, list.kind)),
            QueueFrom::Artist(id) | QueueFrom::Familiar(id) => ("ARTIST", id.clone()),
            _ if self.wave_queue => ("RADIO", self.wave_base.first().cloned().unwrap_or_default()),
            _ => ("VARIOUS", String::new()),
        };
        let paused = !self.is_playing();
        let progress = if self.active { self.player.as_ref().map_or(self.position, |p| p.position()) } else { self.position };
        Local {
            queue,
            index: self.current,
            entity_id,
            entity_type: entity_type.to_string(),
            from: "tune-desktop".to_string(),
            paused,
            progress,
            duration: self.current_track().map_or(Duration::ZERO, |track| track.duration),
            speed: if self.current_track().is_some_and(|track| track.remember) { self.speed } else { 1.0 },
        }
    }

    fn ynison_here(&self) -> bool {
        match (&self.ynison, &self.ynison_state.active) {
            (Some(y), Some(active)) => active == y.device_id(),
            _ => false,
        }
    }

    /// Другое устройство играет: его название.
    pub(crate) fn ynison_elsewhere(&self) -> Option<&ynison::Device> {
        let y = self.ynison.as_ref()?;
        let active = self.ynison_state.active.as_deref().filter(|id| *id != y.device_id())?;
        self.ynison_state.devices.iter().find(|device| device.id == active)
    }

    /// На каждом такте: принять пришедшее, отправить своё изменившееся.
    pub(crate) fn ynison_tick(&mut self) -> Task<Message> {
        let Some(states) = self.ynison.as_ref().map(|y| y.take()) else { return Task::none() };
        let mut tasks = Vec::new();
        for state in states {
            tasks.push(self.ynison_apply(state));
        }
        tasks.push(self.ynison_push());
        Task::batch(tasks)
    }

    /// Отправляет наше состояние, если оно заметно изменилось.
    fn ynison_push(&mut self) -> Task<Message> {
        let local = self.ynison_local();
        let here = self.ynison_here();
        let Some(y) = self.ynison.as_ref() else { return Task::none() };
        let Some((sent, at)) = self.ynison_sent.clone() else {
            y.set_full(&local);
            self.ynison_sent = Some((local, Instant::now()));
            return Task::none();
        };
        let same_queue = sent.queue.iter().map(|p| &p.id).eq(local.queue.iter().map(|p| &p.id)) && sent.index == local.index;
        let expected = if sent.paused { sent.progress } else { sent.progress + at.elapsed().mul_f32(sent.speed.max(0.1)) };
        let jumped = local.progress.abs_diff(expected) > SEEK_JUMP;
        if same_queue && sent.paused == local.paused && sent.speed == local.speed && !jumped {
            return Task::none();
        }
        // Включили здесь, пока играло другое устройство или ничего: забираем воспроизведение.
        let started_here = !local.paused && (sent.paused || !same_queue);
        if !here && !started_here {
            y.set_full(&local);
        } else if !same_queue || !here {
            y.send_state(&local, !here);
            if !here {
                if let Some(y) = &self.ynison {
                    self.ynison_state.active = Some(y.device_id().to_string());
                }
            }
        } else {
            y.send_status(&local);
        }
        self.ynison_sent = Some((local, Instant::now()));
        Task::none()
    }

    /// Состояние от узла: другие устройства, кто играет и что.
    fn ynison_apply(&mut self, state: Shared) -> Task<Message> {
        let Some(me) = self.ynison.as_ref().map(|y| y.device_id().to_string()) else { return Task::none() };
        let was = self.ynison_state.active.clone();
        self.ynison_state = state.clone();
        let here = state.active.as_deref() == Some(me.as_str());
        if !here {
            // Включили на другом устройстве: здесь пауза, как у официального клиента.
            if state.active.is_some() && state.active != was && self.is_playing() {
                self.ynison_pause();
            }
            return Task::none();
        }
        // Своё же изменение вернулось эхом.
        if state.status_by == me && state.queue_by == me {
            return Task::none();
        }
        let local = self.ynison_local();
        let same_list = state.queue.iter().map(|p| &p.id).eq(local.queue.iter().map(|p| &p.id));
        if !same_list {
            if state.queue.is_empty() {
                return Task::none();
            }
            let Some(token) = self.token() else { return Task::none() };
            let ids: Vec<String> = state.queue.iter().map(|p| p.id.clone()).collect();
            return Task::perform(background(move || api::tracks_by_ids(&ids, &token)), move |result| Message::YnisonQueue(state.clone(), result));
        }
        let mut task = Task::none();
        if let Some(index) = state.index.filter(|index| Some(*index) != local.index) {
            task = self.ynison_take(index, &state);
        } else {
            if state.paused != local.paused {
                if state.paused {
                    self.ynison_pause();
                } else {
                    task = self.update(Message::Toggle);
                }
            }
            if state.progress.abs_diff(local.progress) > SEEK_JUMP && self.current.is_some() {
                self.seeking = Some(state.progress.as_secs_f32());
                task = Task::batch([task, self.update(Message::SeekDone)]);
            }
        }
        // То, что сделали по чужой команде, обратно не отправляем.
        self.ynison_sent = Some((self.ynison_local(), Instant::now()));
        task
    }

    /// Трек очереди с места и паузы из общего состояния.
    fn ynison_take(&mut self, index: usize, state: &Shared) -> Task<Message> {
        self.current = Some(index);
        self.position = state.progress;
        self.resume = Some(state.progress);
        if state.paused {
            if let Some(p) = &self.player {
                p.stop();
            }
            self.active = false;
            return Task::none();
        }
        self.play(index)
    }

    fn ynison_pause(&mut self) {
        if let Some(p) = self.player.clone().filter(|p| !p.is_paused() && self.active) {
            p.toggle();
            self.progress_save(false);
        }
        self.ynison_sent = Some((self.ynison_local(), Instant::now()));
    }

    pub(crate) fn ynison_handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::YnisonQueue(state, Ok(tracks)) => {
                // Ответ пришёл, когда играет уже другое: не перебиваем.
                if self.ynison_state.active != state.active {
                    return Task::none();
                }
                let index = state.index.and_then(|at| state.queue.get(at)).and_then(|p| tracks.iter().position(|t| t.id == p.id)).unwrap_or(0);
                self.queue = tracks;
                self.queue_title = Some("С другого устройства".to_string());
                self.queue_from = QueueFrom::Unknown;
                self.wave = None;
                self.wave_queue = false;
                self.unshuffled = None;
                let task = self.ynison_take(index, &state);
                self.ynison_sent = Some((self.ynison_local(), Instant::now()));
                return task;
            }
            Message::YnisonQueue(_, Err(e)) => return self.fail(e),
            Message::DevicesOpen => {
                // В предпросмотре мыши нет: список встаёт там, где кнопка в широком окне.
                let x = if matches!(self.preview, Some(Preview::Devices)) { WINDOW.lock().unwrap().0 - 250.0 } else { CURSOR.lock().unwrap().0 };
                self.devices_menu = Some(x);
            }
            Message::DeviceSelect(id) => {
                self.devices_menu = None;
                let Some(y) = &self.ynison else { return Task::none() };
                if id == y.device_id() {
                    // Сюда: продолжаем чужую очередь с того же места.
                    let state = self.ynison_state.clone();
                    let same = state.queue.iter().map(|p| &p.id).eq(self.queue.iter().map(|t| &t.id));
                    if state.queue.is_empty() || same {
                        let index = state.index.or(self.current).unwrap_or(0);
                        let mut state = state;
                        state.paused = false;
                        return self.ynison_take(index, &state);
                    }
                    let Some(token) = self.token() else { return Task::none() };
                    let ids: Vec<String> = state.queue.iter().map(|p| p.id.clone()).collect();
                    let mut state = state;
                    state.paused = false;
                    state.active = Some(y.device_id().to_string());
                    self.ynison_state.active = state.active.clone();
                    return Task::perform(background(move || api::tracks_by_ids(&ids, &token)), move |result| Message::YnisonQueue(state.clone(), result));
                }
                // Туда: сначала наше состояние, чтобы устройство продолжило с того же места.
                let local = self.ynison_local();
                y.send_state(&local, false);
                y.send_active(Some(&id));
                self.ynison_state.active = Some(id);
                self.ynison_pause();
            }
            _ => {}
        }
        Task::none()
    }

    /// Кнопка устройств в трекбаре: видна, когда кроме Tune есть куда передать музыку.
    pub(crate) fn devices_tool<'a>(&self, size: f32) -> Option<Element<'a, Message>> {
        // Кнопка есть всегда, как в оригинале: без связи или без других устройств список это и скажет.
        self.ynison.as_ref()?;
        let elsewhere = self.ynison_elsewhere();
        let label = if elsewhere.is_some() { "Играет на другом устройстве" } else { "Устройства" };
        let color = if elsewhere.is_some() { ui::c().accent } else { ui::c().muted };
        Some(hint(glow(button(ui::icon(ui::DEVICES, size, color)).padding(4).style(ui::ghost).on_press(Message::DevicesOpen), 20.0), label))
    }

    /// Окно со списком устройств над трекбаром.
    pub(crate) fn view_devices(&self, x: f32) -> Option<Element<'_, Message>> {
        let y = self.ynison.as_ref()?;
        let me = y.device_id();
        let active = self.ynison_state.active.as_deref();
        let mut list = column![text("Устройства").size(16).font(ui::font_medium()).color(ui::c().text)].spacing(4);
        let connected = y.connected();
        let mut devices: Vec<ynison::Device> = if connected { self.ynison_state.devices.iter().filter(|device| device.id != me && device.can_play).cloned().collect() } else { Vec::new() };
        let note = match (connected, devices.is_empty()) {
            (false, _) => Some("Нет связи с Ynison: другие устройства не видны. Tune подключается заново сам"),
            (true, true) => Some("Других устройств нет. Откройте Яндекс Музыку на телефоне или колонке — они появятся здесь"),
            _ => None,
        };
        devices.insert(0, ynison::Device { id: me.to_string(), title: "Это устройство".to_string(), kind: "WEB_DESKTOP".to_string(), can_play: true });
        for device in devices {
            let playing = active == Some(device.id.as_str());
            let tint = if playing { ui::c().accent } else { ui::c().text };
            let mut words = column![text(device.title.clone()).size(14).color(tint)];
            let kind = if playing { "Играет сейчас".to_string() } else { device_kind(&device.kind).to_string() };
            if !kind.is_empty() {
                words = words.push(text(kind).size(12).color(ui::c().muted));
            }
            let line = row![ui::icon(ui::DEVICES, 22.0, tint), words].spacing(10).align_y(Alignment::Center);
            list = list.push(glow(button(line).width(Length::Fill).padding([8, 8]).style(ui::row).on_press(Message::DeviceSelect(device.id.clone())), 8.0));
        }
        if let Some(note) = note {
            list = list.push(container(text(note).size(13).color(ui::c().muted)).padding([4, 8]));
        }
        let bottom = self.bar_top_y() - 12.0;
        Some(popup_above(list.into(), x - DEVICES_WIDTH / 2.0, bottom, DEVICES_WIDTH, 1.0, true))
    }
}

fn device_kind(kind: &str) -> &'static str {
    match kind {
        "ANDROID" => "Телефон на Android",
        "IOS" => "iPhone или iPad",
        "WEB" => "Браузер",
        "WEB_DESKTOP" => "Компьютер",
        "SMART_SPEAKER" => "Умная колонка",
        "ANDROID_TV" | "APPLE_TV" | "WEB_TV" => "Телевизор",
        "ANDROID_AUTO" | "CARPLAY" => "Автомобиль",
        "ANDROID_WEAR" => "Часы",
        _ => "",
    }
}

