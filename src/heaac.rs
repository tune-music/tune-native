//! HE-AAC («Экономичное» качество): MP4 разбирает symphonia, а кадры декодирует AAC-декодер
//! Windows (Media Foundation). Сам symphonia понимает только основу AAC без SBR: у такого
//! файла звучала бы лишь половина полосы, до 11 кГц.

use std::io::{Read, Seek};
use std::time::Duration;

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::units::Time;
use symphonia::default::formats::IsoMp4Reader;
use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

/// AAC-декодер Windows (`CLSID_CMSAACDecMFT`).
const AAC_DECODER: GUID = GUID::from_u128(0x32d186a7_218f_4c75_8876_dd77273a8999);

/// Файл, который читает symphonia: тот же временный файл, что докачивается в фоне.
struct Input<R>(R, u64);

impl<R: Read> Read for Input<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<R: Seek> Seek for Input<R> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for Input<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.1)
    }
}

pub struct HeAac {
    reader: IsoMp4Reader,
    track: u32,
    decoder: IMFTransform,
    /// Размер выходного буфера, который просит декодер.
    out_size: u32,
    channels: ChannelCount,
    rate: SampleRate,
    duration: Option<Duration>,
    out: Vec<f32>,
    out_at: usize,
    ended: bool,
}

// Декодер создан в многопоточном COM и дальше живёт только в потоке звука.
unsafe impl Send for HeAac {}

fn fail(e: impl std::fmt::Display) -> String {
    format!("Не удалось прочитать файл трека: {e}")
}

impl HeAac {
    pub fn new<R: Read + Seek + Send + Sync + 'static>(data: R, len: u64) -> Result<Self, String> {
        let stream = MediaSourceStream::new(Box::new(Input(data, len)), Default::default());
        let reader = IsoMp4Reader::try_new(stream, &FormatOptions::default()).map_err(fail)?;
        let track = reader.default_track().ok_or_else(|| fail("в файле нет звука"))?;
        let params = track.codec_params.clone();
        let config = params.extra_data.clone().ok_or_else(|| fail("нет настроек AAC"))?;
        let core_rate = params.sample_rate.ok_or_else(|| fail("нет частоты"))?;
        let channels = params.channels.map_or(2, |c| c.count() as u32);
        let duration = params.n_frames.zip(params.time_base).map(|(frames, base)| {
            let time = base.calc_time(frames);
            Duration::from_secs(time.seconds) + Duration::from_secs_f64(time.frac)
        });
        let id = track.id;
        let (decoder, out_size, rate, out_channels) = unsafe { open_decoder(&config, core_rate, channels) }.map_err(fail)?;
        Ok(Self {
            reader,
            track: id,
            decoder,
            out_size,
            channels: ChannelCount::new(out_channels as u16).ok_or_else(|| fail("нет каналов"))?,
            rate: SampleRate::new(rate).ok_or_else(|| fail("нет частоты"))?,
            duration,
            out: Vec::new(),
            out_at: 0,
            ended: false,
        })
    }

    /// Отдаёт декодеру кадры, пока он не вернёт звук. `false` — файл кончился.
    fn refill(&mut self) -> bool {
        self.out.clear();
        self.out_at = 0;
        loop {
            match unsafe { self.drain() } {
                Ok(true) => return true,
                Ok(false) => {}
                Err(_) => return false,
            }
            if self.ended {
                return false;
            }
            let packet = loop {
                match self.reader.next_packet() {
                    Ok(packet) if packet.track_id() == self.track => break Some(packet),
                    Ok(_) => continue,
                    Err(_) => break None,
                }
            };
            let Some(packet) = packet else {
                self.ended = true;
                let _ = unsafe { self.decoder.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) };
                continue;
            };
            if unsafe { feed(&self.decoder, &packet.data) }.is_err() {
                return false;
            }
        }
    }

    /// Забирает готовый звук из декодера. `Ok(false)` — ему нужны ещё кадры.
    unsafe fn drain(&mut self) -> windows::core::Result<bool> {
        unsafe {
            let buffer = MFCreateMemoryBuffer(self.out_size)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            let mut output = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(Some(sample)), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
            let mut status = 0;
            let result = self.decoder.ProcessOutput(0, &mut output, &mut status);
            let sample = std::mem::ManuallyDrop::take(&mut output[0].pSample);
            drop(std::mem::ManuallyDrop::take(&mut output[0].pEvents));
            match result {
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    set_output(&self.decoder)?;
                    return Ok(false);
                }
                Err(e) => return Err(e),
                Ok(()) => {}
            }
            let Some(sample) = sample else { return Ok(false) };
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let floats = std::slice::from_raw_parts(data as *const f32, length as usize / 4);
            self.out.extend_from_slice(floats);
            buffer.Unlock()?;
            Ok(!self.out.is_empty())
        }
    }
}

