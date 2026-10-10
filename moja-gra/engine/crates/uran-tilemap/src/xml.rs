//! Wczytywanie i zapisywanie mapy w XML.
//!
//! Format pliku `*.xml` jest celowo **czytelny dla człowieka**: mapa ma
//! rozmiar, listę arkuszy kafli, warstwy z danymi w wierszach oraz obszar
//! edycji. Dzięki temu nową mapę da się napisać albo zmienić w edytorze
//! tekstowym, bez programowania.
//!
//! ```xml
//! <map name="farma" tile-size="16" width="32" height="24">
//!   <spawn x="8" y="6"/>
//!   <edit-area x="4" y="4" width="12" height="10"/>
//!   <tileset name="terrain" src="tilesets/terrain.png" tile-size="16"/>
//!
//!   <layer name="ground" tileset="terrain" editable="true">
//!     <row y="0">1 1 1 1 1 1 1 1</row>
//!   </layer>
//!
//!   <layer name="walls" tileset="terrain" solid="true">
//!     <props x="3" y="3">solid</props>
//!   </layer>
//! </map>
//! ```
//!
//! ## Kolejność elementów
//!
//! `row y="0"` to wiersz **najbliższy do dołu** (oś Y w górę, jak w całym
//! silniku). Kolejność `<row>` w pliku nie ma znaczenia — ważny jest atrybut
//! `y`, więc fragmenty mapy można wstawiać i przestawiać bez pomyłek.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::Reader;
use uran_asset::AssetServer;
use uran_math::{Rect, Vec2};

use crate::autotile::{AutoTile, MatchMode};
use crate::layer::{index_from_packed, TILE_EMPTY};
use crate::map::{Layer, MapError, TileMap, TileProps};

/// Opis pojedynczej warstwy w trakcie parsowania.
struct LayerBuilder {
    name: String,
    tileset: String,
    solid: bool,
    editable: bool,
    hidden: bool,
    /// Opcjonalny rozmiar warstwy; `None` = rozmiar mapy.
    size: Option<(u32, u32)>,
    rows: BTreeMap<i32, Vec<u32>>,
    props: Vec<(i32, i32, TileProps)>,
    autotile: Option<AutoTile>,
}

/// Zamienia zawartość zdarzenia tekstowego na `String`.
///
/// `quick-xml` zwraca surowe bajty; `decode()` obsługuje encje (`&amp;`,
/// `&lt;`) i zwraca błąd dla niepoprawnej treści.
fn text_of(e: &BytesText<'_>, path: &str) -> Result<String, MapError> {
    e.decode()
        .map(|s| s.into_owned())
        .map_err(|err| MapError::Xml {
            path: path.into(),
            message: err.to_string(),
        })
}

