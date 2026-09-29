//! Wczytywanie modeli budynków z katalogu `assets/`.
//!
//! ## Konwencja osi: te pliki są Y-up
//!
//! Wbrew pierwszemu wrażeniu (`# Blender v2.79` w nagłówku) pliki
//! Quaternius **nie są** Z-up. Dowód z samych wymiarów:
//!
//! | model | x | y | z | wniosek |
//! |-------|---|---|---|---------|
//! | `Fence` | 5,89 | **1,10** | 0,17 | płot jest długi w x, niski w y |
//! | `Silo` | 3,67 | **9,07** | 3,51 | silos jest wysoki w y, okrągły w x/z |
//!
//! Gdyby to był Z-up, `Fence` miałby 5,89 m **grubości** i 0,17 m
//! wysokości, a `Silo` byłby płaskim krążkiem. Żaden z tych modeli
//! nie pasowałby do opisu.
//!
//! Dlatego ładujemy `AxisUp::Y` (to układ docelowy, importer nic nie
//! obraca). Przy `AxisUp::Z` wszystkie budynki lądują na boku — co
//! widać na zrzucie ekranu jako „kupa belek" zamiast wsi.
//!
//! ## Dlaczego osobna funkcja szukająca pliku
//!
//! `cargo run -p farm-simulator` uruchamia grę z katalogu repozytorium,
//! a `assets/` leży w katalogu crate'a. Sama ścieżka względna by nie
//! zadziałała i dostalibyśmy pusty świat zamiast farmy.

use std::path::{Path, PathBuf};

use uran_render3d::import::{self, AxisUp, ImportedModel};

/// Katalog z modelami w repozytorium.
const ASSET_SUBDIR: &str = "Farm Buildings by Quaternius/OBJ";

/// Wybrane budynki: nazwa pliku + docelowa wysokość w metrach.
///
/// Wysokość jest jedyną skalą, jakiej potrzebujemy — budynki stawiamy
/// na ziemi (import już to robi) i skalujemy jednolicie, żeby zachować
/// proporcje.
pub struct BuildingSpec {
    pub file: &'static str,
    /// Wysokość docelowa w metrach.
    pub height: f32,
}

/// Lista budynków użytych w grze.
///
/// Wybór jest celowo mały: stodoło, silos, młyn i studnia. Każdy
/// budynek to kilkanaście siatek GPU i kilkadziesiąt draw calli, a gra
/// ma być czytelna — sześć różnych budynków rozrzuconych po polu
/// tylko przeszkadza w rozpoznaniu, co jest uprawą.
pub const BUILDINGS: &[BuildingSpec] = &[
    BuildingSpec {
        file: "BigBarn",
        // Model ma 7,89 m — zostawiamy naturalną skalę, żeby proporcje
        // dachu i ścian odpowiadały oryginałowi.
        height: 7.89,
    },
    BuildingSpec {
        file: "Silo",
        // Naturalne 9,07 m: silos ma być wyraźnie wyższy od stodoła,
        // bo to on zaznacza pion i pomaga zlokalizować się na mapie.
        height: 9.07,
    },
    BuildingSpec {
        file: "Windmill",
        // 11,20 m — najwyższy element, dobrze widoczny z całej farmy.
        height: 11.20,
    },
    BuildingSpec {
        file: "Well",
        height: 2.15,
    },
];

/// Wysokość płotu w metrach.
///
/// Model ma 1,10 m i 5,89 m długości. Nie skalujemy — naturalna
/// wysokość jest poprawna, a proporcja długość/wysokość (5,4:1)
/// daje dokładnie taki płot, jakiego oczekujemy.
pub const FENCE_HEIGHT: f32 = 1.10;

/// Ścieżka katalogu z modelami, znaleziona w kilku miejscach.
pub fn asset_dir() -> Option<PathBuf> {
    let rel = Path::new("assets").join(ASSET_SUBDIR);
    if rel.is_dir() {
        return Some(rel);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(ASSET_SUBDIR);
    manifest.is_dir().then_some(manifest)
}

/// Ścieżka jednego pliku `.obj`.
pub fn model_path(file: &str) -> Option<PathBuf> {
    let dir = asset_dir()?;
    let p = dir.join(format!("{file}.obj"));
    p.is_file().then_some(p)
}

/// Wczytuje model budynku i skaluje go do zadanej wysokości.
///
/// Zwraca `None` z komunikatem na `stderr`, gdy pliku nie ma — gra ma
/// działać dalej (sama farma), bo brak stodoła to brak dekoracji,
/// a nie powód do pustego kadru.
pub fn load_building(file: &str, height: f32) -> Option<ImportedModel> {
    let path = model_path(file)?;
    // `AxisUp::Y` — te pliki SĄ Y-up (dowody w dokumentacji modułu).
    match import::load_from_file_with_axes(&path, AxisUp::Y) {
        Ok(mut model) => {
            model.scale_to_height(height);
            Some(model)
        }
        Err(e) => {
            eprintln!("⚠️  nie udało się wczytać `{}`: {e}", path.display());
            None
        }
    }
}

/// Ile budynków udało się wczytać (do diagnostyki na starcie).
pub fn available_buildings() -> usize {
    BUILDINGS
        .iter()
        .filter(|b| model_path(b.file).is_some())
        .count()
}
