//! Wczytywanie map z programu **Tiled** (format TMX / TSX).
//!
//! Konwertuje mapę Tiled na natywną [`TileMap`] silnika:
//!
//! * warstwy kafli → warstwy silnika (warstwa używająca kilku arkuszy jest
//!   dzielona na podwarstwy `nazwa [arkusz]`),
//! * arkusze (inline i zewnętrzne `.tsx`) → [`Tileset`],
//! * warstwy obiektów → [`TiledObject`] (spawny, strefy, kolizje),
//! * obsługiwane kodowania `<data>`: `csv` oraz `base64` (+ `zlib`/`gzip`).
//!
//! ## Ograniczenia
//!
//! * Orientacja **orthogonal** (bez izometrycznej) i mapy **skończone**
//!   (`infinite="0"`),
//! * kafle **kwadratowe** (`tilewidth == tileheight`),
//! * flip poziomy/pionowy jest zachowany; flip diagonalny jest przybliżany.
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! let tiled = ctx.load_tiled_map("maps/poziom.tmx").unwrap();
//! tiled.map.draw(&mut ctx.gfx, ctx.camera.visible_rect(ctx.window.size));
//! for obj in &tiled.objects {
//!     if obj.name == "spawn" { gracz.pos = obj.rect.min; }
//! }
//! ```

use std::path::Path;

use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::Reader;
use uran_asset::AssetServer;
use uran_math::Rect;

use crate::layer::pack_tile;
use crate::map::{Layer, MapError, TileMap};
use crate::tileset::TileFlags;

/// Obiekt z warstwy obiektów Tiled (spawn, strefa, kolizja…).
#[derive(Debug, Clone)]
pub struct TiledObject {
    pub name: String,
    /// Obszar obiektu w jednostkach świata (punkt = prostokąt zerowy).
    pub rect: Rect,
}

/// Wynik wczytania mapy Tiled: mapa kafli + obiekty.
pub struct TiledMap {
    pub map: TileMap,
    pub objects: Vec<TiledObject>,
}

/// Bity flag w GID Tiled.
const FLIP_H: u32 = 0x8000_0000; // odbicie poziome
const FLIP_V: u32 = 0x4000_0000; // odbicie pionowe
const FLIP_D: u32 = 0x2000_0000; // flip diagonalny (obrót 90°)
const GID_MASK: u32 = 0x1FFF_FFFF; // dolne 29 bitów = „czysty" gid

/// Arkusz kafli zebrany z `<tileset>` (inline lub z `.tsx`).
struct TilesetInfo {
    firstgid: u32,
    name: String,
    image: String, // ścieżka względem katalogu assetów
    tile_size: u32,
    margin: u32,
    spacing: u32,
}

/// Warstwa kafli zebrana z `<layer>` — surowe GID-y w kolejności Tiled.
struct TiledLayer {
    name: String,
    width: u32,
    height: u32,
    gids: Vec<u32>,
}

/// Wczytuje mapę Tiled (`.tmx`) i zwraca mapę + obiekty.
pub fn load_tmx(path: &str, assets: &mut AssetServer) -> Result<TiledMap, MapError> {
    let full = assets.resolve(Path::new(path));
    let xml = std::fs::read_to_string(&full).map_err(|source| MapError::Io {
        path: path.to_string(),
        source,
    })?;
    parse_tmx(&xml, path, assets)
}

fn parse_tmx(xml: &str, path: &str, assets: &mut AssetServer) -> Result<TiledMap, MapError> {
    let (name, width, height, tile_width, _tile_height) = read_header(xml, path)?;
    let tilesets = read_tilesets(xml, path, assets)?;
    let layers = read_layers(xml, path)?;
    let objects = read_objects(xml, path, height, tile_width as f32)?;
    build_tiled(
        name,
        width,
        height,
        tile_width as f32,
        &tilesets,
        &layers,
        objects,
        path,
        assets,
    )
}

fn xml_err(path: &str) -> impl Fn(quick_xml::Error) -> MapError + '_ {
    move |e| MapError::Xml {
        path: path.into(),
        message: e.to_string(),
    }
}

