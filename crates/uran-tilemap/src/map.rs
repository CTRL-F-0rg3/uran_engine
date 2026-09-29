//! Cała mapa: warstwy, kolizje i ograniczony obszar edytowania.

use std::collections::BTreeMap;

use uran_asset::AssetServer;
use uran_math::{Rect, Vec2};

use crate::autotile::AutoTile;
use crate::layer::{index_from_packed, TileLayer, TILE_EMPTY};
use crate::tileset::Tileset;

/// Kafel z semantyką (nie tylko numer w arkuszu).
///
/// Zwykły indeks mówi tylko „to jest kafel 37", a [`TileId`] mówi też
/// **do czego służy**: co blokuje ruch, po czym można chodzić, czy da się
/// uprawiać. Dzięki temu mapa z pliku XML jest czytelna („to jest woda",
/// „to jest ściana"), a nie listą liczb do rozszyfrowywania.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TileId {
    /// Indeks kafla w arkuszu (`u32::MAX` = brak kafla).
    pub index: u32,
    /// Co kafel blokuje / pozwala — patrz [`TileFlags`].
    pub props: TileProps,
}

impl TileId {
    /// Kafel o danym indeksie, bez właściwości.
    pub const fn new(index: u32) -> Self {
        Self {
            index,
            props: TileProps::EMPTY,
        }
    }

    /// Kafel o indeksie i właściwościach.
    pub const fn with_props(index: u32, props: TileProps) -> Self {
        Self { index, props }
    }

    /// Kafel z kolizją (np. ściana, drzewo, dom).
    pub const fn solid(index: u32) -> Self {
        Self::with_props(index, TileProps::SOLID)
    }

    /// Pusty kafel.
    pub const fn empty() -> Self {
        Self::new(TILE_EMPTY)
    }

    pub const fn is_empty(self) -> bool {
        self.index == TILE_EMPTY
    }
}

/// Właściwości kafla (przechowywane w warstwie osobno od indeksu).
///
/// Trzymamy je **osobno** od numeru kafla, bo ten sam kafel w arkuszu może
/// być albo blokujący, albo nie — zależnie od tego, gdzie go położymy
/// (np. płot na mapie blokuje, ten sam płot jako dekoracja nie musi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TileProps(u16);

impl TileProps {
    pub const EMPTY: Self = Self(0);
    /// Kafel blokuje ruch.
    pub const SOLID: Self = Self(1 << 0);
    /// Na kaflu da się siać (uprawiona ziemia).
    pub const FARMLAND: Self = Self(1 << 1);
    /// Kafel można usunąć (kruszywo, chwast).
    pub const REMOVABLE: Self = Self(1 << 2);
    /// Kafel można postawić (kamień, drzewo).
    pub const PLACEABLE: Self = Self(1 << 3);
    /// Woda — nie można przejść, można w niej łowić.
    pub const WATER: Self = Self(1 << 4);
    /// Kafel da się podlać (ziemia).
    pub const WATERABLE: Self = Self(1 << 5);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_solid(self) -> bool {
        self.contains(Self::SOLID)
    }

    pub const fn is_farmland(self) -> bool {
        self.contains(Self::FARMLAND)
    }

    pub const fn is_water(self) -> bool {
        self.contains(Self::WATER)
    }

    /// Parsuje listę nazw właściwości (`"solid water"`).
    pub fn parse_list(text: &str) -> Self {
        let mut props = Self::EMPTY;
        for word in text.split_whitespace() {
            let bit = match word.to_ascii_lowercase().as_str() {
                "solid" | "blokada" | "wall" => Self::SOLID,
                "farmland" | "pole" | "uprawa" => Self::FARMLAND,
                "removable" | "usun" => Self::REMOVABLE,
                "placeable" | "postaw" => Self::PLACEABLE,
                "water" | "woda" => Self::WATER,
                "waterable" | "polej" => Self::WATERABLE,
                _ => continue,
            };
            props = Self(props.0 | bit.0);
        }
        props
    }
}

/// Warstwa wraz z opisem, w jakim arkuszu szukać jej kafli.
#[derive(Debug, Clone)]
pub struct Layer {
    pub tiles: TileLayer,
    /// Klucz arkusza (nazwa pliku PNG) — patrz [`TileMap::tilesets`].
    pub tileset: String,
    /// Warstwa nieprzechodnia (kolizje sprawdzane są tylko na niej).
    pub solid: bool,
    /// Warstwa edytowalna w grze (podłoga, nie dekoracje).
    pub editable: bool,
    /// Warstwa ukryta.
    pub hidden: bool,
}