/// Открывает декодер на сырые кадры AAC с настройками из MP4 и просит на выходе float.
/// Вернёт декодер, размер выходного буфера, частоту и число каналов звука.
unsafe fn open_decoder(config: &[u8], core_rate: u32, channels: u32) -> windows::core::Result<(IMFTransform, u32, u32, u32)> {
    unsafe {
        // Поток мог уже войти в COM с другими настройками: тогда работаем в них.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let decoder: IMFTransform = CoCreateInstance(&AAC_DECODER, None, CLSCTX_INPROC_SERVER)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, core_rate)?;
        input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        input.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
        input.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, 0xfe)?;
        // Хвост HEAACWAVEINFO после WAVEFORMATEX: тип кадров (сырые), профиль, тип
        // структуры и резерв — 12 байт, за ними AudioSpecificConfig из MP4.
        let mut user = vec![0u8; 12];
        user[2..4].copy_from_slice(&0xfeu16.to_le_bytes());
        user.extend_from_slice(config);
        input.SetBlob(&MF_MT_USER_DATA, &user)?;
        decoder.SetInputType(0, &input, 0)?;
        let (rate, out_channels) = set_output(&decoder)?;
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        decoder.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        let info = decoder.GetOutputStreamInfo(0)?;
        // Кадр HE-AAC — 2048 отсчётов на канал; с запасом, если декодер не назвал размер.
        let size = info.cbSize.max(2048 * 8 * out_channels);
        Ok((decoder, size, rate, out_channels))
    }
}

/// Выбирает выход float и возвращает его частоту и число каналов.
unsafe fn set_output(decoder: &IMFTransform) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let mut index = 0;
        loop {
            let kind = decoder.GetOutputAvailableType(0, index)?;
            if kind.GetGUID(&MF_MT_SUBTYPE)? == MFAudioFormat_Float {
                decoder.SetOutputType(0, &kind, 0)?;
                return Ok((kind.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?, kind.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?));
            }
            index += 1;
        }
    }
}

unsafe fn feed(decoder: &IMFTransform, data: &[u8]) -> windows::core::Result<()> {
    unsafe {
        let buffer = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut ptr = std::ptr::null_mut();
        buffer.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(data.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        decoder.ProcessInput(0, &sample, 0)
    }
}

impl Iterator for HeAac {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.out_at >= self.out.len() && !self.refill() {
            return None;
        }
        self.out_at += 1;
        Some(self.out[self.out_at - 1])
    }
}

impl Source for HeAac {
    fn current_span_len(&self) -> Option<usize> {
        let pending = self.out.len() - self.out_at;
        Some(if pending > 0 { pending } else { 2048 * self.channels.get() as usize })
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let time = Time::new(pos.as_secs(), pos.subsec_nanos() as f64 / 1e9);
        self.reader
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track) })
            .map_err(|e| SeekError::Other(std::sync::Arc::new(std::io::Error::other(e.to_string()))))?;
        unsafe {
            let _ = self.decoder.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
        self.out.clear();
        self.out_at = 0;
        self.ended = false;
        Ok(())
    }
}
