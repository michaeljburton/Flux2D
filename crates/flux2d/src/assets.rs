//! Finding and loading game files: art, sounds, fonts and JSON data.
//!
//! An [`Assets`] is a root folder. [`Assets::locate`] finds one the way a game run from a
//! source checkout needs: an environment variable first, then the folder next to the
//! executable, then the working directory and the folders above both (so the Cosmic Commando
//! repo sitting next to this one is found from `cargo run`).

use flux2d_audio::{AudioError, Sound};
use flux2d_render::{AtlasBuilder, AtlasError, FontId, SpriteId};
use serde::de::DeserializeOwned;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum AssetError {
    Io(PathBuf, std::io::Error),
    Json(PathBuf, serde_json::Error),
    Atlas(AtlasError),
    Audio(AudioError),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            AssetError::Json(p, e) => write!(f, "{}: {e}", p.display()),
            AssetError::Atlas(e) => write!(f, "{e}"),
            AssetError::Audio(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// A folder of game files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assets {
    root: PathBuf,
}

/// How many folders up from the working directory and the executable [`Assets::locate`] looks.
const SEARCH_UP: usize = 5;

impl Assets {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Finds a folder that contains `marker` (a file or folder inside it, to be sure it's the
    /// right one). Tries, in order: the `env` variable's path, then each of `candidates` (paths
    /// relative to a search folder, e.g. `"cosmic-commando/cosmic-commando-2d"`) under the
    /// executable's folder, the working directory, and up to five folders above each.
    pub fn locate(env: Option<&str>, candidates: &[&str], marker: &str) -> Option<Assets> {
        if let Some(var) = env
            && let Some(p) = std::env::var_os(var)
        {
            let p = PathBuf::from(p);
            if p.join(marker).exists() {
                return Some(Assets::new(p));
            }
            log::warn!("{var}={} has no {marker}", p.display());
        }
        let mut starts = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            starts.push(dir.to_path_buf());
        }
        if let Ok(cwd) = std::env::current_dir() {
            starts.push(cwd);
        }
        for start in starts {
            for dir in start.ancestors().take(SEARCH_UP + 1) {
                for c in candidates.iter().copied().chain([""]) {
                    let root = if c.is_empty() { dir.to_path_buf() } else { dir.join(c) };
                    if root.join(marker).exists() {
                        return Some(Assets::new(root));
                    }
                }
            }
        }
        None
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The full path of `rel` (a `/`-separated path inside the root).
    pub fn path(&self, rel: &str) -> PathBuf {
        rel.split('/').filter(|s| !s.is_empty()).fold(self.root.clone(), |p, s| p.join(s))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// The same files under a subfolder.
    pub fn sub(&self, rel: &str) -> Assets {
        Assets::new(self.path(rel))
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>, AssetError> {
        let p = self.path(rel);
        std::fs::read(&p).map_err(|e| AssetError::Io(p, e))
    }

    pub fn read_to_string(&self, rel: &str) -> Result<String, AssetError> {
        let p = self.path(rel);
        std::fs::read_to_string(&p).map_err(|e| AssetError::Io(p, e))
    }

    /// Parses a JSON file into any `serde` type (Cosmic Commando's missions, story and comics
    /// are JSON).
    pub fn json<T: DeserializeOwned>(&self, rel: &str) -> Result<T, AssetError> {
        let p = self.path(rel);
        let text = std::fs::read_to_string(&p).map_err(|e| AssetError::Io(p.clone(), e))?;
        serde_json::from_str(&text).map_err(|e| AssetError::Json(p, e))
    }

    /// Loads a sound into memory.
    pub fn sound(&self, rel: &str) -> Result<Sound, AssetError> {
        Sound::load(self.path(rel)).map_err(AssetError::Audio)
    }

    /// Adds one image to `atlas`, scaled down to `max_size` if given.
    pub fn image(
        &self,
        atlas: &mut AtlasBuilder,
        rel: &str,
        max_size: Option<u32>,
    ) -> Result<SpriteId, AssetError> {
        atlas.add_file(self.path(rel), max_size).map_err(AssetError::Atlas)
    }

    /// Adds every image under the folder `rel` to `atlas`, decoded in parallel and named by
    /// their path inside that folder (see [`AtlasBuilder::add_dir`]).
    pub fn images(
        &self,
        atlas: &mut AtlasBuilder,
        rel: &str,
        max_size: Option<u32>,
    ) -> Result<Vec<(String, SpriteId)>, AssetError> {
        atlas.add_dir(self.path(rel), max_size).map_err(AssetError::Atlas)
    }

    /// Bakes a font file into `atlas` at `px` pixels.
    pub fn font(
        &self,
        atlas: &mut AtlasBuilder,
        rel: &str,
        px: f32,
        chars: &str,
    ) -> Result<FontId, AssetError> {
        atlas.add_font_file(self.path(rel), px, chars).map_err(AssetError::Atlas)
    }
}

/// A TrueType font from the operating system, for tools and examples that ship no font of
/// their own: Segoe UI or Arial on Windows, Helvetica or Arial on macOS, DejaVu or Liberation
/// on Linux.
pub fn system_font() -> Option<PathBuf> {
    let candidates = [
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_json_and_locate() {
        let dir = std::env::temp_dir().join(format!("flux2d-assets-test-{}", std::process::id()));
        let data = dir.join("game/data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("wave.json"), r#"{"enemies": 12, "name": "first"}"#).unwrap();

        #[derive(serde::Deserialize)]
        struct Wave {
            enemies: u32,
            name: String,
        }
        let a = Assets::new(dir.join("game"));
        assert!(a.exists("data/wave.json"));
        let w: Wave = a.json("data/wave.json").unwrap();
        assert_eq!((w.enemies, w.name.as_str()), (12, "first"));
        assert!(matches!(a.json::<Wave>("data/missing.json"), Err(AssetError::Io(..))));
        assert_eq!(a.sub("data").path("wave.json"), data.join("wave.json"));

        // SAFETY: tests in this crate don't read this variable from other threads.
        unsafe { std::env::set_var("FLUX2D_TEST_ASSETS", dir.join("game")) };
        let found = Assets::locate(Some("FLUX2D_TEST_ASSETS"), &[], "data/wave.json").unwrap();
        assert_eq!(found.root(), dir.join("game"));
        assert!(Assets::locate(None, &["no-such-folder"], "data/nothing-here.json").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
