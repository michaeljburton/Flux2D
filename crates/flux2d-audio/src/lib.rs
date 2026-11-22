//! Flux2D audio: sounds and music on mixer buses, built on [kira](https://docs.rs/kira).
//!
//! There is one [`Audio`] per game, with a bus for each kind of sound ([`Bus::Music`],
//! [`Bus::Sfx`], [`Bus::Voice`]) under a master volume, like Cosmic Commando's Godot audio
//! buses. Sounds are loaded once into memory ([`Sound`], cheap to clone) and played many
//! times; music streams from disk.
//!
//! If the machine has no audio device, [`Audio::new`] still succeeds: it logs a warning and
//! every call does nothing, so a game (or a CI run) works without sound.

use kira::backend::Backend;
use kira::sound::FromFileError;
use kira::sound::static_sound::StaticSoundData;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning, PlaybackRate, Tween,
};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use kira;

/// A mixer bus. Each has its own volume, under the master volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    Music,
    Sfx,
    Voice,
}

impl Bus {
    pub const ALL: [Bus; 3] = [Bus::Music, Bus::Sfx, Bus::Voice];

    fn index(self) -> usize {
        self as usize
    }
}

/// Sounds one bus can play at once; more are dropped (a burst of 500 laser shots doesn't need
/// 500 voices).
const VOICES_PER_BUS: usize = 128;

/// A sound held in memory. Cloning shares the samples.
#[derive(Clone)]
pub struct Sound(StaticSoundData);

impl Sound {
    /// Decodes a WAV, OGG Vorbis, MP3 or FLAC file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let path = path.as_ref();
        StaticSoundData::from_file(path)
            .map(Sound)
            .map_err(|e| AudioError::Load(format!("{}: {e}", path.display())))
    }

    /// A mono sound from samples in -1..1, for sounds made in code.
    pub fn from_samples(sample_rate: u32, samples: &[f32]) -> Self {
        let frames: Arc<[Frame]> = samples.iter().map(|&s| Frame::from_mono(s)).collect();
        Sound(StaticSoundData { sample_rate, frames, settings: Default::default(), slice: None })
    }

    /// Length in seconds.
    pub fn duration(&self) -> f32 {
        self.0.duration().as_secs_f32()
    }
}

/// How to play one sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayParams {
    /// Linear volume, 1 = as recorded.
    pub volume: f32,
    /// Playback speed (and pitch), 1 = as recorded.
    pub pitch: f32,
    /// -1 = left only, 0 = centre, 1 = right only.
    pub pan: f32,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self { volume: 1.0, pitch: 1.0, pan: 0.0 }
    }
}