impl Layer {
    pub fn new(name: &str, tileset: &str, width: u32, height: u32) -> Self {
        Self {
            tiles: TileLayer::new(name, width, height),
            tileset: tileset.to_string(),
            solid: false,
            editable: false,
            hidden: false,
        }
    }

    /// Ustawia warstwę jako nieprzechodnią.
    pub fn as_solid(mut self) -> Self {
        self.solid = true;
        self
    }

    /// Ustawia warstwę jako edytowalną w grze.
    pub fn as_editable(mut self) -> Self {
        self.editable = true;
        self
    }
}

/// Błąd wczytywania lub edycji mapy.
#[derive(Debug)]
pub enum MapError {
    /// Pliku nie ma lub nie da się go odczytać.
    Io {
        path: String,
        source: std::io::Error,
    },
    /// XML jest błędny składniowo.
    Xml { path: String, message: String },
    /// W pliku brakuje wymaganych elementów/atrybutów.
    Format { path: String, message: String },
    /// W pliku jest błędna liczba.
    Number {
        path: String,
        field: String,
        value: String,
    },
    /// Warstwa odwołuje się do nieznanego arkusza.
    UnknownTileset { layer: String, tileset: String },
    /// Operacja edycji poza dozwolonym zakresem.
    OutsideEditArea { x: i32, y: i32 },
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "nie udało się otworzyć `{path}`: {source}"),
            Self::Xml { path, message } => write!(f, "błąd XML w `{path}`: {message}"),
            Self::Format { path, message } => write!(f, "błędny format `{path}`: {message}"),
            Self::Number { path, field, value } => {
                write!(f, "`{field}` w `{path}` nie jest liczbą: `{value}`")
            }
            Self::UnknownTileset { layer, tileset } => {
                write!(f, "warstwa `{layer}` używa nieznanego arkusza `{tileset}`")
            }
            Self::OutsideEditArea { x, y } => {
                write!(f, "kafel ({x}, {y}) jest poza dozwolonym obszarem edycji")
            }
        }
    }
}

impl std::error::Error for MapError {}

/// Mapa świata: zbiór warstw + reguły kolizji i edycji.
///
/// ## Warstwy
///
/// Warstwy rysowane są **w kolejności w pliku XML** — pierwsza jest spodem,
/// ostatnia na wierzchu. Dzięki temu podłoga może być osobną warstwą od
/// dekoracji, a rośliny od domu, bez rozdzielania tego, co blokuje ruch.
///
/// ## Właściwości kafli
///
/// Numer kafla (`u32`) mówi tylko, co rysujemy. Obok niego trzymamy osobną
/// mapę [`TileProps`] na współrzędne, bo ten sam kafel graficzny może być
/// albo przechodni, albo blokujący.
#[derive(Debug, Clone)]
pub struct TileMap {
    pub name: String,
    /// Rozmiar kafla **w świecie** (jednostki świata, nie piksele tekstury).
    pub tile_size: f32,
    width: u32,
    height: u32,
    pub layers: Vec<Layer>,
    /// Arkusze wczytane z plików PNG: nazwa -> uchwyt.
    pub tilesets: BTreeMap<String, Tileset>,
    /// Właściwości kafli: warstwa -> współrzędne -> właściwości.
    props: BTreeMap<usize, BTreeMap<(i32, i32), TileProps>>,
    /// Obszar, w którym gracz może zmieniać kafle (`None` = cała mapa).
    edit_area: Option<Rect>,
    /// Punkt startowy gracza (w jednostkach świata).
    pub spawn: Option<Vec2>,
    /// Autotiling do przeliczania brzegów po edycji.
    autotiles: BTreeMap<usize, AutoTile>,
}

impl TileMap {
    /// Pusta mapa o danym rozmiarze (bez warstw).
    pub fn new(name: impl Into<String>, tile_size: f32, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            tile_size,
            width,
            height,
            layers: Vec::new(),
            tilesets: BTreeMap::new(),
            props: BTreeMap::new(),
            edit_area: None,
            spawn: None,
            autotiles: BTreeMap::new(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Cały świat jako prostokąt.
    pub fn world_rect(&self) -> Rect {
        Rect::from_xywh(
            0.0,
            0.0,
            self.width as f32 * self.tile_size,
            self.height as f32 * self.tile_size,
        )
    }

    /// Dodaje warstwę (na wierzch listy = na wierzchu sceny).
    pub fn add_layer(&mut self, layer: Layer) -> usize {
        self.props.insert(self.layers.len(), BTreeMap::new());
        self.layers.push(layer);
        self.layers.len() - 1
    }

    /// Znajduje indeks warstwy po nazwie.
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.tiles.name == name)
    }

