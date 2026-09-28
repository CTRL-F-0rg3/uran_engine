//! Warstwa renderująca 2D: batching, kamery, tekstury, tekst i kształty.
//!
//! ## Jak to działa
//!
//! 1. Gra (albo encje ECS) wypełnia [`DrawList`] wywołaniami przez [`Graphics`].
//! 2. Tekst rozwijany jest na sprite'y glifów, 9-slice na 9 prostokątów.
//! 3. Lista jest sortowana po `(warstwa, tryb mieszania, tekstura)`.
//! 4. Sprite'y tej samej tekstury i trybu mieszania idą **jednym draw calle**
//!    (instancjonowanie), siatki mają własny potok.
//!
//! Koszt typowej klatki to 1–3 draw calle niezależnie od liczby obiektów.

pub mod backend;
pub mod batch;
pub mod camera;
pub mod graphics;
pub mod mesh_builder;
pub mod renderer;
pub mod shape;
pub mod text;

pub use batch::{
    DrawList, Globals, MeshDraw, MeshGeometry, MeshPushConstants, NineSliceDraw, SpriteDraw,
    SpriteInstance, TextAlign, TextDraw, TextureKey,
};
pub use camera::Camera2d;
pub use graphics::{DrawStyle, Graphics};
pub use mesh_builder::MeshBuilder;
pub use renderer::{FrameStats, Renderer, ScreenshotRequest};
pub use shape::{circle, polygon, ring, rounded_rect, star, thick_line, triangulate};
pub use text::{FontMetrics, FontRegistry, GlyphInfo, ATLAS_SIZE};

/// Wersja silnika (do logów i diagnostyki).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
