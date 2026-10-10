//! Tekst: atlas glifów rasteryzowanych na żądanie + składanie tekstu.
//!
//! Podejście jest takie samo jak w większości silników 2D: glif rasteryzujemy
//! raz, wkładamy do atlasu R8 i od tej chwili każdy znak jest zwykłym
//! sprite'em z prostokątnym UV. Dzięki temu cały HUD to nadal jeden batch,
//! bez osobnego potoku do tekstu.

use ab_glyph::{Font as _, FontVec, GlyphId, InvalidFont, PxScale};
use std::collections::HashMap;
use uran_asset::{FontData, Handle, HandleId};
use uran_ecs::UvRect;
use uran_math::{Mat3, Rect, Vec2};

use crate::batch::{SpriteDraw, TextAlign, TextDraw, TextureKey};

/// Bok atlasu glifów w pikselach.
pub const ATLAS_SIZE: u32 = 1024;

/// Odstęp między glifami w atlasie (zapobiega „prześwitywaniu" przy filtrowaniu).
const ATLAS_PADDING: u32 = 1;

/// Limit znaków przetwarzanych w jednym wywołaniu.
const MAX_TEXT_CHARS: usize = 8192;

/// Pojedynczy glif w atlasie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphInfo {
    /// Prostokąt glifu w atlasie (piksele).
    pub rect: Rect,
    /// Przesunięcie od początku linii bazowej (piksele).
    pub offset: Vec2,
    /// Rozmiar glifu (piksele).
    pub size: Vec2,
    /// Szerokość posuwu (piksele).
    pub advance: f32,
}

/// Stan jednej czcionki: atlas + cache glifów.
struct FontEntry {
    font: FontVec,
    units_per_em: f32,
    ascent: f32,
    descent: f32,
    line_gap: f32,
    /// Zawartość atlasu (R8, `ATLAS_SIZE` x `ATLAS_SIZE`).
    pixels: Vec<u8>,
    /// Kolejna wolna kolumna na bieżącej półce.
    pen_x: u32,
    /// Wysokość zajętych półek.
    shelf_y: u32,
    /// Największa wysokość glifu na BIEŻĄCEJ półce.
    ///
    /// Bez tego nowa półka zaczynałaby się zbyt wysoko (lub zbyt nisko,
    /// jeśli przesuwamy ją o wysokość pojedynczego glifu) i glify z
    /// różnych półek nachodziłyby na siebie.
    shelf_height: u32,
    /// Rasteryzowany rozmiar w pikselach.
    raster_size: u32,
    /// Czy atlas zmienił się od ostatniego wgrania na GPU.
    dirty: bool,
    glyphs: HashMap<GlyphId, Option<GlyphInfo>>,
    /// Ile razy atlas się zapełnił (diagnostyka).
    resets: u32,
}

