//! Audio — odtwarzanie dźwięków i muzyki (opakowanie [rodio]).
//!
//! API jest celowo małe: tworzysz [`Audio`], wczytujesz [`Clip`] (bufor próbek),
//! odtwarzasz go raz albo w pętli i dostajesz [`Sound`] — kontrolkę głośności
//! i pauzy. Bez urządzenia audio (serwer CI) [`Audio::new`] zwraca błąd, a gra
//! działa dalej.
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! let audio = Audio::new().ok();               // Option<Audio>
//! let skok = Clip::from_file("assets/sfx/skok.ogg").ok();
//!
//! // w systemie:
//! if let (Some(a), Some(s)) = (&audio, &skok) {
//!     if ctx.input.just_pressed(Key::Space) {
//!         if let Ok(sound) = a.play_clip(s) {
//!             sound.set_volume(0.8);
//!         }
//!     }
//! }
//! ```

use std::cell::Cell;
use std::io::BufReader;
use std::path::Path;

use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use uran_math::Vec2;

/// Połowa odległości między uszami słuchacza (jednostki świata).
///
/// Większa wartość = mocniejszy efekt stereo (dźwięk z boku mocniej „panuje"),
/// mniejsza = bardziej mono.
const EAR_SPACING: f32 = 0.5;

/// Błąd audio — nie wywraca gry, tylko sygnalizuje brak dźwięku.
#[derive(Debug)]
pub enum AudioError {
    /// Nie udało się otworzyć urządzenia wyjściowego (np. bez karty dźwiękowej).
    NoDevice,
    /// Pliku nie da się zdekodować (nieobsługiwany format / uszkodzony).
    Decode(String),
    /// Błąd wejścia/wyjścia (brak pliku, brak uprawnień).
    Io(std::io::Error),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "brak urządzenia audio"),
            AudioError::Decode(m) => write!(f, "nie udało się zdekodować dźwięku: {m}"),
            AudioError::Io(e) => write!(f, "błąd pliku dźwięku: {e}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Silnik audio — trzyma urządzenie wyjściowe i odtwarza dźwięki.
///
/// Utwórz raz przy starcie i złap w closure systemu. Trzymaj [`Sound`]
/// zwrócony przez `play_*`, dopóki dźwięk ma grać — porzucenie go zatrzymuje
/// odtwarzanie.
pub struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    listener: Cell<Vec2>,
}

/// Wczytany dźwięk (bufor próbek) — tani do odtwarzania wielokrotnie.
#[derive(Clone)]
pub struct Clip {
    channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
}

/// Kontrolka odtwarzanego dźwięku: głośność, pauza, stop.
pub struct Sound {
    sink: Sink,
}

impl Audio {
    /// Otwiera domyślne urządzenie wyjściowe. Zwraca błąd, gdy go nie ma.
    pub fn new() -> Result<Self, AudioError> {
        let (stream, handle) =
            rodio::OutputStream::try_default().map_err(|_| AudioError::NoDevice)?;
        Ok(Self {
            _stream: stream,
            handle,
            listener: Cell::new(Vec2::ZERO),
        })
    }

    /// Odtwarza plik dźwiękowy raz (nieblokująco).
    pub fn play_file(&self, path: impl AsRef<Path>) -> Result<Sound, AudioError> {
        let source = decode_file(path)?.convert_samples::<f32>();
        self.play_source(source)
    }

    /// Odtwarza plik dźwiękowy w pętli.
    pub fn loop_file(&self, path: impl AsRef<Path>) -> Result<Sound, AudioError> {
        let source = decode_file(path)?.repeat_infinite().convert_samples::<f32>();
        self.play_source(source)
    }

    /// Odtwarza wczytany [`Clip`] raz.
    pub fn play_clip(&self, clip: &Clip) -> Result<Sound, AudioError> {
        self.play_source(clip.buffer())
    }

    /// Odtwarza wczytany [`Clip`] w pętli.
    pub fn loop_clip(&self, clip: &Clip) -> Result<Sound, AudioError> {
        self.play_source(clip.buffer().repeat_infinite())
    }

    /// Ustawia pozycję słuchacza dla dźwięku przestrzennego.
    ///
    /// Słuchacz „patrzy" w kierunku +X; dźwięk po lewej (w osi +Y) panuje na
    /// lewe ucho, po prawej (w osi -Y) na prawe, a głośność spada z odległością.
    pub fn set_listener_position(&self, position: Vec2) {
        self.listener.set(position);
    }

    /// Odtwarza klip **przestrzennie**: głośność i stereo zależą od pozycji
    /// względem słuchacza ([`Audio::set_listener_position`]).
    pub fn play_spatial(&self, clip: &Clip, position: Vec2) -> Result<SpatialSound, AudioError> {
        self.play_spatial_source(clip.buffer(), position)
    }