    /// Rejestruje arkusz kafli pod nazwą (ta sama nazwa co w XML).
    pub fn add_tileset(&mut self, name: impl Into<String>, tileset: Tileset) {
        self.tilesets.insert(name.into(), tileset);
    }

    /// Włącza autotiling dla warstwy o podanym indekszie bazowym.
    pub fn set_autotile(&mut self, layer: usize, autotile: AutoTile) {
        self.autotiles.insert(layer, autotile);
    }

    /// Autotiling przypisany do warstwy.
    pub fn autotile(&self, layer: usize) -> Option<AutoTile> {
        self.autotiles.get(&layer).copied()
    }

    /// Buduje i rejestruje arkusz na podstawie wczytanego obrazu.
    ///
    /// Rozmiar arkusza **musi** pochodzić z faktycznego obrazu, a nie być
    /// wpisany w XML: inaczej kafel wyliczony z błędnego rozmiaru pokazywałby
    /// fragment sąsiada. `tile_px` to rozmiar kafla w pikselach obrazu.
    pub fn add_tileset_from_image(
        &mut self,
        name: &str,
        assets: &mut AssetServer,
        path: &str,
        tile_px: u32,
        margin: u32,
        spacing: u32,
    ) -> Result<(), MapError> {
        let texture = assets.load_image(path).map_err(|e| MapError::Format {
            path: path.to_string(),
            message: format!("nie udało się wczytać arkusza: {e}"),
        })?;
        let size = assets
            .image(texture)
            .map(|img| Vec2::new(img.width as f32, img.height as f32))
            .ok_or_else(|| MapError::Format {
                path: path.to_string(),
                message: "uchwyt obrazu nie wskazuje na wczytany plik".into(),
            })?;
        let tileset = if margin == 0 && spacing == 0 {
            Tileset::new(texture, tile_px, size)
        } else {
            Tileset::with_layout(texture, tile_px, size, margin, spacing)
        };
        self.tilesets.insert(name.to_string(), tileset);
        Ok(())
    }

    /// Właściwości kafla na warstwie `layer` pod (x, y).
    pub fn props_at(&self, layer: usize, x: i32, y: i32) -> TileProps {
        self.props
            .get(&layer)
            .and_then(|m| m.get(&(x, y)))
            .copied()
            .unwrap_or(TileProps::EMPTY)
    }

    /// Ustawia właściwości kafla.
    pub fn set_props(&mut self, layer: usize, x: i32, y: i32, props: TileProps) {
        if let Some(map) = self.props.get_mut(&layer) {
            map.insert((x, y), props);
        }
    }

    /// Kafel z warstwy (indeks `u32`, `TILE_EMPTY` gdy brak).
    pub fn tile_at(&self, layer: usize, x: i32, y: i32) -> u32 {
        self.layers
            .get(layer)
            .map_or(TILE_EMPTY, |l| l.tiles.get(x, y))
    }

    /// Prostokąt kafla w świecie.
    pub fn cell_rect(&self, x: i32, y: i32) -> Rect {
        TileLayer::cell_rect(x, y, self.tile_size)
    }

    /// Środek kafla w świecie (wygodne do stawiania przedmiotów).
    pub fn cell_center(&self, x: i32, y: i32) -> Vec2 {
        self.cell_rect(x, y).center()
    }

    /// Współrzędne kafla dla punktu świata.
    pub fn world_to_cell(&self, p: Vec2) -> (i32, i32) {
        (
            (p.x / self.tile_size).floor() as i32,
            (p.y / self.tile_size).floor() as i32,
        )
    }