#[derive(Debug)]
pub enum AudioError {
    Load(String),
    Device(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Load(e) => write!(f, "could not load sound: {e}"),
            AudioError::Device(e) => write!(f, "could not open the audio device: {e}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Linear volume (0..) to decibels, with 0 as silence.
pub fn to_decibels(volume: f32) -> Decibels {
    if volume <= 0.001 { Decibels::SILENCE } else { Decibels(20.0 * volume.log10()) }
}

struct Mixer<B: Backend> {
    manager: AudioManager<B>,
    buses: [TrackHandle; 3],
    music: Option<StreamingSoundHandle<FromFileError>>,
}

/// The game's audio: a master volume and three buses.
pub struct Audio<B: Backend = DefaultBackend> {
    mixer: Option<Mixer<B>>,
    volumes: [f32; 3],
    master: f32,
    /// Sounds dropped because a bus was full.
    dropped: u64,
}

impl Audio<DefaultBackend> {
    /// Opens the default audio device. Without one, returns a silent `Audio` (see
    /// [`is_enabled`](Self::is_enabled)).
    pub fn new() -> Self {
        Self::with_settings(AudioManagerSettings::default()).unwrap_or_else(|e| {
            log::warn!("{e}; running without sound");
            Self::silent()
        })
    }
}

impl Default for Audio<DefaultBackend> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: Backend> Audio<B> {
    /// Audio on a specific kira backend (tests use kira's mock backend).
    pub fn with_settings(settings: AudioManagerSettings<B>) -> Result<Self, AudioError>
    where
        B::Error: fmt::Debug,
    {
        let mut manager =
            AudioManager::<B>::new(settings).map_err(|e| AudioError::Device(format!("{e:?}")))?;
        let mut bus = || {
            manager
                .add_sub_track(TrackBuilder::new().sound_capacity(VOICES_PER_BUS))
                .map_err(|e| AudioError::Device(e.to_string()))
        };
        let buses = [bus()?, bus()?, bus()?];
        Ok(Self {
            mixer: Some(Mixer { manager, buses, music: None }),
            volumes: [1.0; 3],
            master: 1.0,
            dropped: 0,
        })
    }

    /// An `Audio` that plays nothing.
    pub fn silent() -> Self {
        Self { mixer: None, volumes: [1.0; 3], master: 1.0, dropped: 0 }
    }

    /// Whether there is an audio device behind this.
    pub fn is_enabled(&self) -> bool {
        self.mixer.is_some()
    }

    /// Sets the master volume (linear, 1 = full).
    pub fn set_master_volume(&mut self, volume: f32) {
        self.master = volume.max(0.0);
        if let Some(m) = &mut self.mixer {
            m.manager.main_track().set_volume(to_decibels(self.master), Tween::default());
        }
    }

    pub fn master_volume(&self) -> f32 {
        self.master
    }

    /// Sets one bus's volume (linear, 1 = full).
    pub fn set_volume(&mut self, bus: Bus, volume: f32) {
        self.volumes[bus.index()] = volume.max(0.0);
        if let Some(m) = &mut self.mixer {
            m.buses[bus.index()].set_volume(to_decibels(volume), Tween::default());
        }
    }

    pub fn volume(&self, bus: Bus) -> f32 {
        self.volumes[bus.index()]
    }

    /// Plays `sound` once on `bus`.
    pub fn play(&mut self, sound: &Sound, bus: Bus) {
        self.play_with(sound, bus, PlayParams::default());
    }

    /// Plays `sound` once on `bus` with a volume, pitch and pan.
    pub fn play_with(&mut self, sound: &Sound, bus: Bus, params: PlayParams) {
        let Some(m) = &mut self.mixer else { return };
        let data = sound
            .0
            .volume(to_decibels(params.volume))
            .playback_rate(PlaybackRate(f64::from(params.pitch.max(0.01))))
            .panning(Panning(params.pan.clamp(-1.0, 1.0)));
        if m.buses[bus.index()].play(data).is_err() {
            self.dropped += 1;
        }
    }

    /// Streams a music file from disk on [`Bus::Music`], looping, and stops the previous track.
    /// `fade` is the cross-fade time in seconds.
    pub fn play_music(&mut self, path: impl AsRef<Path>, fade: f32) -> Result<(), AudioError> {
        let Some(m) = &mut self.mixer else { return Ok(()) };
        let path = path.as_ref();
        let tween = Tween { duration: Duration::from_secs_f32(fade.max(0.0)), ..Default::default() };
        let data = StreamingSoundData::from_file(path)
            .map_err(|e| AudioError::Load(format!("{}: {e}", path.display())))?
            .loop_region(..)
            .fade_in_tween(tween);
        if let Some(mut old) = m.music.take() {
            old.stop(tween);
        }
        let handle =
            m.buses[Bus::Music.index()].play(data).map_err(|e| AudioError::Device(format!("{e:?}")))?;
        m.music = Some(handle);
        Ok(())
    }

    /// Fades out and stops the music.
    pub fn stop_music(&mut self, fade: f32) {
        if let Some(m) = &mut self.mixer
            && let Some(mut music) = m.music.take()
        {
            music.stop(Tween { duration: Duration::from_secs_f32(fade.max(0.0)), ..Default::default() });
        }
    }

    /// Sounds playing on `bus` right now.
    pub fn playing(&self, bus: Bus) -> usize {
        self.mixer.as_ref().map_or(0, |m| m.buses[bus.index()].num_sounds())
    }

    /// Sounds dropped so far because their bus already had [`VOICES_PER_BUS`] playing.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The kira manager, for anything this wrapper doesn't cover (effects, clocks...).
    pub fn manager(&mut self) -> Option<&mut AudioManager<B>> {
        self.mixer.as_mut().map(|m| &mut m.manager)
    }
}

/// Simple sounds made in code, so examples and tests need no audio files.
pub mod synth {
    use super::Sound;

    pub const SAMPLE_RATE: u32 = 44_100;

    /// Small deterministic noise source.
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
    }

    /// A falling "pew": a square-ish tone sweeping from `start_hz` to `end_hz`.
    pub fn laser(start_hz: f32, end_hz: f32, seconds: f32) -> Sound {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        let mut phase = 0.0f32;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let hz = start_hz + (end_hz - start_hz) * t;
                phase = (phase + hz / SAMPLE_RATE as f32).fract();
                let wave = (phase * std::f32::consts::TAU).sin().signum() * 0.5
                    + (phase * std::f32::consts::TAU).sin() * 0.5;
                wave * (1.0 - t).powi(2) * 0.35
            })
            .collect();
        Sound::from_samples(SAMPLE_RATE, &samples)
    }