    /// Jak [`Audio::play_spatial`], ale w pętli (np. rzeka, ogień, silnik).
    pub fn loop_spatial(&self, clip: &Clip, position: Vec2) -> Result<SpatialSound, AudioError> {
        self.play_spatial_source(clip.buffer().repeat_infinite(), position)
    }

    fn play_spatial_source(
        &self,
        source: impl Source<Item = f32> + Send + 'static,
        position: Vec2,
    ) -> Result<SpatialSound, AudioError> {
        let (left, right) = self.ears();
        let sink = rodio::SpatialSink::try_new(
            &self.handle,
            [position.x, position.y, 0.0],
            left,
            right,
        )
        .map_err(|_| AudioError::NoDevice)?;
        sink.append(source);
        Ok(SpatialSound { sink })
    }

    fn ears(&self) -> ([f32; 3], [f32; 3]) {
        let p = self.listener.get();
        (
            [p.x, p.y + EAR_SPACING, 0.0], // lewe ucho (+Y)
            [p.x, p.y - EAR_SPACING, 0.0], // prawe ucho (-Y)
        )
    }

    fn play_source(&self, source: impl Source<Item = f32> + Send + 'static) -> Result<Sound, AudioError> {
        let sink = rodio::Sink::try_new(&self.handle).map_err(|_| AudioError::NoDevice)?;
        sink.append(source);
        Ok(Sound { sink })
    }
}

impl Clip {
    /// Wczytuje i dekoduje plik dźwiękowy do bufora próbek.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let file = std::fs::File::open(path).map_err(AudioError::Io)?;
        let decoder = rodio::Decoder::new(BufReader::new(file))
            .map_err(|e| AudioError::Decode(e.to_string()))?;
        Ok(Self::from_decoder(decoder))
    }

    /// Dekoduje dźwięk z pamięci (bajty pliku).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AudioError> {
        let decoder = rodio::Decoder::new(std::io::Cursor::new(bytes.to_vec()))
            .map_err(|e| AudioError::Decode(e.to_string()))?;
        Ok(Self::from_decoder(decoder))
    }

    fn from_decoder<R>(decoder: rodio::Decoder<R>) -> Self
    where
        R: std::io::Read + std::io::Seek + Send + 'static,
    {
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
        Self {
            channels,
            sample_rate,
            samples,
        }
    }

    fn buffer(&self) -> rodio::buffer::SamplesBuffer<f32> {
        rodio::buffer::SamplesBuffer::new(self.channels, self.sample_rate, self.samples.clone())
    }
}

impl Sound {
    /// Ustawia głośność (0.0..=1.0).
    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    /// Wstrzymuje odtwarzanie (wznowisz przez [`Sound::resume`]).
    pub fn pause(&self) {
        self.sink.pause();
    }

    /// Wznawia odtwarzanie po pauzie.
    pub fn resume(&self) {
        self.sink.play();
    }

    /// Zatrzymuje i resetuje dźwięk.
    pub fn stop(&self) {
        self.sink.stop();
    }

    /// Czy dźwięk skończył grać (true = można odrzucić kontrolkę).
    pub fn is_finished(&self) -> bool {
        self.sink.len() == 0
    }
}

/// Kontrolka dźwięku **przestrzennego**: pozycja emitera, głośność, pauza.
///
/// Trzymaj ją, dopóki dźwięk ma grać; porzucenie zatrzymuje odtwarzanie.
pub struct SpatialSound {
    sink: rodio::SpatialSink,
}

impl SpatialSound {
    /// Przesuwa źródło dźwięku w świat — efekt stereo i głośność zaktualizują się.
    pub fn set_position(&self, position: Vec2) {
        self.sink.set_emitter_position([position.x, position.y, 0.0]);
    }

    /// Ustawia głośność (0.0..=1.0).
    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    /// Wstrzymuje odtwarzanie.
    pub fn pause(&self) {
        self.sink.pause();
    }

    /// Wznawia odtwarzanie po pauzie.
    pub fn resume(&self) {
        self.sink.play();
    }

    /// Zatrzymuje i resetuje dźwięk.
    pub fn stop(&self) {
        self.sink.stop();
    }

    /// Czy dźwięk skończył grać.
    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }
}

fn decode_file(path: impl AsRef<Path>) -> Result<rodio::Decoder<BufReader<std::fs::File>>, AudioError> {
    let file = std::fs::File::open(path).map_err(AudioError::Io)?;
    rodio::Decoder::new(BufReader::new(file)).map_err(|e| AudioError::Decode(e.to_string()))
}