/// Atrybut elementu jako tekst (`None`, gdy go nie ma).
fn attr(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<String>, MapError> {
    for a in e.attributes() {
        let a = a.map_err(|err| MapError::Xml {
            path: path.into(),
            message: err.to_string(),
        })?;
        if a.key.as_ref() == name.as_bytes() {
            // Atrybuty bywają z encjami (`&amp;`), więc dekodujemy i rozwikłujemy.
            let value = a
                .decode_and_unescape_value(decoder)
                .map_err(|err| MapError::Xml {
                    path: path.into(),
                    message: err.to_string(),
                })?;
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

/// Atrybut jako `u32`.
fn attr_u32(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<u32>, MapError> {
    match attr(e, path, name, decoder)? {
        None => Ok(None),
        Some(v) => v
            .trim()
            .parse::<u32>()
            .map(Some)
            .map_err(|_| MapError::Number {
                path: path.into(),
                field: name.into(),
                value: v,
            }),
    }
}

/// Atrybut jako `i32` (współrzędne mogą być ujemne).
fn attr_i32(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<i32>, MapError> {
    match attr(e, path, name, decoder)? {
        None => Ok(None),
        Some(v) => v
            .trim()
            .parse::<i32>()
            .map(Some)
            .map_err(|_| MapError::Number {
                path: path.into(),
                field: name.into(),
                value: v,
            }),
    }
}

/// Atrybut jako `f32` (rozmiary i współrzędne świata).
fn attr_f32(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<f32>, MapError> {
    match attr(e, path, name, decoder)? {
        None => Ok(None),
        Some(v) => v
            .trim()
            .parse::<f32>()
            .map(Some)
            .map_err(|_| MapError::Number {
                path: path.into(),
                field: name.into(),
                value: v,
            }),
    }
}

/// Atrybut logiczny: `true`/`1`/`yes` to prawda.
fn attr_bool(e: &BytesStart<'_>, name: &str, decoder: quick_xml::encoding::Decoder) -> bool {
    match attr(e, "", name, decoder) {
        Ok(Some(v)) => matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => false,
    }
}

/// Parsuje listę liczb oddzielonych dowolnymi białymi znakami.
///
/// Akceptujemy też przecinki i średniki, bo mapa pisana ręcznie w notatniku
/// łatwo dostaje przecinek „na oku". Kropka `.` oznacza **pusty kafel**
/// (`TILE_EMPTY`) — jest czytelniejsza niż `4294967295`, a w edytorze tekstu
/// od razu widać, że to „nic tu nie ma".
fn parse_numbers(text: &str, path: &str) -> Result<Vec<u32>, MapError> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s == "." || s == "-" {
                return Ok(TILE_EMPTY);
            }
            s.parse::<u32>().map_err(|_| MapError::Number {
                path: path.into(),
                field: "wiersz kafli".into(),
                value: s.to_string(),
            })
        })
        .collect()
}

/// Parsuje tekst XML w mapę (bez wczytywania tekstur).
///
/// Rozdzielenie parsowania od wczytywania obrazów pozwala testować format
/// bez plików PNG i bez GPU.
pub fn parse_map(xml: &str, path: &str) -> Result<TileMap, MapError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut name = String::new();
    let mut tile_size = 32.0f32;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut spawn: Option<Vec2> = None;
    let mut edit_area: Option<Rect> = None;
    let mut layers: Vec<LayerBuilder> = Vec::new();
    let mut pending: Option<LayerBuilder> = None;
    let mut row_y: Option<i32> = None;
    let mut row_values: Vec<u32> = Vec::new();
    // Współrzędne odczytane z `<props x=.. y=..>`, używane przy następnym `Text`.
    let mut pending_props_x: Option<i32> = None;
    let mut pending_props_y: Option<i32> = None;

    loop {
        let event = reader.read_event().map_err(|e| MapError::Xml {
            path: path.into(),
            message: e.to_string(),
        })?;
        match event {
            // `Start` i `Empty` obsługujemy tak samo: element `<spawn x="1"/>`
            // jest w dokumencie samozamykający, więc parser nigdy nie zobaczy
            // dla niego osobnego `End` — a bez tego mapa gubiłaby punkt startowy
            // i obszar edycji przy każdym zapisie w edytorze.
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_self_closed = matches!(event, Event::Empty(_));
                let dec = reader.decoder();
                let element = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match element.as_str() {
                    "map" => {
                        name = attr(e, path, "name", dec)?.unwrap_or_else(|| "mapa".to_string());
                        tile_size = attr_f32(e, path, "tile-size", dec)?.unwrap_or(32.0);
                        width = attr_u32(e, path, "width", dec)?.unwrap_or(0);
                        height = attr_u32(e, path, "height", dec)?.unwrap_or(0);
                    }
                    "spawn" => {
                        let x = attr_f32(e, path, "x", dec)?.unwrap_or(0.0);
                        let y = attr_f32(e, path, "y", dec)?.unwrap_or(0.0);
                        spawn = Some(Vec2::new(x, y));
                    }
                    "edit-area" => {
                        // `x`/`y`/`width`/`height` są w jednostkach świata,
                        // a wariant `tiles="true"` podaje je w kaflach — przy
                        // ręcznym pisaniu mapy liczenie pikseli bywa kłopotliwe.
                        let in_tiles = attr_bool(e, "tiles", dec);
                        let scale = if in_tiles { tile_size } else { 1.0 };
                        let x = attr_f32(e, path, "x", dec)?.unwrap_or(0.0) * scale;
                        let y = attr_f32(e, path, "y", dec)?.unwrap_or(0.0) * scale;
                        let w = attr_f32(e, path, "width", dec)?.unwrap_or(0.0) * scale;
                        let h = attr_f32(e, path, "height", dec)?.unwrap_or(0.0) * scale;
                        edit_area = Some(Rect::from_xywh(x, y, w, h));
                    }
                    "layer" => {
                        let l = LayerBuilder {
                            name: attr(e, path, "name", dec)?.unwrap_or_else(|| "layer".into()),
                            tileset: attr(e, path, "tileset", dec)?.ok_or_else(|| {
                                MapError::Format {
                                    path: path.into(),
                                    message: "warstwa bez atrybutu `tileset`".into(),
                                }
                            })?,
                            solid: attr_bool(e, "solid", dec),
                            editable: attr_bool(e, "editable", dec),
                            hidden: attr_bool(e, "hidden", dec),
                            // Warstwa może mieć własny rozmiar (np. mniejsza
                            // mapa dekoracji); brak atrybutów = rozmiar mapy.
                            size: Some((
                                attr_u32(e, path, "width", dec)?.unwrap_or(width),
                                attr_u32(e, path, "height", dec)?.unwrap_or(height),
                            )),
                            rows: BTreeMap::new(),
                            props: Vec::new(),
                            autotile: None,
                        };
                        pending = Some(l);
                        // Warstwa zapisana jako `<layer .../>` nie dostanie
                        // zdarzenia `End`, więc zamykamy ją od razu.
                        if is_self_closed {
                            if let Some(l) = pending.take() {
                                layers.push(l);
                            }
                        }
                    }
                    "row" => {
                        row_y = Some(attr_i32(e, path, "y", dec)?.unwrap_or(0));
                        row_values.clear();
                    }
                    "props" => {
                        // Współrzędne zapamiętujemy, treść doczytamy w `Text`.
                        pending_props_x = attr_i32(e, path, "x", dec)?;
                        pending_props_y = attr_i32(e, path, "y", dec)?;
                    }
                    "autotile" => {
                        if let Some(l) = pending.as_mut() {
                            let base = attr_u32(e, path, "base", dec)?.unwrap_or(0);
                            let mode = match attr(e, path, "mode", dec)?.as_deref() {
                                Some("presence") => MatchMode::NonEmpty,
                                _ => MatchMode::Exact,
                            };
                            l.autotile = Some(AutoTile {
                                base,
                                include_self: !attr_bool(e, "exclude-self", dec),
                                mode,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref e) => {
                if row_y.is_some() {
                    row_values.extend(parse_numbers(&text_of(e, path)?, path)?);
                } else if let (Some(l), Some(px), Some(py)) =
                    (pending.as_mut(), pending_props_x, pending_props_y)
                {
                    // Tekst poza `<row>`, ale we współrzędnych `<props>`.
                    l.props
                        .push((px, py, TileProps::parse_list(&text_of(e, path)?)));
                    pending_props_x = None;
                    pending_props_y = None;
                }
            }
            Event::End(ref e) => {
                let element = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match element.as_str() {
                    "row" => {
                        if let (Some(l), Some(y)) = (pending.as_mut(), row_y) {
                            l.rows.insert(y, std::mem::take(&mut row_values));
                        }
                        row_y = None;
                    }
                    "layer" => {
                        if let Some(l) = pending.take() {
                            layers.push(l);
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    build_map(
        name, tile_size, width, height, layers, spawn, edit_area, path,
    )
}

/// Składa z zebranych danych gotową [`TileMap`].
#[allow(clippy::too_many_arguments)]
fn build_map(
    name: String,
    tile_size: f32,
    width: u32,
    height: u32,
    layers: Vec<LayerBuilder>,
    spawn: Option<Vec2>,
    edit_area: Option<Rect>,
    path: &str,
) -> Result<TileMap, MapError> {
    if width == 0 || height == 0 {
        return Err(MapError::Format {
            path: path.into(),
            message: format!("mapa musi mieć niezerowy rozmiar, jest {width}x{height}"),
        });
    }
    if tile_size <= 0.0 {
        return Err(MapError::Format {
            path: path.into(),
            message: format!("`tile-size` musi być dodatni, jest {tile_size}"),
        });
    }

    let mut map = TileMap::new(name, tile_size, width, height);
    map.spawn = spawn;
    map.set_edit_area(edit_area);

    for lb in layers {
        // Warstwa może mieć mniejszy rozmiar niż mapa (np. sama dekoracja),
        // wtedy nie wychodzi poza własną siatkę.
        let (lw, lh) = lb.size.unwrap_or((width, height));
        let mut layer = Layer::new(&lb.name, &lb.tileset, lw, lh);
        layer.solid = lb.solid;
        layer.editable = lb.editable;
        layer.hidden = lb.hidden;

        // Wypełniamy warstwę wierszami od dołu; brakujące wiersze zostają puste.
        for (y, values) in &lb.rows {
            if *y < 0 || *y >= lh as i32 {
                // Wiersz poza warstwą to błąd danych, ale nie wywracamy gry —
                // pomijamy go i mapa reszty wczytuje się normalnie.
                continue;
            }
            for (x, v) in values.iter().enumerate() {
                let x = x as i32;
                if x >= lw as i32 {
                    break;
                }
                if *v != TILE_EMPTY && *v != u32::MAX {
                    layer.tiles.set(x, *y, *v);
                }
            }
        }

        let idx = map.add_layer(layer);
        for (x, y, props) in lb.props {
            map.set_props(idx, x, y, props);
        }
        if let Some(at) = lb.autotile {
            map.set_autotile(idx, at);
        }
    }

    Ok(map)
}

/// Wczytuje mapę z pliku XML i rejestruje arkusze kafli w `assets`.
///
/// Ścieżki `src` w `<tileset>` są **relatywne do katalogu assetów**, nie do
/// pliku mapy — dzięki temu ta sama mapa działa z każdego katalogu gry.
pub fn load_map(path: &str, assets: &mut AssetServer) -> Result<TileMap, MapError> {
    let full = assets.resolve(std::path::Path::new(path));
    let xml = std::fs::read_to_string(&full).map_err(|source| MapError::Io {
        path: path.to_string(),
        source,
    })?;

    let mut map = parse_map(&xml, path)?;
    for (name, spec) in read_tilesets(&xml, path)? {
        map.add_tileset_from_image(
            &name,
            assets,
            &spec.src,
            spec.tile_size,
            spec.margin,
            spec.spacing,
        )?;
    }
    Ok(map)
}

/// Opis arkusza kafli zadeklarowanego w XML.
struct TilesetSpec {
    src: String,
    tile_size: u32,
    margin: u32,
    spacing: u32,
}

/// Odczytuje deklaracje `<tileset>` (osobny przebieg, bo wymagają I/O).
fn read_tilesets(xml: &str, path: &str) -> Result<Vec<(String, TilesetSpec)>, MapError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| MapError::Xml {
            path: path.into(),
            message: e.to_string(),
        })?;
        match event {
            Event::Empty(ref e) | Event::Start(ref e) => {
                if e.name().as_ref() == b"tileset" {
                    let dec = reader.decoder();
                    let name = attr(e, path, "name", dec)?.ok_or_else(|| MapError::Format {
                        path: path.into(),
                        message: "<tileset> bez atrybutu `name`".into(),
                    })?;
                    let src = attr(e, path, "src", dec)?.ok_or_else(|| MapError::Format {
                        path: path.into(),
                        message: format!("<tileset name=\"{name}\"> bez atrybutu `src`"),
                    })?;
                    out.push((
                        name,
                        TilesetSpec {
                            src,
                            tile_size: attr_u32(e, path, "tile-size", dec)?.unwrap_or(16),
                            margin: attr_u32(e, path, "margin", dec)?.unwrap_or(0),
                            spacing: attr_u32(e, path, "spacing", dec)?.unwrap_or(0),
                        },
                    ));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Zamienia znaki specjalne w atrybutach XML, żeby nazwy i ścieżki
/// nie psuły struktury dokumentu.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl TileMap {
    /// Serializuje mapę do XML (format wczytywany przez [`load_map`]).
    ///
    /// Puste kafle zapisujemy jako `.`, a flagi wariantu (flip) nie są
    /// zapisywane — format XML przechowuje sam indeks, tak samo jak
    /// [`parse_map`] przy wczytywaniu.
    pub fn to_xml(&self) -> String {
        use std::fmt::Write;

        let mut out = String::with_capacity(2048);
        let _ = writeln!(
            out,
            "<map name=\"{}\" tile-size=\"{}\" width=\"{}\" height=\"{}\">",
            escape_attr(&self.name),
            self.tile_size,
            self.width(),
            self.height()
        );

        if let Some(spawn) = self.spawn {
            let _ = writeln!(out, "  <spawn x=\"{}\" y=\"{}\"/>", spawn.x, spawn.y);
        }
        if let Some(area) = self.edit_area() {
            let _ = writeln!(
                out,
                "  <edit-area x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                area.min.x,
                area.min.y,
                area.width(),
                area.height()
            );
        }

        for (name, ts) in &self.tilesets {
            let mut line = format!(
                "  <tileset name=\"{}\" src=\"{}\" tile-size=\"{}\"",
                escape_attr(name),
                escape_attr(ts.src()),
                ts.tile_size()
            );
            if ts.margin() != 0 {
                let _ = write!(line, " margin=\"{}\"", ts.margin());
            }
            if ts.spacing() != 0 {
                let _ = write!(line, " spacing=\"{}\"", ts.spacing());
            }
            let _ = writeln!(out, "{line}/>");
        }

        for (i, layer) in self.layers.iter().enumerate() {
            let lw = layer.tiles.width();
            let lh = layer.tiles.height();

            let mut attrs = format!(
                "name=\"{}\" tileset=\"{}\"",
                escape_attr(&layer.tiles.name),
                escape_attr(&layer.tileset)
            );
            if layer.solid {
                attrs.push_str(" solid=\"true\"");
            }
            if layer.editable {
                attrs.push_str(" editable=\"true\"");
            }
            if layer.hidden {
                attrs.push_str(" hidden=\"true\"");
            }
            if lw != self.width() || lh != self.height() {
                let _ = write!(attrs, " width=\"{lw}\" height=\"{lh}\"");
            }

            let has_tiles = (0..lh)
                .any(|y| (0..lw).any(|x| layer.tiles.get(x as i32, y as i32) != TILE_EMPTY));
            let has_props = self.props_map(i).map_or(false, |m| !m.is_empty());
            let autotile = self.autotile(i);

            if !has_tiles && !has_props && autotile.is_none() {
                let _ = writeln!(out, "  <layer {attrs}/>");
                continue;
            }

            let _ = writeln!(out, "  <layer {attrs}>");
            if let Some(at) = autotile {
                let mode = match at.mode {
                    MatchMode::NonEmpty => "presence",
                    MatchMode::Exact => "exact",
                };
                let exclude = if at.include_self {
                    ""
                } else {
                    " exclude-self=\"true\""
                };
                let _ = writeln!(
                    out,
                    "    <autotile base=\"{}\" mode=\"{mode}\"{exclude}/>",
                    at.base
                );
            }
            for y in 0..lh {
                let cells: Vec<String> = (0..lw)
                    .map(|x| {
                        let packed = layer.tiles.get(x as i32, y as i32);
                        if packed == TILE_EMPTY {
                            ".".to_string()
                        } else {
                            index_from_packed(packed).to_string()
                        }
                    })
                    .collect();
                if cells.iter().all(|c| c == ".") {
                    continue;
                }
                let _ = writeln!(out, "    <row y=\"{y}\">{}</row>", cells.join(" "));
            }
            if let Some(props) = self.props_map(i) {
                for ((x, y), p) in props {
                    let names = p.to_names();
                    if names.is_empty() {
                        continue;
                    }
                    let _ = writeln!(out, "    <props x=\"{x}\" y=\"{y}\">{names}</props>");
                }
            }
            let _ = writeln!(out, "  </layer>");
        }

        let _ = writeln!(out, "</map>");
        out
    }

    /// Zapisuje mapę do pliku XML.
    pub fn save_xml(&self, path: &str) -> Result<(), MapError> {
        std::fs::write(path, self.to_xml()).map_err(|source| MapError::Io {
            path: path.to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI: &str = r#"
    <map name="test" tile-size="16" width="4" height="3">
      <spawn x="1" y="2"/>
      <edit-area x="1" y="1" width="2" height="2" tiles="true"/>
      <layer name="ground" tileset="terrain" editable="true">
        <row y="0">1 2 3 4</row>
        <row y="1">5 6 7 8</row>
        <row y="2">9 10 11 12</row>
      </layer>
      <layer name="walls" tileset="terrain" solid="true">
        <row y="1">0 0 0 0</row>
        <props x="2" y="1">solid water</props>
      </layer>
    </map>
    "#;

    #[test]
    fn parses_header_and_size() {
        let m = parse_map(MINI, "test.xml").unwrap();
        assert_eq!(m.name, "test");
        assert_eq!((m.width(), m.height()), (4, 3));
        assert_eq!(m.tile_size, 16.0);
    }

    #[test]
    fn parses_spawn_and_edit_area() {
        let m = parse_map(MINI, "test.xml").unwrap();
        assert_eq!(m.spawn, Some(Vec2::new(1.0, 2.0)));
        let area = m.edit_area().expect("obszar edycji");
        // `tiles="true"` mnoży podane liczby przez rozmiar kafla (16 px):
        // 1..2 kafla to 16..48 w jednostkach świata.
        assert_eq!(area, Rect::from_xywh(16.0, 16.0, 32.0, 32.0));
    }

    #[test]
    fn edit_area_defaults_to_world_units() {
        let xml = r#"<map name="t" tile-size="16" width="4" height="4">
            <edit-area x="0" y="0" width="32" height="32"/>
            </map>"#;
        let m = parse_map(xml, "t.xml").unwrap();
        assert_eq!(m.edit_area(), Some(Rect::from_xywh(0.0, 0.0, 32.0, 32.0)));
    }

    #[test]
    fn rows_land_on_correct_cells() {
        let m = parse_map(MINI, "test.xml").unwrap();
        let ground = m.layer_index("ground").unwrap();
        // y=0 to wiersz najniższy.
        assert_eq!(m.tile_at(ground, 0, 0), 1);
        assert_eq!(m.tile_at(ground, 3, 0), 4);
        assert_eq!(m.tile_at(ground, 0, 2), 9);
        assert_eq!(m.tile_at(ground, 3, 2), 12);
    }

    #[test]
    fn layer_flags_are_read() {
        let m = parse_map(MINI, "test.xml").unwrap();
        let ground = m.layer_index("ground").unwrap();
        let walls = m.layer_index("walls").unwrap();
        assert!(m.layers[ground].editable);
        assert!(!m.layers[ground].solid);
        assert!(m.layers[walls].solid);
    }

    #[test]
    fn props_are_attached_to_their_cell() {
        let m = parse_map(MINI, "test.xml").unwrap();
        let walls = m.layer_index("walls").unwrap();
        let p = m.props_at(walls, 2, 1);
        assert!(p.is_solid());
        assert!(p.is_water());
        assert!(!m.props_at(walls, 0, 0).is_solid());
    }

    #[test]
    fn solid_layer_blocks_where_props_say_so() {
        let m = parse_map(MINI, "test.xml").unwrap();
        // Kafel (2,1) ma SOLID -> x=32..48, y=16..32.
        assert!(m.overlaps_solid(Rect::from_xywh(34.0, 18.0, 8.0, 8.0)));
        // Sąsiedni kafel tej samej warstwy nie ma właściwości.
        assert!(!m.overlaps_solid(Rect::from_xywh(2.0, 18.0, 8.0, 8.0)));
    }

    #[test]
    fn missing_tileset_is_an_error() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2">
            <layer name="x"></layer></map>"#;
        assert!(parse_map(xml, "t.xml").is_err());
    }

    #[test]
    fn zero_size_is_rejected() {
        let xml = r#"<map name="t" tile-size="16" width="0" height="0"></map>"#;
        assert!(matches!(
            parse_map(xml, "t.xml").unwrap_err(),
            MapError::Format { .. }
        ));
    }

    #[test]
    fn negative_tile_size_is_rejected() {
        let xml = r#"<map name="t" tile-size="-4" width="2" height="2"></map>"#;
        assert!(parse_map(xml, "t.xml").is_err());
    }

    #[test]
    fn bad_number_reports_field() {
        let xml = r#"<map name="t" tile-size="16" width="abc" height="2"></map>"#;
        match parse_map(xml, "t.xml").unwrap_err() {
            MapError::Number { field, .. } => assert_eq!(field, "width"),
            other => panic!("oczekiwano błędu liczby, jest {other:?}"),
        }
    }

    #[test]
    fn malformed_xml_is_reported() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2"><layer"#;
        assert!(parse_map(xml, "t.xml").is_err());
    }

    #[test]
    fn dot_means_empty_tile() {
        let xml = r#"<map name="t" tile-size="16" width="3" height="1">
            <layer name="a" tileset="ts">
              <row y="0">1 . 2</row>
            </layer></map>"#;
        let m = parse_map(xml, "t.xml").unwrap();
        let l = m.layer_index("a").unwrap();
        assert_eq!(m.tile_at(l, 0, 0), 1);
        assert_eq!(m.tile_at(l, 1, 0), TILE_EMPTY, "kropka to pusty kafel");
        assert_eq!(m.tile_at(l, 2, 0), 2);
    }

    #[test]
    fn rows_tolerate_commas_and_extra_spacing() {
        let xml = r#"<map name="t" tile-size="16" width="3" height="1">
            <layer name="a" tileset="ts">
              <row y="0">  1, 2   3 </row>
            </layer></map>"#;
        let m = parse_map(xml, "t.xml").unwrap();
        let l = m.layer_index("a").unwrap();
        assert_eq!(
            (m.tile_at(l, 0, 0), m.tile_at(l, 1, 0), m.tile_at(l, 2, 0)),
            (1, 2, 3)
        );
    }

    #[test]
    fn row_longer_than_map_is_truncated() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="1">
            <layer name="a" tileset="ts">
              <row y="0">1 2 3 4 5</row>
            </layer></map>"#;
        let m = parse_map(xml, "t.xml").unwrap();
        let l = m.layer_index("a").unwrap();
        assert_eq!(m.tile_at(l, 1, 0), 2);
    }

    #[test]
    fn rows_outside_map_are_skipped() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2">
            <layer name="a" tileset="ts">
              <row y="9">1 2</row>
              <row y="-1">1 2</row>
            </layer></map>"#;
        // Wiersze poza mapą nie mogą wywrócić wczytywania.
        let m = parse_map(xml, "t.xml").unwrap();
        let l = m.layer_index("a").unwrap();
        assert_eq!(m.tile_at(l, 0, 0), TILE_EMPTY);
        assert_eq!(m.tile_at(l, 0, 1), TILE_EMPTY);
    }

    #[test]
    fn autotile_is_read_from_layer() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2">
            <layer name="a" tileset="ts">
              <autotile base="100" mode="presence"/>
            </layer></map>"#;
        let m = parse_map(xml, "t.xml").unwrap();
        let at = m.autotile(0).expect("autotiling ustawiony");
        assert_eq!(at.base, 100);
        assert_eq!(at.mode, MatchMode::NonEmpty);
    }

    #[test]
    fn edit_area_limits_set_tile() {
        let mut m = parse_map(MINI, "test.xml").unwrap();
        let ground = m.layer_index("ground").unwrap();
        // Obszar 1..2 kafla obejmuje kafle (1,1) i (1,2), a nie (0,0).
        assert!(m.set_tile(ground, 1, 1, 42));
        assert!(!m.set_tile(ground, 0, 0, 42));
        assert_eq!(m.tile_at(ground, 1, 1), 42);
        assert_ne!(m.tile_at(ground, 0, 0), 42);
    }

    #[test]
    fn tilesets_are_listed_without_loading_files() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2">
            <tileset name="terrain" src="tiles/terrain.png" tile-size="16"/>
            </map>"#;
        let specs = read_tilesets(xml, "t.xml").unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].0, "terrain");
        assert_eq!(specs[0].1.src, "tiles/terrain.png");
        assert_eq!(specs[0].1.tile_size, 16);
    }

    #[test]
    fn tileset_without_src_is_rejected() {
        let xml = r#"<map name="t" tile-size="16" width="2" height="2">
            <tileset name="terrain"/></map>"#;
        assert!(read_tilesets(xml, "t.xml").is_err());
    }

    #[test]
    fn to_xml_roundtrips_through_parse_map() {
        let mut m = TileMap::new("t", 16.0, 3, 2);
        m.spawn = Some(Vec2::new(8.0, 9.0));
        let l = m.add_layer(Layer::new("ground", "terrain", 3, 2).as_editable());
        m.layers[l].tiles.set(1, 1, 42);
        m.set_props(l, 1, 1, TileProps::parse_list("solid"));

        let xml = m.to_xml();
        let parsed = parse_map(&xml, "t.xml").unwrap();
        assert_eq!(parsed.name, "t");
        assert_eq!(parsed.width(), 3);
        assert_eq!(parsed.height(), 2);
        assert_eq!(parsed.tile_at(l, 1, 1), 42);
        assert_eq!(parsed.tile_at(l, 0, 0), TILE_EMPTY);
        assert!(parsed.props_at(l, 1, 1).is_solid());
        assert_eq!(parsed.spawn, Some(Vec2::new(8.0, 9.0)));
    }
}
