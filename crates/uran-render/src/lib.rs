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
//!
//! ## Renderer 3D
//!
//! [`Scene3d`] to punkt styku z rendererem 3D (osobny crate `uran-render3d`).
//! Scena 3D dostaje **własny** render pass, **własny** bufor głębokości i
//! **własne** potoki — nie współdzieli z 2D nic poza `Device`/`Queue`/`Surface`.
//! Dzięki temu oba światy rysują się w tej samej klatce: najpierw 3D
//! (czyszczenie koloru i głębokości), potem 2D z `LoadOp::Load`.

pub mod backend;
pub mod batch;
pub mod camera;
pub mod compute;
pub mod graphics;
pub mod mesh_builder;
pub mod renderer;
pub mod scene3d;
pub mod shape;
pub mod text;

pub use batch::{
    DrawList, Globals, MeshDraw, MeshGeometry, MeshPushConstants, NineSliceDraw, SpriteDraw,
    SpriteInstance, TextAlign, TextDraw, TextureKey,
};
pub use backend::device::GpuContext;
pub use camera::{screen_matrix, Camera2d};
pub use scene3d::{Scene3d, Scene3dTarget};
pub use compute::{GpuSim, GpuUnit, SimParams, SimStats, Team, UnitFlags};
pub use graphics::{DrawStyle, Graphics};
pub use mesh_builder::MeshBuilder;
pub use renderer::{FrameStats, Renderer, ScreenshotRequest};
pub use shape::{circle, polygon, ring, rounded_rect, star, thick_line, triangulate};
pub use text::{FontMetrics, FontRegistry, GlyphInfo, ATLAS_SIZE};

/// Wersja silnika (do logów i diagnostyki).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