impl FontEntry {
    fn new(data: &[u8], raster_size: u32) -> Result<Self, InvalidFont> {
        let font = FontVec::try_from_vec(data.to_vec())?;
        Ok(Self {
            units_per_em: font.units_per_em().unwrap_or(1000.0),
            ascent: font.ascent_unscaled(),
            descent: font.descent_unscaled(),
            line_gap: font.line_gap_unscaled(),
            font,
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize],
            pen_x: 0,
            shelf_y: 0,
            shelf_height: 0,
            raster_size,
            dirty: true,
            glyphs: HashMap::new(),
            resets: 0,
        })
    }

    /// Zwalnia miejsce w atlasie (pełne wypełnienie = zaczynamy od nowa).
    fn reset_atlas(&mut self) {
        self.pixels.fill(0);
        self.pen_x = 0;
        self.shelf_y = 0;
        self.shelf_height = 0;
        self.glyphs.clear();
        self.resets += 1;
        self.dirty = true;
    }

    /// Rezerwuje prostokąt w atlasie; `None` = brak miejsca.
    ///
    /// UWAGA na `shelf_height`: nowa półka zaczyna się PO
    /// `shelf_y + shelf_height`, a NIE po `shelf_y + h`. Glify mają różne
    /// wysokości (kropka 2 px, „W" 20 px) i przesuwanie półki o wysokość
    /// pojedynczego glifu kładło niski znak NA bitmapach wyżej położonych
    /// liter. Sąsiadujące glify nachodziły na siebie i w HUD-zie „o"
    /// wyglądało jak „q".
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w + ATLAS_PADDING > ATLAS_SIZE || h + ATLAS_PADDING > ATLAS_SIZE {
            return None;
        }
        if self.pen_x + w + ATLAS_PADDING > ATLAS_SIZE {
            // zamykamy bieżącą półkę i zaczynamy nową na jej wysokości
            self.shelf_y += self.shelf_height + ATLAS_PADDING;
            self.pen_x = 0;
            self.shelf_height = 0;
        }
        if self.shelf_y + h + ATLAS_PADDING > ATLAS_SIZE {
            // brak miejsca — wywołujący zdecyduje, czy zrobić reset
            return None;
        }
        let at = (self.pen_x, self.shelf_y);
        self.pen_x += w + ATLAS_PADDING;
        self.shelf_height = self.shelf_height.max(h);
        self.dirty = true;
        Some(at)
    }

    /// Rasteryzuje glif (raz) i zwraca jego pozycję w atlasie.
    fn rasterize(&mut self, id: GlyphId) -> Option<GlyphInfo> {
        if let Some(cached) = self.glyphs.get(&id) {
            return *cached;
        }
        // UWAGA: `PxScale` w ab_glyph oznacza liczbę pikseli na `em`, a NIE
        // na jednostkę fontu — ab_glyph sam dzieli przez `units_per_em`.
        // Podanie `raster_size / upem` kurczyło cały em do ułamka piksela
        // i każdy glif wychodził 1x1 (tekst był niewidoczny).
        let scale = PxScale::from(self.raster_size as f32);
        let outlined = self.font.outline_glyph(ab_glyph::Glyph {
            id,
            scale,
            position: ab_glyph::Point { x: 0.0, y: 0.0 },
        })?;
        let bounds = outlined.px_bounds();
        let w = bounds.width().ceil().max(0.0) as u32;
        let h = bounds.height().ceil().max(0.0) as u32;
        let advance =
            self.font.h_advance_unscaled(id) / self.units_per_em * self.raster_size as f32;

        if w == 0 || h == 0 {
            // spacja albo znak bez pokrycia
            let info = GlyphInfo {
                rect: Rect::ZERO,
                offset: Vec2::ZERO,
                size: Vec2::ZERO,
                advance,
            };
            self.glyphs.insert(id, Some(info));
            return Some(info);
        }

        // nie zmieścimy się na żadnej półce — czyścimy atlas i próbujemy od nowa
        let at = match self.allocate(w, h) {
            Some(at) => at,
            None => {
                self.reset_atlas();
                self.allocate(w, h)?
            }
        };
        let (x, y) = at;

        // Rysujemy z marginesem, żeby filtrowanie liniowe nie łapało sąsiadów.
        //
        // UWAGA: `Outlined::draw` podaje współrzędne JUŻ przesunięte o
        // `-px_bounds.min` i z osią Y skierowaną w dół, czyli dokładnie
        // jako indeksy w bitmapie glifu. Odejmowanie `bounds.min` po raz
        // drugi wyrzucało poza bitmapę i zostawał ledwie widoczny pasek.
        outlined.draw(|px, py, coverage| {
            if px >= w || py >= h {
                return;
            }
            let dst = ((y + py) * ATLAS_SIZE + x + px) as usize;
            self.pixels[dst] = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
        });

        // `size` i `offset` podajemy jako UŁAMEK wysokości `em` (bitmap
        // dzielony przez `raster_size`), a nie w jednostkach fontu — dzięki
        // temu `layout` mnoży je przez `cmd.size / units_per_em` i dostaje
        // rozmiar w jednostkach świata, niezależnie od rozdzielczości atlasu.
        let em = self.raster_size as f32;
        let info = GlyphInfo {
            rect: Rect::from_xywh(x as f32, y as f32, w as f32, h as f32),
            offset: Vec2::new(bounds.min.x / em, bounds.min.y / em),
            size: Vec2::new(w as f32 / em, h as f32 / em),
            advance: advance / em,
        };
        self.glyphs.insert(id, Some(info));
        Some(info)
    }
}

