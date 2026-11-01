//! Системная медиапанель Windows и медиаклавиши: приходят сюда без глобального перехвата
//! клавиатуры. Объекты панели привязаны к потоку окна, поэтому живут в состоянии приложения.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, Stream, StreamExt};
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use crate::api::Track;

/// Расхождение со временем панели, после которого считаем, что трек перемотали.
const POSITION_DRIFT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub enum Event {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(Duration),
}

type Channel = (UnboundedSender<Event>, Mutex<Option<UnboundedReceiver<Event>>>);
static EVENTS: OnceLock<Channel> = OnceLock::new();

fn channel() -> &'static Channel {
    EVENTS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// События панели для подписки приложения. Читать их можно только один раз.
pub fn events() -> impl Stream<Item = Event> {
    match channel().1.lock().unwrap().take() {
        Some(rx) => rx.left_stream(),
        None => stream::pending().right_stream(),
    }
}

fn convert(event: MediaControlEvent) -> Option<Event> {
    Some(match event {
        MediaControlEvent::Play => Event::Play,
        MediaControlEvent::Pause | MediaControlEvent::Stop => Event::Pause,
        MediaControlEvent::Toggle => Event::Toggle,
        MediaControlEvent::Next => Event::Next,
        MediaControlEvent::Previous => Event::Previous,
        MediaControlEvent::SetPosition(MediaPosition(position)) => Event::Seek(position),
        _ => return None,
    })
}

pub struct Media {
    hwnd: isize,
    controls: Option<MediaControls>,
    track: Option<String>,
    playing: bool,
    position: Duration,
    sent: Instant,
}

impl Media {
    pub fn new(hwnd: isize) -> Self {
        Self { hwnd, controls: None, track: None, playing: false, position: Duration::ZERO, sent: Instant::now() }
    }

    fn create(&self) -> Option<MediaControls> {
        let mut controls = MediaControls::new(PlatformConfig { display_name: "Tune", dbus_name: "tune_native", hwnd: Some(self.hwnd as *mut c_void) }).ok()?;
        let tx = channel().0.clone();
        controls
            .attach(move |event| {
                if let Some(event) = convert(event) {
                    let _ = tx.unbounded_send(event);
                }
            })
            .ok()?;
        Some(controls)
    }

    /// Сообщает панели только об изменениях: зовётся после каждого сообщения приложения.
    pub fn sync(&mut self, track: Option<&Track>, playing: bool, position: Duration) {
        // Нет трека — сессии в панели нет вовсе: пустая показывала бы имя процесса.
        let Some(track) = track else {
            self.controls = None;
            self.track = None;
            return;
        };
        if self.controls.is_none() {
            self.controls = self.create();
            self.track = None;
        }
        let Some(controls) = self.controls.as_mut() else { return };
        let changed = self.track.as_deref() != Some(track.id.as_str());
        if changed {
            let cover = track.cover.as_ref().map(|uri| format!("https://{}", uri.replace("%%", "400x400")));
            let _ = controls.set_metadata(MediaMetadata {
                title: Some(&track.title),
                artist: Some(&track.artists),
                album: None,
                cover_url: cover.as_deref(),
                duration: Some(track.duration),
            });
            self.track = Some(track.id.clone());
        }
        let expected = if self.playing { self.position + self.sent.elapsed() } else { self.position };
        let drift = expected.abs_diff(position);
        if changed || playing != self.playing || drift > POSITION_DRIFT {
            let progress = Some(MediaPosition(position));
            let _ = controls.set_playback(if playing { MediaPlayback::Playing { progress } } else { MediaPlayback::Paused { progress } });
            self.playing = playing;
            self.position = position;
            self.sent = Instant::now();
        }
    }
}
