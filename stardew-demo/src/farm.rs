//! Uprawy: sadzenie, wzrost w czasie, podlewanie i zbiór.
//!
//! Uprawy **nie są kaflami mapy** — trzymamy je w osobnej strukturze
//! (`HashMap<(x, y), Crop>`), a do rysowania dokładamy warstwę `crops`.
//! Dzięki temu nie trzeba przepisywać tilemapy przy każdym dniu, a rośliny
//! mogą mieć własny stan (wiek, podlanie) bez dotykania kafli terenu.

use std::collections::HashMap;

/// Stadium wzrostu uprawy (0 = ziarno, 3 = dojrzałe).
pub const STAGES: u8 = 4;

/// Rodzaj uprawy — każdy ma własne stadium w arkuszu i cenę.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropKind {
    /// Rzodkiewka — rośnie najszybciej.
    Radish,
    /// Pszenica.
    Wheat,
    /// Kukurydza.
    Corn,
}

impl CropKind {
    /// Pierwszy indeks kafla w arkuszu roślin (kolumna w rzędzie).
    ///
    /// Arkusz `crops_dense_spring` ma 4 rzędy × 16 kolumn: rząd `y` to
    /// stadium wzrostu, a kolumna to gatunek. Stąd `y * 16 + x`.
    pub const fn base_tile(self) -> u32 {
        match self {
            CropKind::Radish => 0,
            CropKind::Wheat => 1,
            CropKind::Corn => 2,
        }
    }

    /// Indeks kafla danego stadium w arkuszu roślin.
    pub const fn tile_for_stage(self, stage: u8) -> u32 {
        (stage as u32 % STAGES as u32) * 16 + self.base_tile()
    }

    /// Koszt nasion.
    pub const fn seed_cost(self) -> u32 {
        match self {
            CropKind::Radish => 10,
            CropKind::Wheat => 20,
            CropKind::Corn => 35,
        }
    }

    /// Nazwa do HUD-u.
    pub const fn name(self) -> &'static str {
        match self {
            CropKind::Radish => "Rzodkiewka",
            CropKind::Wheat => "Pszenica",
            CropKind::Corn => "Kukurydza",
        }
    }

    /// Ile dni potrzebuje od posadzenia do dojrzałości.
    pub const fn grow_days(self) -> u32 {
        match self {
            CropKind::Radish => 2,
            CropKind::Wheat => 3,
            CropKind::Corn => 4,
        }
    }

    /// Wartość zbioru (sztuk dojrzałych).
    pub const fn yield_value(self) -> u32 {
        match self {
            CropKind::Radish => 35,
            CropKind::Wheat => 60,
            CropKind::Corn => 90,
        }
    }
}

/// Jedna uprawa na polu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub kind: CropKind,
    /// Stadium wzrostu 0..3.
    pub stage: u8,
    /// Czy pole było podlane dziś (uprawy schną bez podlania).
    pub watered: bool,
    /// Ile dni rośnie (dojrzałość = `grow_days`).
    pub age: u32,
}

impl Crop {
    pub fn new(kind: CropKind) -> Self {
        Self {
            kind,
            stage: 0,
            watered: false,
            age: 0,
        }
    }

    /// Czy uprawa jest gotowa do zbioru.
    pub fn is_ripe(&self) -> bool {
        self.stage >= STAGES - 1
    }
}

/// Stan wszystkich upraw na mapie.
#[derive(Debug, Default)]
pub struct Farm {
    crops: HashMap<(i32, i32), Crop>,
}

impl Farm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Uprawa na polu (x, y).
    pub fn get(&self, x: i32, y: i32) -> Option<&Crop> {
        self.crops.get(&(x, y))
    }

    /// Czy na polu (x, y) coś rośnie.
    pub fn is_occupied(&self, x: i32, y: i32) -> bool {
        self.crops.contains_key(&(x, y))
    }

    /// Sadzi `kind` na polu; `false` gdy pole jest zajęte.
    pub fn plant(&mut self, x: i32, y: i32, kind: CropKind) -> bool {
        if self.crops.contains_key(&(x, y)) {
            return false;
        }
        self.crops.insert((x, y), Crop::new(kind));
        true
    }

    /// Zdejmuje uprawę z pola (zbiór lub zniszczenie).
    pub fn remove(&mut self, x: i32, y: i32) -> Option<Crop> {
        self.crops.remove(&(x, y))
    }

    /// Podlewa pole; `false` gdy nie ma na nim uprawy albo już jest podlane.
    pub fn water(&mut self, x: i32, y: i32) -> bool {
        match self.crops.get_mut(&(x, y)) {
            Some(crop) if !crop.watered => {
                crop.watered = true;
                true
            }
            _ => false,
        }
    }

    /// Liczba upraw na mapie.
    pub fn len(&self) -> usize {
        self.crops.len()
    }

    /// Liczba dojrzałych upraw (do HUD-u).
    pub fn ripe_count(&self) -> usize {
        self.crops.values().filter(|c| c.is_ripe()).count()
    }

    /// Pojedynczy dzień: podlane uprawy rosną o jeden stopień.
    ///
    /// Zwraca liczbę dojrzałych, które właśnie urosły — używane do komunikatu
    /// „Twoje rzodkiewki są gotowe".
    pub fn advance_day(&mut self) -> usize {
        let mut newly_ripe = 0;
        let values: Vec<(i32, i32)> = self.crops.keys().copied().collect();
        for key in values {
            let Some(crop) = self.crops.get_mut(&key) else {
                continue;
            };
            // Niepodlane uprawy nie rosną — i tracą podlanie na nowy dzień.
            if crop.watered && !crop.is_ripe() {
                let before = crop.is_ripe();
                crop.age += 1;
                crop.stage = (crop.age as f32 / crop.kind.grow_days() as f32 * (STAGES - 1) as f32)
                    .floor()
                    .clamp(0.0, (STAGES - 1) as f32) as u8;
                if !before && crop.is_ripe() {
                    newly_ripe += 1;
                }
            }
            crop.watered = false;
        }
        newly_ripe
    }

    /// Wszystkie uprawy (do rysowania).
    pub fn iter(&self) -> impl Iterator<Item = (&(i32, i32), &Crop)> {
        self.crops.iter()
    }
}