    /// A noisy boom with a low rumble, `seconds` long.
    pub fn explosion(seconds: f32, seed: u32) -> Sound {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        let mut noise = Noise(seed.max(1));
        let mut low = 0.0f32;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                // A one-pole low-pass that closes over time: bright crack, then rumble.
                let k = 0.5 * (1.0 - t).powi(2) + 0.02;
                low += (noise.next() - low) * k;
                let thump =
                    (i as f32 / SAMPLE_RATE as f32 * 55.0 * std::f32::consts::TAU).sin() * (1.0 - t).powi(4);
                (low * 1.6 + thump * 0.5) * (1.0 - t).powf(1.5) * 0.6
            })
            .collect();
        Sound::from_samples(SAMPLE_RATE, &samples)
    }

    /// A short metallic tick for hits.
    pub fn hit(seconds: f32) -> Sound {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        let mut noise = Noise(7);
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let s = (i as f32 / SAMPLE_RATE as f32 * 1800.0 * std::f32::consts::TAU).sin();
                (s * 0.6 + noise.next() * 0.4) * (1.0 - t).powi(3) * 0.3
            })
            .collect();
        Sound::from_samples(SAMPLE_RATE, &samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::backend::mock::{MockBackend, MockBackendSettings};

    fn mock() -> Audio<MockBackend> {
        Audio::with_settings(AudioManagerSettings {
            backend_settings: MockBackendSettings::default(),
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn decibels() {
        assert_eq!(to_decibels(1.0), Decibels(0.0));
        assert!((to_decibels(0.5).0 + 6.02).abs() < 0.01);
        assert_eq!(to_decibels(0.0), Decibels::SILENCE);
    }

    #[test]
    fn plays_on_buses_and_keeps_volumes() {
        let mut audio = mock();
        assert!(audio.is_enabled());
        let pew = synth::laser(1200.0, 300.0, 0.1);
        assert!(pew.duration() > 0.09);
        audio.play(&pew, Bus::Sfx);
        audio.play_with(&pew, Bus::Voice, PlayParams { volume: 0.5, pitch: 1.5, pan: -0.3 });
        audio.manager().unwrap().backend_mut().on_start_processing();
        assert_eq!(audio.playing(Bus::Sfx), 1);
        assert_eq!(audio.playing(Bus::Voice), 1);
        assert_eq!(audio.playing(Bus::Music), 0);
        audio.set_volume(Bus::Sfx, 0.25);
        audio.set_master_volume(0.8);
        assert_eq!(audio.volume(Bus::Sfx), 0.25);
        assert_eq!(audio.master_volume(), 0.8);
    }

    #[test]
    fn full_bus_drops_sounds() {
        let mut audio = mock();
        let boom = synth::explosion(0.05, 3);
        for _ in 0..VOICES_PER_BUS + 10 {
            audio.play(&boom, Bus::Sfx);
        }
        assert_eq!(audio.dropped(), 10);
    }

    #[test]
    fn silent_audio_does_nothing() {
        let mut audio = Audio::<MockBackend>::silent();
        audio.play(&synth::hit(0.05), Bus::Sfx);
        audio.set_volume(Bus::Music, 0.0);
        assert!(!audio.is_enabled());
        assert_eq!(audio.playing(Bus::Sfx), 0);
        assert!(audio.play_music("missing.ogg", 1.0).is_ok());
    }
}