    /// Czy (x, y) mieści się w mapie.
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width as i32 && y < self.height as i32
    }

    /// Ustawia obszar, w którym wolno edytować kafle.
    ///
    /// To jest **ograniczenie zasięgu** w grze: gracz ma grabie i farbę, ale
    /// nie może przemalować całej mapy — tylko pole wyznaczone w pliku XML.
    /// `None` oznacza brak ograniczenia.
    pub fn set_edit_area(&mut self, area: Option<Rect>) {
        self.edit_area = area;
    }

    pub fn edit_area(&self) -> Option<Rect> {
        self.edit_area
    }

    /// Czy kafel (x, y) mieści się w mapie **i** w dozwolonym obszarze edycji.
    ///
    /// To jedno miejsce decydujące o tym, czy gracz może zmienić kafel —
    /// dzięki temu limitu nie da się obejść inną metodą.
    pub fn can_edit(&self, x: i32, y: i32) -> bool {
        if !self.in_bounds(x, y) {
            return false;
        }
        match self.edit_area {
            // Nie przycinamy obszaru do mapy: prostokąt z XML może wystawać
            // poza krawędź, a kafle i tak odrzuci `in_bounds`.
            Some(area) => area.contains(self.cell_rect(x, y).center()),
            None => true,
        }
    }

    /// Ustawia kafel na warstwie, pilnując dozwolonego obszaru.
    ///
    /// Zwraca `false` (i nic nie zmienia), gdy kafel jest poza mapą, poza
    /// obszarem edycji albo warstwa nie jest oznaczona jako edytowalna.
    pub fn set_tile(&mut self, layer: usize, x: i32, y: i32, index: u32) -> bool {
        let Some(l) = self.layers.get(layer) else {
            return false;
        };
        if !l.editable || !self.can_edit(x, y) {
            return false;
        }
        // Świeży indeks bez bitów wariantu — flagi trzymamy osobno.
        let index = index_from_packed(index);
        self.layers[layer].tiles.set(x, y, index);
        // Sąsiedzi mogą dostać nowy wariant brzegu, więc odświeżamy ich.
        if let Some(at) = self.autotiles.get(&layer).copied() {
            at.refresh(&mut self.layers[layer].tiles, (x, y), 1);
        }
        true
    }

    /// Czy prostokąt świata styka się z kaflem blokującym ruch.
    ///
    /// Sprawdzamy tylko warstwy oznaczone jako `solid` — podłoga ma być
    /// przechodnia (po niej chodzi się i sadzi), a ściany nie.
    pub fn overlaps_solid(&self, r: Rect) -> bool {
        if self.tile_size <= 0.0 || r.is_empty() {
            return false;
        }
        let x0 = (r.min.x / self.tile_size).floor() as i32;
        let x1 = ((r.max.x - f32::EPSILON) / self.tile_size).floor() as i32;
        let y0 = (r.min.y / self.tile_size).floor() as i32;
        let y1 = ((r.max.y - f32::EPSILON) / self.tile_size).floor() as i32;
        for (i, layer) in self.layers.iter().enumerate() {
            if !layer.solid {
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let t = layer.tiles.get(x, y);
                    if index_from_packed(t) == TILE_EMPTY {
                        continue;
                    }
                    if self.props_at(i, x, y).is_solid() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Czy na kaflu (x, y) można budować (sadzić, stawiać dekoracje).
    pub fn can_build_on(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y) && self.can_edit(x, y)
    }

    /// Właściwości kafla ze **wszystkich** warstw (pierwsza wygrywa).
    ///
    /// Do pytań „czy tu da się siać / czy to woda", gdzie nie obchodzi nas,
    /// z której warstwa pochodzi kafel.
    pub fn props_any_layer(&self, x: i32, y: i32) -> TileProps {
        for i in 0..self.layers.len() {
            let p = self.props_at(i, x, y);
            if p != TileProps::EMPTY {
                return p;
            }
        }
        TileProps::EMPTY
    }

    /// Rysuje widoczną część mapy (wszystkie warstwy po kolei).
    pub fn draw(&self, gfx: &mut uran_render::Graphics<'_>, view: Rect) {
        for layer in &self.layers {
            if layer.hidden {
                continue;
            }
            let Some(tileset) = self.tilesets.get(&layer.tileset) else {
                // Brak arkusza = brak grafiki; mapa wczytana z XML, dla którego
                // nie ma pliku PNG, nie może wywracać gry w panikę.
                continue;
            };
            layer.tiles.draw(gfx, tileset, self.tile_size, view);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mapa 8x8 o kaflu 16 px: podłoga + warstwa przeszkód.
    fn sample() -> TileMap {
        let mut map = TileMap::new("test", 16.0, 8, 8);
        let mut ground = Layer::new("ground", "terrain", 8, 8).as_editable();
        ground.tiles.fill_rect(0, 0, 8, 8, 1);
        map.add_layer(ground);
        let mut walls = Layer::new("walls", "terrain", 8, 8).as_solid();
        walls.tiles.set(3, 3, 5);
        walls.tiles.set(4, 3, 5);
        map.add_layer(walls);
        // Kafel ściany musi mieć właściwość SOLID, inaczej kolizja go pominie.
        map.set_props(1, 3, 3, TileProps::SOLID);
        map.set_props(1, 4, 3, TileProps::SOLID);
        map
    }

    #[test]
    fn world_rect_matches_grid() {
        let m = sample();
        assert_eq!(m.world_rect(), Rect::from_xywh(0.0, 0.0, 128.0, 128.0));
    }

    #[test]
    fn world_to_cell_floor_divides() {
        let m = sample();
        // Środek kafla (2,1) to (40, 24).
        assert_eq!(m.world_to_cell(Vec2::new(40.0, 24.0)), (2, 1));
        assert_eq!(m.world_to_cell(Vec2::new(0.0, 0.0)), (0, 0));
        assert_eq!(m.world_to_cell(Vec2::new(31.9, 31.9)), (1, 1));
    }

    #[test]
    fn solid_tiles_block_movement() {
        let m = sample();
        // Ściana stoi na kaflu (3,3) -> x=48..64, y=48..64.
        assert!(m.overlaps_solid(Rect::from_xywh(50.0, 50.0, 8.0, 8.0)));
        assert!(!m.overlaps_solid(Rect::from_xywh(10.0, 10.0, 8.0, 8.0)));
    }

    #[test]
    fn ground_layer_does_not_block() {
        let m = sample();
        // Podłoga jest wypełniona, ale nie jest warstwą `solid`.
        assert!(!m.overlaps_solid(Rect::from_xywh(10.0, 10.0, 8.0, 8.0)));
    }

    #[test]
    fn tile_without_props_is_walkable_even_on_solid_layer() {
        let mut m = sample();
        // Kafel (3,3) ma w `sample()` właściwość SOLID.
        assert!(m.overlaps_solid(Rect::from_xywh(50.0, 50.0, 8.0, 8.0)));
        // Po usunięciu flagi ten sam kafel graficznie staje się przechodni.
        m.set_props(1, 3, 3, TileProps::EMPTY);
        assert!(!m.overlaps_solid(Rect::from_xywh(50.0, 50.0, 8.0, 8.0)));
    }

    #[test]
    fn set_tile_respects_edit_area() {
        let mut m = sample();
        // Bez limitu można edytować wszędzie.
        assert!(m.set_tile(0, 1, 1, 9));
        assert_eq!(m.tile_at(0, 1, 1), 9);

        // Z limitem tylko pole 2x2 w środku mapy.
        m.set_edit_area(Some(Rect::from_xywh(32.0, 32.0, 32.0, 32.0)));
        assert!(m.can_edit(2, 2), "(2,2) leży w polu");
        assert!(!m.can_edit(6, 6), "(6,6) leży poza polem");
        assert!(
            !m.set_tile(0, 6, 6, 9),
            "edycja poza polem musi się nie udać"
        );
        assert_eq!(m.tile_at(0, 6, 6), 1, "kafel poza polem nietknięty");
    }

    #[test]
    fn set_tile_rejects_non_editable_layer() {
        let mut m = sample();
        // Warstwa `walls` nie jest edytowalna.
        assert!(!m.set_tile(1, 1, 1, 9));
        assert!(!m.set_tile(99, 1, 1, 9), "nie ma takiej warstwy");
    }

    #[test]
    fn can_edit_respects_map_bounds() {
        let m = sample();
        assert!(!m.can_edit(-1, 0));
        assert!(!m.can_edit(0, 8));
        assert!(m.can_edit(7, 7));
    }

    #[test]
    fn props_parse_known_names() {
        let p = TileProps::parse_list("solid water");
        assert!(p.is_solid());
        assert!(p.is_water());
        assert!(!p.is_farmland());
        // Nieznane słowa są ignorowane, nie wywracają parsowania.
        let p = TileProps::parse_list("solid glitch");
        assert!(p.is_solid());
    }

    #[test]
    fn props_any_layer_reports_first_hit() {
        let m = sample();
        assert!(m.props_any_layer(3, 3).is_solid());
        assert_eq!(m.props_any_layer(6, 6), TileProps::EMPTY);
    }

    #[test]
    fn autotile_refresh_runs_on_edit() {
        let mut m = sample();
        m.set_autotile(0, AutoTile::exact(1));
        // Pole 3x3 w środku mapy ma kafel bazowy 1.
        m.layers[0].tiles.fill_rect(2, 2, 3, 3, 1);
        assert!(m.set_tile(0, 3, 3, 1));
        // Środek pola ma sąsiadów ze wszystkich stron -> wariant 0b1111.
        assert_eq!(m.tile_at(0, 3, 3), 1 + 15);
    }
}
