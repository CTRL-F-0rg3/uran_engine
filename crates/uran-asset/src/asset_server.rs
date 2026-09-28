//! `AssetServer` — punkt wejścia do ładowania assetów (obrazy, czcionki).
//!
//! Wczytanie tego samego pliku dwa razy zwraca ten sam uchwyt (cache po
//! ścieżce). Wszystko działa synchronicznie — to świadoma decyzja na tym
//! etapie: gry 2D zwykle mają mało i małe assety, a synchroniczne API jest
//! znacznie wygodniejsze w użyciu niż callbacki.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::handle::Handle;
use crate::loader::{AssetError, FontData, Image};
use crate::server::Assets;

/// Katalog domyślny, w którym szukamy assetów.
pub const DEFAULT_ASSET_ROOT: &str = "assets";

#[derive(Default)]
pub struct AssetServer {
    images: Assets<Image>,
    fonts: Assets<FontData>,
    /// cache „ścieżka -> uchwyt", żeby nie czytać pliku dwa razy
    image_cache: HashMap<PathBuf, Handle<Image>>,
    font_cache: HashMap<PathBuf, Handle<FontData>>,
    root: PathBuf,
}

impl AssetServer {
    /// Serwer assetów rooted w `root` (np. `assets/`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            images: Assets::new(),
            fonts: Assets::new(),
            image_cache: HashMap::new(),
            font_cache: HashMap::new(),
            root: root.into(),
        }
    }

    /// Serwer assetów w domyślnym katalogu `assets/`.
    pub fn with_default_root() -> Self {
        Self::new(DEFAULT_ASSET_ROOT)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Wczytuje PNG (z cache) i zwraca uchwyt obrazu.
    pub fn load_image(&mut self, path: impl AsRef<Path>) -> Result<Handle<Image>, AssetError> {
        let full = self.resolve(path.as_ref());
        if let Some(handle) = self.image_cache.get(&full) {
            return Ok(*handle);
        }
        let image = Image::load_png(&full)?;
        let handle = self.images.insert(image);
        self.image_cache.insert(full, handle);
        Ok(handle)
    }

    /// Rejestruje gotowy obraz (np. wygenerowany proceduralnie) i zwraca uchwyt.
    pub fn create_image(&mut self, image: Image) -> Handle<Image> {
        self.images.insert(image)
    }

    pub fn image(&self, handle: Handle<Image>) -> Option<&Image> {
        self.images.get(handle)
    }

    pub fn image_mut(&mut self, handle: Handle<Image>) -> Option<&mut Image> {
        self.images.get_mut(handle)
    }

    /// Wczytuje czcionkę (z cache) i zwraca uchwyt.
    pub fn load_font(&mut self, path: impl AsRef<Path>) -> Result<Handle<FontData>, AssetError> {
        let full = self.resolve(path.as_ref());
        if let Some(handle) = self.font_cache.get(&full) {
            return Ok(*handle);
        }
        let font = FontData::load(&full)?;
        let handle = self.fonts.insert(font);
        self.font_cache.insert(full, handle);
        Ok(handle)
    }

    pub fn font(&self, handle: Handle<FontData>) -> Option<&FontData> {
        self.fonts.get(handle)
    }

    /// Ścieżka bezwzględna względem katalogu assetów.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_read_back_image() {
        let mut server = AssetServer::new("assets");
        let mut img = Image::new(2, 2);
        img.fill_rect(0, 0, 2, 2, [9, 8, 7, 255]);
        let handle = server.create_image(img);
        assert_eq!(server.image(handle).unwrap().pixel(0, 0), [9, 8, 7, 255]);
    }

    #[test]
    fn resolve_is_relative_to_root() {
        let server = AssetServer::new("assets");
        assert_eq!(server.resolve(Path::new("player.png")), PathBuf::from("assets/player.png"));
        assert_eq!(server.resolve(Path::new("/abs/player.png")), PathBuf::from("/abs/player.png"));
    }

    #[test]
    fn load_image_caches_by_path() {
        let dir = std::env::temp_dir().join("uran_assets_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("tile.png");
        Image::filled(1, 1, [1, 2, 3, 255]).save_png(&file).unwrap();

        let mut server = AssetServer::new(&dir);
        let a = server.load_image("tile.png").unwrap();
        let b = server.load_image("tile.png").unwrap();
        assert_eq!(a, b, "drugie wczytanie tego samego pliku zwraca ten sam uchwyt");
        assert!(server.load_image("nie_ma_mnie.png").is_err());

        let _ = std::fs::remove_file(&file);
    }
}