/// Glif po ułożeniu w tekście — używany w testach i diagnostyce.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    /// Jakiego fragmentu atlasu użyć.
    pub uv: UvRect,
    /// Pozycja środka kwadratu glifu (jednostki świata).
    pub position: Vec2,
    /// Rozmiar kwadratu glifu (jednostki świata).
    pub size: Vec2,
}

/// Metryki czcionki przeliczone na jednostki świata.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMetrics {
    /// Wysokość linii (ascent - descent + line gap).
    pub line_height: f32,
    /// Względem linii bazowej.
    pub ascent: f32,
    pub descent: f32,
}

/// Rejestr czcionek: trzyma atlasy i składa tekst w sprite'y.
pub struct FontRegistry {
    fonts: Vec<Option<FontEntry>>,
    raster_size: u32,
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FontRegistry {
    /// Domyślny rozmiar rasteryzacji. 48 px to dobry kompromis jakość/miejsce
    /// dla typowych rozmiarów czcionek 12–32 px.
    pub const DEFAULT_RASTER_SIZE: u32 = 48;

    pub fn new() -> Self {
        Self {
            fonts: Vec::new(),
            raster_size: Self::DEFAULT_RASTER_SIZE,
        }
    }

    /// Zmiana skali rasteryzacji unieważnia atlasy.
    pub fn set_raster_size(&mut self, size: u32) {
        let size = size.clamp(8, 96);
        if size != self.raster_size {
            self.raster_size = size;
            self.fonts.clear();
        }
    }

    pub fn raster_size(&self) -> u32 {
        self.raster_size
    }

    /// Wczytuje czcionkę (idempotentne — powtórne wywołanie nic nie robi).
    pub fn load(&mut self, handle: Handle<FontData>, data: &[u8]) -> Result<(), InvalidFont> {
        let index = handle.index() as usize;
        if self.fonts.len() <= index {
            self.fonts.resize_with(index + 1, || None);
        }
        if self.fonts[index].is_some() {
            return Ok(());
        }
        self.fonts[index] = Some(FontEntry::new(data, self.raster_size)?);
        Ok(())
    }

    /// Czy atlas czcionki zmienił się od ostatniego wgrania na GPU.
    pub fn is_dirty(&self, handle: HandleId) -> bool {
        self.fonts
            .get(handle.index() as usize)
            .and_then(|f| f.as_ref())
            .map(|f| f.dirty)
            .unwrap_or(false)
    }

    pub fn mark_clean(&mut self, handle: HandleId) {
        if let Some(Some(entry)) = self.fonts.get_mut(handle.index() as usize) {
            entry.dirty = false;
        }
    }

    /// Bajty atlasu do wgrania na GPU (R8).
    pub fn atlas_data(&self, handle: HandleId) -> Option<&[u8]> {
        self.fonts
            .get(handle.index() as usize)
            .and_then(|f| f.as_ref())
            .map(|f| f.pixels.as_slice())
    }

    /// Metryki czcionki dla zadanego rozmiaru tekstu.
    pub fn metrics(&self, handle: HandleId, size: f32) -> FontMetrics {
        let Some(entry) = self
            .fonts
            .get(handle.index() as usize)
            .and_then(|f| f.as_ref())
        else {
            return FontMetrics {
                line_height: size,
                ascent: size,
                descent: 0.0,
            };
        };
        let s = size / entry.units_per_em;
        FontMetrics {
            line_height: (entry.ascent - entry.descent + entry.line_gap) * s,
            ascent: entry.ascent * s,
            descent: entry.descent * s,
        }
    }

    /// Szerokość tekstu w jednostkach świata (bez łamania wierszy).
    pub fn measure(&mut self, handle: HandleId, text: &str, size: f32) -> Vec2 {
        let Some(entry) = self
            .fonts
            .get_mut(handle.index() as usize)
            .and_then(|f| f.as_mut())
        else {
            return Vec2::ZERO;
        };
        let scale = size / entry.units_per_em;
        let mut width = 0.0_f32;
        let mut previous = GlyphId::default();
        for c in text.chars().take(MAX_TEXT_CHARS) {
            let id = entry.font.glyph_id(c);
            width += (entry.font.h_advance_unscaled(id) + entry.font.kern_unscaled(previous, id))
                * scale;
            previous = id;
        }
        Vec2::new(width, self.metrics(handle, size).line_height)
    }

    /// Składa tekst i wypisuje gotowe sprite'y do `out`.
    ///
    /// Zajmuje się kerningiem, łamaniem wierszy, wyrównaniem i wielkością
    /// (`cmd.size` to wysokość `em` w jednostkach świata).
    pub fn layout(&mut self, cmd: &TextDraw, transform: Mat3, out: &mut Vec<SpriteDraw>) {
        let key = HandleId::from(cmd.font);
        let Some(entry) = self
            .fonts
            .get_mut(key.index() as usize)
            .and_then(|f| f.as_mut())
        else {
            return;
        };

        let world_per_unit = cmd.size / entry.units_per_em;
        let line_height = (entry.ascent - entry.descent + entry.line_gap) * world_per_unit;
        let ascent = entry.ascent * world_per_unit;

        // --- 1) ułożenie: identyfikatory glifów, posuwy i podział na wiersze
        let mut lines: Vec<(usize, usize, f32)> = Vec::new();
        let mut line_start = 0usize;
        let mut pen = 0.0f32;
        let mut previous = GlyphId::default();
        let mut word_start = 0usize;
        let mut space_width = 0.0f32;
        let mut index = 0usize;

        for c in cmd.text.chars().take(MAX_TEXT_CHARS) {
            let id = entry.font.glyph_id(c);
            if c == '\n' {
                lines.push((line_start, index, pen));
                line_start = index + 1;
                pen = 0.0;
                previous = GlyphId::default();
                index += 1;
                continue;
            }
            let kern = entry.font.kern_unscaled(previous, id);
            let advance = (entry.font.h_advance_unscaled(id) + kern) * world_per_unit;

            if c == ' ' {
                word_start = index;
                space_width = advance;
            } else if let Some(max_w) = cmd.max_width {
                // łamanie wierszy po spacji, gdy przekroczymy `max_width`
                if pen + advance > max_w && index > word_start {
                    lines.push((line_start, word_start, pen - space_width));
                    line_start = word_start;
                    pen = 0.0;
                    previous = GlyphId::default();
                }
            }

            pen += advance;
            previous = id;
            index += 1;
        }
        lines.push((line_start, index, pen));

        // --- 2) rasteryzacja i produkcja sprite'ów
        let atlas = ATLAS_SIZE as f32;
        for (line_no, (start, end, width)) in lines.into_iter().enumerate() {
            let shift = match cmd.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => -width * 0.5,
                TextAlign::Right => -width,
            };
            let baseline = -ascent - line_no as f32 * line_height * cmd.line_height;
            let mut pen = 0.0f32;
            let mut previous = GlyphId::default();

            for c in cmd.text.chars().take(end).skip(start) {
                let id = entry.font.glyph_id(c);
                let kern = entry.font.kern_unscaled(previous, id);
                let advance = (entry.font.h_advance_unscaled(id) + kern) * world_per_unit;

                if c != ' ' {
                    if let Some(glyph) = entry.rasterize(id) {
                        if glyph.size.x > 0.0 && glyph.size.y > 0.0 {
                            let uv = UvRect::new(
                                Vec2::new(glyph.rect.min.x / atlas, glyph.rect.min.y / atlas),
                                Vec2::new(glyph.rect.max.x / atlas, glyph.rect.max.y / atlas),
                            );
                            // Środek glifu: posuw + przesunięcie od początku
                            // linii bazowej, wszystko w jednostkach świata.
                            //
                            // `GlyphInfo::size/offset` to UŁAMKI `em`, więc
                            // mnożymy je wprost przez `cmd.size`. Posuwy
                            // (advance) pochodzą z jednostek fontu, stąd
                            // osobny przelicznik `world_per_unit`.
                            let size = glyph.size * cmd.size;
                            let local = Vec2::new(
                                shift + pen + glyph.offset.x * cmd.size,
                                baseline - glyph.offset.y * cmd.size,
                            );
                            out.push(SpriteDraw {
                                transform: transform
                                    * Mat3::from_translation(
                                        local + Vec2::new(size.x, -size.y) * 0.5,
                                    )
                                    * Mat3::from_scale(size),
                                uv,
                                color: cmd.color,
                                texture: TextureKey::FontAtlas(key),
                                blend: cmd.blend,
                                layer: cmd.layer,
                                screen_space: cmd.screen_space,
                            });
                        }
                    }
                }
                // UWAGA: posuw PRZESUWAMY DOPIERO PO narysowaniu glifu.
                //
                // `pen` to pozycja, w której stoi BIEŻĄCY glif — tak jak
                // `pen` w `measure` i w fazie układania. Robienie `pen +=`
                // przed rysowaniem przesuwało każdą literę o jej własny
                // advance, więc pierwsza litera leciała o `advance` za daleko
                // i litery nachodziły na siebie ("Dzien" => "Dżen", nachodzące
                // na siebie "z" i "D"), a tekst z biegiem czasu rozjeżdżał się
                // coraz bardziej.
                pen += advance;
                previous = id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::Globals;
    use crate::Camera2d;
    use uran_ecs::BlendMode;
    use uran_math::Color;

    /// Czcionka systemowa — dzięki temu testy nie potrzebują plików w repo.
    fn system_font() -> Option<Vec<u8>> {
        [
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/TTF/Hack-Regular.ttf",
            "/usr/share/fonts/TTF/MesloLGS-NF-Regular.ttf",
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
    }

    fn text_cmd(font: Handle<FontData>, text: &str) -> TextDraw {
        TextDraw {
            font,
            text: text.into(),
            position: Vec2::ZERO,
            size: 24.0,
            color: Color::WHITE,
            layer: 0,
            blend: BlendMode::Alpha,
            align: TextAlign::Left,
            line_height: 1.0,
            max_width: None,
            screen_space: false,
        }
    }

    #[test]
    fn sprite_reaches_expected_pixels_end_to_end() {
        // Regresja dla całego łańcucha: kamera -> Globals -> jednostkowy quad.
        // Sprite o szerokości 1800 jednostek świata przy kamerze dopasowanej
        // do okna 1072 px MUSI zająć całą szerokość okna.
        let window = Vec2::new(1072.0, 756.0);
        let arena = Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0));
        let mut camera = Camera2d::new();
        camera.fit_world(arena, window);

        let globals = Globals::new(camera.view_projection(window));
        // prostokąt wypełniający arenę, przekształcony jak w SpriteDraw::rect
        let transform = Mat3::from_translation(arena.center()) * Mat3::from_scale(arena.size());
        // lewy dolny róg jednostkowego quada (-0.5, -0.5) i prawy górny (0.5, 0.5)
        let bl =
            globals.transform_point((transform * Vec2::new(-0.5, -0.5).extend(1.0)).truncate());
        let tr = globals.transform_point((transform * Vec2::new(0.5, 0.5).extend(1.0)).truncate());

        // szerokość w NDC pomnożona przez połowę okna daje piksele
        let width_px = (tr.x - bl.x).abs() * window.x * 0.5;
        assert!(
            (width_px - window.x).abs() < 1.0,
            "arena zajęła {width_px} px zamiast {} px (NDC {bl:?}..{tr:?})",
            window.x
        );
    }

    /// Ile pikseli bitmapy glifu jest wypełnionych.
    fn coverage(entry: &mut FontEntry, c: char) -> (u32, u32, usize) {
        let id = entry.font.glyph_id(c);
        let g = entry.rasterize(id).expect("brak konturu");
        let (w, h) = (g.rect.width() as u32, g.rect.height() as u32);
        let atlas = ATLAS_SIZE as usize;
        let mut filled = 0;
        for row in 0..h {
            let start = ((g.rect.min.y as usize) + row as usize) * atlas + g.rect.min.x as usize;
            let slice = &entry.pixels[start..start + w as usize];
            filled += slice.iter().filter(|&&v| v > 32).count();
        }
        (w, h, filled)
    }

    #[test]
    fn rasterized_glyph_is_actually_drawn() {
        // Regresja: `Outlined::draw` już zwraca współrzędne względem
        // początku bitmapi. Odejmowanie `bounds.min` drugi raz sprawiało,
        // że litera wychodziła jako wąski pasek u góry.
        let Some(data) = system_font() else { return };
        let handle: Handle<FontData> = Handle::new(0, 0);
        let mut registry = FontRegistry::new();
        registry.load(handle, &data).unwrap();
        let entry = registry.fonts.get_mut(0).and_then(|f| f.as_mut()).unwrap();

        for c in ['M', 'a', 'g'] {
            let (w, h, filled) = coverage(entry, c);
            let ratio = filled as f32 / (w * h) as f32;
            assert!(w > 10 && h > 10, "'{c}': bitmap {w}x{h} jest za mały");
            assert!(
                ratio > 0.05,
                "'{c}': tylko {filled}/{w}x{h} pikseli ({:.1}%) — kontur nie trafił do bitmapy",
                ratio * 100.0
            );
        }
    }

    #[test]
    fn glyphs_start_at_pen_and_follow_advances() {
        // Regresja: posuw `pen` musi wskazywac pozycje BIEZACEGO glifu.
        //
        // Wczesniej `pen += advance` szlo PRZED rysowaniem, wiec kazda litera
        // ladowala o swoj wlasny advance za daleko: pierwsza litera tekstu
        // zaczynala sie zamiast w `shift`, a kolejne nakladaly sie na siebie
        // ("Dzien" wygladalo jak "Dzen" z plamiastymi literami).
        //
        // Sprawdzamy to niezaleznie od czcionki, na SAMEJ metryce fontu:
        //   * lewa krawedz pierwszego glifu == jego `offset.x`,
        //   * odleglosc miedzy sasiednimi glifami == `advance` poprzedniego,
        //   * ostatni glif konczy sie wylacznie przed `measure()`.
        let Some(data) = system_font() else { return };
        let handle: Handle<FontData> = Handle::new(0, 0);
        let mut registry = FontRegistry::new();
        registry.load(handle, &data).unwrap();

        let text = "Dzien 1";
        let size = 15.0f32;
        let mut out = Vec::new();
        let mut cmd = text_cmd(handle, text);
        cmd.size = size;
        registry.layout(&cmd, Mat3::IDENTITY, &mut out);
        assert!(!out.is_empty(), "layout nie wyprodukowal glifow");

        // Lewa krawedz i szerokosc kazdego quada w jednostkach swiata.
        let edges: Vec<(f32, f32)> = out
            .iter()
            .map(|s| {
                let c = s.transform.to_cols_array_2d();
                let w = c[0][0].abs();
                (c[2][0] - w * 0.5, c[2][0] + w * 0.5)
            })
            .collect();

        // 1) pierwszy glif zaczyna sie w `shift` (= 0 dla Left), a NIE
        //    o jeden advance dalej.
        let first_offset = {
            let e = registry.fonts[0].as_mut().unwrap();
            let g = e.rasterize(e.font.glyph_id('D')).unwrap();
            g.offset.x * size
        };
        assert!(
            (edges[0].0 - first_offset).abs() < 0.01,
            "pierwszy glif zaczyna sie w {:.3}, oczekiwano {:.3} (offset.x * size)",
            edges[0].0,
            first_offset
        );

        // 2) sasiednie glify NIE zachodza na siebie: kazda prawa krawedz
        //    lezy w najwyzej na lewej krawedzi nastepnego (+ tolerancja na
        //    kerning i na kolidujace bounding boxy w bitmape).
        for w in edges.windows(2) {
            assert!(
                w[0].1 <= w[1].0 + 0.5,
                "glify nachodza na siebie: prawa krawedz {:.3} > lewa nastepnego {:.3}",
                w[0].1,
                w[1].0
            );
        }

        // 3) ostatni glif konczy sie w `measure()` — calkowita szerokosc
        //    tekstu zgadza sie z suma posuwow (tu `shift` = 0).
        let measured = registry.measure(HandleId::from(handle), text, size).x;
        let last = *edges.last().unwrap();
        assert!(
            last.1 <= measured + 0.5,
            "tekst wider niz `measure()`: koniec {:.3} > {:.3}",
            last.1,
            measured
        );
        // `measure` liczy posuwy w jednostkach fontu przeliczonych przez
        // `size / upem` — sprawdzamy, ze ta szerokosc jest wogole sensowna
        // (kiedys `pen` byl liczony w surowych jednostkach fontu i tekst
        // rozjeżdżal sie o czynnik `upem`).
        assert!(
            (10.0..400.0).contains(&measured),
            "`measure` zwrocilo bledna szerokosc {measured} dla rozmiaru {size}"
        );
    }

    #[test]
    fn glyph_quad_has_requested_size() {
        // Glif 'M' przy `size` jednostek świata musi dać kwadrat o boku
        // zbliżonym do `size` (a nie `size * raster_size / upem`).
        let Some(data) = system_font() else { return };
        let handle: Handle<FontData> = Handle::new(0, 0);
        let mut registry = FontRegistry::new();
        registry.load(handle, &data).unwrap();
        let Some(entry) = registry.fonts.get_mut(0).and_then(|f| f.as_mut()) else {
            panic!("brak wpisu czcionki");
        };
        let id = entry.font.glyph_id('M');
        let raster = entry.raster_size;
        let g = entry.rasterize(id).expect("brak konturu glifu");
        // regresja: przy em = `raster` px litera 'M' musi mieć kilkadziesiąt
        // pikseli, a nie 1x1 (pomyłka z interpretacją PxScale)
        let px = g.size * raster as f32;
        assert!(
            px.x > 4.0 && px.y > 4.0,
            "glif 'M' ma tylko {px:?} px przy em = {raster} px"
        );
        assert!(px.x <= raster as f32 && px.y <= raster as f32);
        eprintln!(
            "[test] 'M': atlas rect={:?} ({}x{} px) offset={:?} em-size={:?} => {px:?} px",
            g.rect,
            g.rect.width(),
            g.rect.height(),
            g.offset,
            g.size
        );

        let size = 32.0f32;
        let mut out = Vec::new();
        let mut cmd = text_cmd(handle, "M");
        cmd.size = size;
        registry.layout(&cmd, Mat3::IDENTITY, &mut out);

        let glyph = out.first().expect("brak glifu");
        let c = glyph.transform.to_cols_array_2d();
        eprintln!(
            "[test] quad 'M' w jednostkach świata: x={:?} y={:?} pos={:?}",
            c[0][0], c[1][1], c[2]
        );
        eprintln!("[test] uv = {:?}", glyph.uv);
        // jednostkowy quad (-0.5..0.5) pomnożony przez macierz daje szerokość
        // równą skali kolumny X; wielkość 'M' to ok. 0.7 em
        assert!(
            c[0][0] > size * 0.3 && c[0][0] < size * 1.2,
            "szerokość glifu {:?}, oczekiwano ~{size}",
            c[0][0]
        );
        assert!(c[1][1] > 0.0, "glif jest spłaszczony: {:?}", c[1][1]);
    }

    #[test]
    fn layout_produces_glyphs() {
        let Some(data) = system_font() else {
            eprintln!("brak czcionki systemowej — pomijam test");
            return;
        };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();

        let mut out = Vec::new();
        registry.layout(&text_cmd(handle, "Uran 2D"), Mat3::IDENTITY, &mut out);

        // Diagnostyka: które znaki nie wygenerowały sprite'a?
        let visible: Vec<char> = "Uran 2D".chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(
            out.len(),
            visible.len(),
            "wygenerowano {} sprite'ów dla {} znaków: {visible:?}",
            out.len(),
            visible.len()
        );
        assert!(out
            .iter()
            .all(|s| s.texture == TextureKey::FontAtlas(HandleId::from(handle))));
        assert!(out.iter().all(|s| s.uv.min.x >= 0.0 && s.uv.max.x <= 1.0));
    }

    #[test]
    fn measure_is_monotonic() {
        let Some(data) = system_font() else { return };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();
        let key = HandleId::from(handle);

        let short = registry.measure(key, "ab", 24.0);
        let long = registry.measure(key, "abcdef", 24.0);
        assert!(long.x > short.x, "dłuższy tekst musi być szerszy");
        assert!(short.x > 0.0);
        assert!(short.y > 0.0, "metryki muszą zwrócić wysokość linii");

        // dwa razy większy rozmiar = dwa razy szerszy
        let bigger = registry.measure(key, "ab", 48.0);
        assert!((bigger.x - short.x * 2.0).abs() < 0.01);
    }

    #[test]
    fn alignment_shifts_line() {
        let Some(data) = system_font() else { return };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();
        let base = text_cmd(handle, "abcd");

        let mut left = Vec::new();
        registry.layout(&base, Mat3::IDENTITY, &mut left);
        let mut center = Vec::new();
        registry.layout(
            &TextDraw {
                align: TextAlign::Center,
                ..base.clone()
            },
            Mat3::IDENTITY,
            &mut center,
        );

        assert!(
            (center[0].transform.to_cols_array_2d()[2][0])
                < (left[0].transform.to_cols_array_2d()[2][0])
        );
    }

    #[test]
    fn newline_splits_lines_downwards() {
        let Some(data) = system_font() else { return };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();

        let mut out = Vec::new();
        registry.layout(&text_cmd(handle, "ab\nab"), Mat3::IDENTITY, &mut out);
        assert_eq!(out.len(), 4);
        assert!(
            (out[2].transform.to_cols_array_2d()[2][1])
                < (out[0].transform.to_cols_array_2d()[2][1])
        );
    }

    #[test]
    fn max_width_wraps_text() {
        let Some(data) = system_font() else { return };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();

        let narrow_cmd = TextDraw {
            max_width: Some(50.0),
            ..text_cmd(handle, "aaa bbb ccc")
        };
        let wide_cmd = TextDraw {
            max_width: Some(10_000.0),
            ..text_cmd(handle, "aaa bbb ccc")
        };
        let mut narrow = Vec::new();
        let mut wide = Vec::new();
        registry.layout(&narrow_cmd, Mat3::IDENTITY, &mut narrow);
        registry.layout(&wide_cmd, Mat3::IDENTITY, &mut wide);

        // zawijanie nie usuwa glifów, tylko przesuwa je na kolejne wiersze
        assert_eq!(narrow.len(), wide.len());
        assert!(
            (narrow.last().unwrap().transform.to_cols_array_2d()[2][1])
                < (wide.last().unwrap().transform.to_cols_array_2d()[2][1])
        );
    }

    #[test]
    fn atlas_is_dirty_only_after_rasterization() {
        let Some(data) = system_font() else { return };
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(0, 0);
        registry.load(handle, &data).unwrap();
        let key = HandleId::from(handle);

        registry.mark_clean(key);
        assert!(!registry.is_dirty(key));

        let mut out = Vec::new();
        registry.layout(&text_cmd(handle, "abc"), Mat3::IDENTITY, &mut out);
        assert!(registry.is_dirty(key));

        registry.mark_clean(key);
        assert!(!registry.is_dirty(key));
        assert_eq!(
            registry.atlas_data(key).unwrap().len(),
            (ATLAS_SIZE * ATLAS_SIZE) as usize
        );
    }

    #[test]
    fn unknown_font_is_ignored() {
        let mut registry = FontRegistry::new();
        let handle: Handle<FontData> = Handle::new(7, 0);
        let mut out = Vec::new();
        registry.layout(&text_cmd(handle, "abc"), Mat3::IDENTITY, &mut out);
        assert!(out.is_empty());
        assert_eq!(
            registry.measure(HandleId::from(handle), "abc", 24.0),
            Vec2::ZERO
        );
    }
}