fn format_err(path: &str, message: impl Into<String>) -> MapError {
    MapError::Format {
        path: path.into(),
        message: message.into(),
    }
}

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

fn attr_u32(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<u32>, MapError> {
    match attr(e, path, name, decoder)? {
        None => Ok(None),
        Some(v) => v.trim().parse::<u32>().map(Some).map_err(|_| MapError::Number {
            path: path.into(),
            field: name.into(),
            value: v,
        }),
    }
}

fn text_of(e: &BytesText<'_>, path: &str) -> Result<String, MapError> {
    e.decode()
        .map(|s| s.into_owned())
        .map_err(|err| MapError::Xml {
            path: path.into(),
            message: err.to_string(),
        })
}

/// Łączy ścieżkę `source` (relatywną do katalogu pliku `base`) w ścieżkę
/// relatywną do katalogu assetów.
fn resolve_relative(base: &str, source: &str) -> String {
    let dir = Path::new(base).parent().unwrap_or_else(|| Path::new(""));
    dir.join(source).to_string_lossy().into_owned()
}

fn read_header(xml: &str, path: &str) -> Result<(String, u32, u32, u32, u32), MapError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    loop {
        let event = reader.read_event().map_err(xml_err(path))?;
        match &event {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"map" => {
                let dec = reader.decoder();
                let name = attr(e, path, "name", dec)?.unwrap_or_else(|| "map".to_string());
                let width = attr_u32(e, path, "width", dec)?
                    .ok_or_else(|| format_err(path, "<map> bez atrybutu `width`"))?;
                let height = attr_u32(e, path, "height", dec)?
                    .ok_or_else(|| format_err(path, "<map> bez atrybutu `height`"))?;
                let tw = attr_u32(e, path, "tilewidth", dec)?.unwrap_or(32);
                let th = attr_u32(e, path, "tileheight", dec)?.unwrap_or(tw);

                if let Some(o) = attr(e, path, "orientation", dec)? {
                    if o != "orthogonal" {
                        return Err(format_err(
                            path,
                            format!("nieobsługiwana orientacja `{o}` — tylko `orthogonal`"),
                        ));
                    }
                }
                if attr(e, path, "infinite", dec)?.as_deref() == Some("1") {
                    return Err(format_err(path, "mapy `infinite` nie są obsługiwane"));
                }
                return Ok((name, width, height, tw, th));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Err(format_err(path, "brak elementu <map>"))
}

/// Zbiera deklaracje `<tileset>`: inline i zewnętrzne pliki `.tsx`.
fn read_tilesets(xml: &str, path: &str, assets: &AssetServer) -> Result<Vec<TilesetInfo>, MapError> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    // (firstgid, name, tile_size, margin, spacing) czekające na <image source>.
    let mut pending: Option<(u32, String, u32, u32, u32)> = None;

    loop {
        let event = reader.read_event().map_err(xml_err(path))?;
        match &event {
            Event::Empty(e) if e.name().as_ref() == b"tileset" => {
                let dec = reader.decoder();
                let firstgid = attr_u32(e, path, "firstgid", dec)?.unwrap_or(1);
                if let Some(src) = attr(e, path, "source", dec)? {
                    let tsx_path = resolve_relative(path, &src);
                    let tsx_full = assets.resolve(Path::new(&tsx_path));
                    let tsx_xml = std::fs::read_to_string(&tsx_full).map_err(|source| {
                        MapError::Io {
                            path: tsx_path.clone(),
                            source,
                        }
                    })?;
                    out.push(read_tsx(&tsx_xml, &tsx_path, firstgid)?);
                }
            }
            Event::Start(e) if e.name().as_ref() == b"tileset" => {
                let dec = reader.decoder();
                let firstgid = attr_u32(e, path, "firstgid", dec)?.unwrap_or(1);
                let name = attr(e, path, "name", dec)?.unwrap_or_default();
                let tw = attr_u32(e, path, "tilewidth", dec)?.unwrap_or(32);
                let margin = attr_u32(e, path, "margin", dec)?.unwrap_or(0);
                let spacing = attr_u32(e, path, "spacing", dec)?.unwrap_or(0);
                pending = Some((firstgid, name, tw, margin, spacing));
            }
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"image" => {
                if let Some((firstgid, name, tw, margin, spacing)) = pending.take() {
                    let dec = reader.decoder();
                    let src = attr(e, path, "source", dec)?
                        .ok_or_else(|| format_err(path, "<tileset> bez <image source>"))?;
                    out.push(TilesetInfo {
                        firstgid,
                        name,
                        image: resolve_relative(path, &src),
                        tile_size: tw,
                        margin,
                        spacing,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if out.is_empty() {
        return Err(format_err(path, "mapa nie ma żadnego <tileset>"));
    }
    out.sort_by_key(|t| t.firstgid);
    Ok(out)
}

/// Parsuje pojedynczy plik `.tsx` (zewnętrzny arkusz).
fn read_tsx(xml: &str, path: &str, firstgid: u32) -> Result<TilesetInfo, MapError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut name = String::new();
    let mut tw = 32u32;
    let mut margin = 0u32;
    let mut spacing = 0u32;
    let mut image: Option<String> = None;

    loop {
        let event = reader.read_event().map_err(xml_err(path))?;
        if let Event::Start(e) | Event::Empty(e) = &event {
            let dec = reader.decoder();
            match e.name().as_ref() {
                b"tileset" => {
                    name = attr(e, path, "name", dec)?.unwrap_or_default();
                    tw = attr_u32(e, path, "tilewidth", dec)?.unwrap_or(32);
                    margin = attr_u32(e, path, "margin", dec)?.unwrap_or(0);
                    spacing = attr_u32(e, path, "spacing", dec)?.unwrap_or(0);
                }
                b"image" => {
                    image = Some(
                        attr(e, path, "source", dec)?
                            .ok_or_else(|| format_err(path, "<image> bez `source`"))?,
                    );
                }
                _ => {}
            }
        }
        if matches!(event, Event::Eof) {
            break;
        }
    }
    let image = image.ok_or_else(|| format_err(path, "arkusz `.tsx` bez <image source>"))?;
    Ok(TilesetInfo {
        firstgid,
        name,
        image: resolve_relative(path, &image),
        tile_size: tw,
        margin,
        spacing,
    })
}

/// Zbiera warstwy kafli (`<layer>` z `<data>`).
fn read_layers(xml: &str, path: &str) -> Result<Vec<TiledLayer>, MapError> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    // Stan aktualnej warstwy: (nazwa, szerokość, wysokość, encoding, compression, data).
    let mut pending: Option<(String, u32, u32, Option<String>, Option<String>, String)> = None;
    let mut in_data = false;

    loop {
        let event = reader.read_event().map_err(xml_err(path))?;
        match &event {
            Event::Empty(e) | Event::Start(e) if e.name().as_ref() == b"layer" => {
                let dec = reader.decoder();
                let name = attr(e, path, "name", dec)?.unwrap_or_default();
                let w = attr_u32(e, path, "width", dec)?.unwrap_or(0);
                let h = attr_u32(e, path, "height", dec)?.unwrap_or(0);
                pending = Some((name, w, h, None, None, String::new()));
                in_data = false;
            }
            Event::Start(e) if e.name().as_ref() == b"data" => {
                let dec = reader.decoder();
                if let Some(p) = pending.as_mut() {
                    p.3 = attr(e, path, "encoding", dec)?;
                    p.4 = attr(e, path, "compression", dec)?;
                }
                in_data = true;
            }
            Event::End(e) if e.name().as_ref() == b"data" => {
                in_data = false;
            }
            Event::Text(e) if in_data => {
                if let Some(p) = pending.as_mut() {
                    p.5.push_str(&text_of(e, path)?);
                }
            }
            Event::End(e) if e.name().as_ref() == b"layer" => {
                if let Some((name, w, h, encoding, compression, data)) = pending.take() {
                    let gids = decode_data(&data, encoding.as_deref(), compression.as_deref(), path)?;
                    out.push(TiledLayer {
                        name,
                        width: w,
                        height: h,
                        gids,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Dekoduje zawartość `<data>` (csv / base64 [+zlib|gzip]) na GID-y.
fn decode_data(
    text: &str,
    encoding: Option<&str>,
    compression: Option<&str>,
    path: &str,
) -> Result<Vec<u32>, MapError> {
    match encoding {
        None | Some("csv") => {
            let mut gids = Vec::new();
            for tok in text.split(',') {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                let v = tok.parse::<u32>().map_err(|_| MapError::Number {
                    path: path.into(),
                    field: "data".into(),
                    value: tok.to_string(),
                })?;
                gids.push(v);
            }
            Ok(gids)
        }
        Some("base64") => {
            use base64::Engine as _;
            let compact: String = text.split_whitespace().collect();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&compact)
                .map_err(|e| format_err(path, format!("błędny base64: {e}")))?;
            let raw = match compression {
                None => bytes,
                Some("zlib") => inflate_zlib(&bytes, path)?,
                Some("gzip") => inflate_gzip(&bytes, path)?,
                Some(other) => {
                    return Err(format_err(
                        path,
                        format!("nieobsługiwana kompresja `{other}`"),
                    ));
                }
            };
            Ok(raw
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect())
        }
        Some(other) => Err(format_err(
            path,
            format!("nieobsługiwane kodowanie `{other}`"),
        )),
    }
}

fn inflate_zlib(bytes: &[u8], path: &str) -> Result<Vec<u8>, MapError> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| format_err(path, format!("błąd dekompresji zlib: {e}")))?;
    Ok(out)
}

fn inflate_gzip(bytes: &[u8], path: &str) -> Result<Vec<u8>, MapError> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|e| format_err(path, format!("błąd dekompresji gzip: {e}")))?;
    Ok(out)
}

fn attr_f32(
    e: &BytesStart<'_>,
    path: &str,
    name: &str,
    decoder: quick_xml::encoding::Decoder,
) -> Result<Option<f32>, MapError> {
    match attr(e, path, name, decoder)? {
        None => Ok(None),
        Some(v) => v.trim().parse::<f32>().map(Some).map_err(|_| MapError::Number {
            path: path.into(),
            field: name.into(),
            value: v,
        }),
    }
}

/// Zbiera obiekty z warstw obiektów (`<objectgroup>` → `<object>`).
fn read_objects(
    xml: &str,
    path: &str,
    map_height: u32,
    tile_size: f32,
) -> Result<Vec<TiledObject>, MapError> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    loop {
        let event = reader.read_event().map_err(xml_err(path))?;
        if let Event::Empty(e) | Event::Start(e) = &event {
            if e.name().as_ref() == b"object" {
                let dec = reader.decoder();
                let name = attr(e, path, "name", dec)?.unwrap_or_default();
                let x = attr_f32(e, path, "x", dec)?.unwrap_or(0.0);
                let y = attr_f32(e, path, "y", dec)?.unwrap_or(0.0);
                let w = attr_f32(e, path, "width", dec)?.unwrap_or(0.0);
                let h = attr_f32(e, path, "height", dec)?.unwrap_or(0.0);
                // Tiled: y od góry; silnik: y w górę.
                let world_h = map_height as f32 * tile_size;
                let wy = world_h - y - h;
                out.push(TiledObject {
                    name,
                    rect: Rect::from_xywh(x, wy, w, h),
                });
            }
        }
        if matches!(event, Event::Eof) {
            break;
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn build_tiled(
    name: String,
    width: u32,
    height: u32,
    tile_size: f32,
    tilesets: &[TilesetInfo],
    layers: &[TiledLayer],
    objects: Vec<TiledObject>,
    _path: &str,
    assets: &mut AssetServer,
) -> Result<TiledMap, MapError> {
    let mut map = TileMap::new(name, tile_size, width, height);

    for ts in tilesets {
        map.add_tileset_from_image(&ts.name, assets, &ts.image, ts.tile_size, ts.margin, ts.spacing)?;
    }

    for layer in layers {
        // Które arkusze są używane w tej warstwie.
        let used: Vec<usize> = (0..tilesets.len())
            .filter(|&i| {
                let lo = tilesets[i].firstgid;
                let hi = tilesets.get(i + 1).map_or(u32::MAX, |t| t.firstgid);
                layer
                    .gids
                    .iter()
                    .any(|&g| clean_gid(g) != 0 && clean_gid(g) >= lo && clean_gid(g) < hi)
            })
            .collect();

        if used.is_empty() {
            continue;
        }

        for &ti in &used {
            let ts = &tilesets[ti];
            let hi = tilesets.get(ti + 1).map_or(u32::MAX, |t| t.firstgid);
            let lname = if used.len() == 1 {
                layer.name.clone()
            } else {
                format!("{} [{}]", layer.name, ts.name)
            };
            let mut el = Layer::new(&lname, &ts.name, layer.width, layer.height);
            let lower = lname.to_ascii_lowercase();
            el.solid = lower.contains("wall") || lower.contains("solid") || lower.contains("collision");
            el.editable = lower.contains("ground") || lower.contains("floor") || lower.contains("terrain");

            let lw = layer.width.max(1);
            for (i, &g) in layer.gids.iter().enumerate() {
                let c = clean_gid(g);
                if c == 0 || c < ts.firstgid || c >= hi {
                    continue;
                }
                let local = c - ts.firstgid;
                let flags = tile_flags(g);
                let tx = i as u32 % lw;
                let ty = i as u32 / lw; // wiersz Tiled: 0 = góra
                if ty >= layer.height {
                    continue; // nadmiarowe dane poza warstwą
                }
                let ey = layer.height - 1 - ty; // wiersz silnika: 0 = dół
                el.tiles.set(tx as i32, ey as i32, pack_tile(local, flags));
            }
            map.add_layer(el);
        }
    }

    Ok(TiledMap { map, objects })
}

/// Dolne 29 bitów GID to „czysty" identyfikator kafla (bez flag).
fn clean_gid(g: u32) -> u32 {
    g & GID_MASK
}

/// Zamienia flagi GID Tiled na [`TileFlags`] silnika.
fn tile_flags(g: u32) -> TileFlags {
    let mut f = TileFlags::NONE;
    if g & FLIP_H != 0 {
        f = TileFlags::combined(f, TileFlags::FLIP_X);
    }
    if g & FLIP_V != 0 {
        f = TileFlags::combined(f, TileFlags::FLIP_Y);
    }
    if g & FLIP_D != 0 {
        f = TileFlags::combined(f, TileFlags::FLIP_BOTH);
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_data_decodes() {
        let gids = decode_data("1,2,0,3, 5", Some("csv"), None, "t").unwrap();
        assert_eq!(gids, vec![1, 2, 0, 3, 5]);
    }

    #[test]
    fn base64_data_roundtrips() {
        use base64::Engine as _;
        let gids: Vec<u32> = vec![1, 2, 0, 3, 5];
        let bytes: Vec<u8> = gids.iter().flat_map(|g| g.to_le_bytes()).collect();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let decoded = decode_data(&b64, Some("base64"), None, "t").unwrap();
        assert_eq!(decoded, gids);
    }

    #[test]
    fn base64_zlib_roundtrips() {
        use base64::Engine as _;
        use std::io::Write;
        let gids: Vec<u32> = vec![1, 2, 0, 3];
        let raw: Vec<u8> = gids.iter().flat_map(|g| g.to_le_bytes()).collect();
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
        let decoded = decode_data(&b64, Some("base64"), Some("zlib"), "t").unwrap();
        assert_eq!(decoded, gids);
    }

    #[test]
    fn gid_flags_and_clean_gid_work() {
        assert_eq!(clean_gid(0x8000_0001), 1);
        assert_eq!(clean_gid(0xFFFF_FFFF), 0x1FFF_FFFF);
        assert_eq!(tile_flags(0x8000_0001), TileFlags::FLIP_X);
        assert_eq!(tile_flags(0x4000_0001), TileFlags::FLIP_Y);
        assert_eq!(tile_flags(1), TileFlags::NONE);
    }
}






