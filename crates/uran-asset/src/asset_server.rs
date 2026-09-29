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

    /// Znajduje katalog assetów gry i ustawia go jako korzeń.
    ///
    /// Kolejność kandydatów opisuje [`asset_dir_candidates`]; najpierw jest
    /// sprawdzany podany katalog (względem katalogu roboczego), potem
    /// `URAN_ASSET_ROOT`, katalog wykonywalnej i kilka poziomów w górę od
    /// niej (bo `target/debug/gra` ma `assets/` dwa katalogi wyżej).
    pub fn auto_root(preferred: impl Into<PathBuf>) -> Self {
        let preferred = preferred.into();
        let candidates = asset_dir_candidates(&preferred, None);
        Self::new(pick_asset_dir(&candidates).unwrap_or(preferred))
    }

    /// Jak [`AssetServer::auto_root`], ale zna dodatkowo katalog crate'a
    /// (`CARGO_MANIFEST_DIR`).
    ///
    /// Dzięki temu `cargo run -p gra` działa z **dowolnego** katalogu
    /// roboczego: binarka leży w `target/debug/`, a assety w `gra/assets/`.
    /// Bez podania katalogu crate'a żaden kandydat nie wskazywałby na
    /// `stardew-demo/assets` i gra zgłaszałaby brak plików.
    pub fn auto_root_in_manifest(preferred: impl Into<PathBuf>, manifest_dir: &Path) -> Self {
        let preferred = preferred.into();
        let candidates = asset_dir_candidates(&preferred, Some(manifest_dir));
        Self::new(pick_asset_dir(&candidates).unwrap_or(preferred))
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
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

/// Zmienna środowiskowa pozwalająca wskazać katalog assetów wprost.
pub const ASSET_ROOT_ENV: &str = "URAN_ASSET_ROOT";

/// Lista katalogów, w których szukamy assetów, w kolejności rosnącego
/// priorytetu znaczenia.
///
/// Kolejność:
/// 1. `URAN_ASSET_ROOT` — jawne wskazanie (CI, uruchomienie z własnego
///    skryptu, instalacja w nietypowym miejscu),
/// 2. `preferred` względem katalogu roboczego — `cd gra && cargo run`,
/// 3. `preferred` w katalogu crate'a — `cargo run -p gra` z dowolnego
///    miejsca (to jest przypadek, który psuł `cargo run -p stardew-demo`
///    uruchomione z katalogu workspace: binarka jest w `target/debug/`,
///    a assety w `stardew-demo/assets/`),
/// 4. `preferred` obok wykonywalnej i 1–3 poziomy w górę — binarka
///    zainstalowana razem z assetami.
pub fn asset_dir_candidates(preferred: &Path, manifest_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var(ASSET_ROOT_ENV) {
        if !dir.is_empty() {
            candidates.push(PathBuf::from(dir));
        }
    }
    candidates.push(preferred.to_path_buf());
    if let Some(manifest) = manifest_dir {
        candidates.push(manifest.join(preferred));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(preferred));
            for up in 1usize..=3 {
                let mut base = dir.to_path_buf();
                for _ in 0..up {
                    if !base.pop() {
                        break;
                    }
                }
                candidates.push(base.join(preferred));
            }
        }
    }
    candidates
}

/// Pierwszy istniejący katalog z listy.
fn pick_asset_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|d| d.is_dir()).cloned()
}

/// Wyznacza katalog assetów gry biorąc pod uwagę `CARGO_MANIFEST_DIR`.
///
/// Typowe użycie w `main.rs`:
///
/// ```ignore
/// let assets = uran_asset::find_asset_dir(env!("CARGO_MANIFEST_DIR"));
/// App::new().assets(assets).run();
/// ```
///
/// Gdy nic nie znaleziono, zwraca `assets/` — komunikat błędu pokazuje
/// wtedy ścieżkę względną, a nie pustą.
pub fn find_asset_dir(manifest_dir: &str) -> PathBuf {
    let preferred = PathBuf::from(DEFAULT_ASSET_ROOT);
    let candidates = asset_dir_candidates(&preferred, Some(Path::new(manifest_dir)));
    pick_asset_dir(&candidates).unwrap_or(preferred)
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
    fn candidates_include_manifest_and_prefer_working_dir() {
        // `cargo run -p gra` z katalogu workspace: katalog roboczy (`assets/`)
        // nie istnieje, ale katalog crate'a tak — i musi być wśród kandydatów,
        // inaczej demo zgłasza brak plików mimo że są na dysku.
        let manifest = Path::new("/tmp/gra_crate");
        let candidates = asset_dir_candidates(Path::new("assets"), Some(manifest));
        assert!(
            candidates.contains(&PathBuf::from("assets")),
            "katalog roboczy musi być kandydatem"
        );
        assert!(
            candidates.contains(&PathBuf::from("/tmp/gra_crate/assets")),
            "katalog crate'a musi być kandydatem, inaczej `cargo run -p gra` \
             zgłasza brak plikow; kandydaci: {candidates:?}"
        );
        // Katalog roboczy jest sprawdzany przed katalogiem crate'a, żeby
        // uruchomienie z własnego katalogu gry wygrało nad danymi z repo.
        let pos_cwd = candidates
            .iter()
            .position(|c| c == Path::new("assets"))
            .expect("katalog roboczy");
        let pos_manifest = candidates
            .iter()
            .position(|c| c == &manifest.join("assets"))
            .expect("katalog crate'a");
        assert!(pos_cwd < pos_manifest);
    }

    #[test]
    fn picks_first_existing_candidate() {
        let base = std::env::temp_dir().join("uran_root_pick_test");
        let existing = base.join("ma_assets");
        std::fs::create_dir_all(&existing).unwrap();
        let missing = base.join("nie_ma_assets");
        let _ = std::fs::remove_dir_all(&missing);

        let found = pick_asset_dir(&[missing.clone(), existing.clone()]);
        assert_eq!(found.as_deref(), Some(existing.as_path()));
        assert_eq!(pick_asset_dir(&[missing]), None);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_is_relative_to_root() {
        let server = AssetServer::new("assets");
        assert_eq!(
            server.resolve(Path::new("player.png")),
            PathBuf::from("assets/player.png")
        );
        assert_eq!(
            server.resolve(Path::new("/abs/player.png")),
            PathBuf::from("/abs/player.png")
        );
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
        assert_eq!(
            a, b,
            "drugie wczytanie tego samego pliku zwraca ten sam uchwyt"
        );
        assert!(server.load_image("nie_ma_mnie.png").is_err());

        let _ = std::fs::remove_file(&file);
    }
}
