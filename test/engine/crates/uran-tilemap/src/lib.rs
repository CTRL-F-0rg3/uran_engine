//! Tilemapa 2D dla silnika Uran.
//!
//! Moduł daje grze komplet do budowy i rysowania świata z kafli:
//!
//! * [`tileset::Tileset`] — arkusz kafli: margines, spacing, flipping,
//!   przeliczanie UV,
//! * [`layer::TileLayer`] — jedna warstwa siatki kafli z cullingiem
//!   i zapytaniami geometrycznymi,
//! * [`autotile::AutoTile`] — dobieranie kafla brzegowego do otoczenia
//!   (16 wariantów „blob"),
//! * [`map::TileMap`] — cała mapa: warstwy, kolizje, obszar edytowalny,
//! * [`xml`] — wczytywanie i zapisywanie mapy w XML (format `*.xml`).
//!
//! ## Przykład
//!
//! ```ignore
//! use uran_tilemap::prelude::*;
//!
//! let mut map = TileMap::load_xml(assets, "maps/farm.xml")?;
//!
//! // rysowanie: kamera pokazuje prostokąt `view` w świecie
//! map.draw(&mut ctx.gfx, ctx.camera.visible_rect(ctx.window.size));
//!
//! // kolizja gracza
//! if map.overlaps_solid(player_rect) { /* cofnij ruch */ }
//!
//! // edycja w ograniczonym zakresie (reszta mapy jest zablokowana)
//! if map.can_edit(4, 7) { map.set_tile(4, 7, TileId::new(12)); }
//! ```

pub mod autotile;
pub mod layer;
pub mod map;
pub mod tileset;
pub mod xml;

pub use autotile::{AutoTile, MatchMode};
pub use layer::{
    flags_from_packed, index_from_packed, pack_tile, TileLayer, FLAGS_MASK, INDEX_MASK, TILE_EMPTY,
};
pub use map::{Layer, MapError, TileId, TileMap, TileProps};
pub use tileset::{TileFlags, TileRef, Tileset};

/// Wszystko, czego typowo potrzebuje gra korzystająca z tilemapy.
pub mod prelude {
    pub use crate::autotile::{AutoTile, MatchMode};
    pub use crate::layer::{pack_tile, TileLayer, TILE_EMPTY};
    pub use crate::map::{Layer, MapError, TileId, TileMap, TileProps};
    pub use crate::tileset::{TileFlags, TileRef, Tileset};
}

/// Wersja modułu (do logów i diagnostyki).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
