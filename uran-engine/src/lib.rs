//! Uran Engine — fasada łącząca okno, ECS, input i render 2D.
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! fn main() {
//!     App::new()
//!         .window(windowed(1280, 720).title("Moja gra"))
//!         .add_system(|ctx: &mut Ctx| {
//!             let rect = uran_math::Rect::from_xywh(0.0, 0.0, 100.0, 50.0);
//!             ctx.gfx.color(Color::RED).draw_rect(rect);
//!         })
//!         .run();
//! }
//! ```

pub mod app;
pub mod context;
pub mod scene;

pub use uran_anim as anim;
pub use uran_asset as asset;
pub use uran_audio as audio;
pub use uran_core as core;
pub use uran_ecs as ecs;
pub use uran_ffi as ffi;
pub use uran_math as math;
pub use uran_physics as physics;
pub use uran_render as render;
pub use uran_render3d as render3d;
pub use uran_tilemap as tilemap;
pub use uran_ui as ui;

pub use app::App;
pub use context::{Ctx, SystemFn, WindowState};

// Wyszukiwanie katalogu assetów jest częścią fasady: demo ma szansę działać
// po `cargo run -p gra` bez wskazywania ścieżek z ręki.
pub use uran_asset::find_asset_dir;

/// Wszystko, czego typowo potrzebuje gra w jednym `use`.
pub mod prelude {
    pub use crate::anim::{
        AnimationClip, Animator, Character, CharacterConfig, CharacterSheets, EffectConfig, Facing,
        LoopMode, MotionState, SpriteEffect, SpriteSheet,
    };
    pub use crate::app::App;
    pub use crate::asset::{find_asset_dir, AssetServer, FontData, Handle, Image};
    pub use crate::context::{Ctx, WindowState};
    pub use crate::core::{windowed, Input, Key, Modifiers, Mouse, Time, WindowDescriptor};
    pub use crate::ecs::{
        BlendMode, Entity, Material, Mesh, Sprite, Transform, UvRect, Vertex, Visibility, World,
    };
    pub use crate::math::{Color, Mat3, Rect, Vec2, Vec3, Vec4};
    pub use crate::physics::{BodyKind, CollisionWorld, PhysicsBody, PhysicsWorld, Solid};
    pub use crate::render::{
        Camera2d, DrawList, FrameStats, GpuSim, GpuUnit, Graphics, MeshBuilder, PostFxSettings,
        Renderer, SimParams, SimStats, Team, TextAlign, UnitFlags,
    };
    pub use crate::tilemap::{Layer, TileId, TileMap, TileProps, TiledMap, TiledObject, Tileset};
    pub use crate::audio::{Audio, AudioError, Clip, Sound};
    pub use crate::render3d::{
        Atmosphere, Camera3d, DrawCmd, ImportedModel, InstanceModel, Lighting, MaterialId, MeshId,
        PostSettings, Ray, Renderer3d, SceneBounds, ShadowMap, ShadowSettings, Stylization, Surface,
    };
    pub use crate::scene::{Scene, SceneManager, Transition};
    pub use crate::ui::{Canvas, CanvasMode, Element, ElementKind, RectTransform};
    pub use crate::ffi::{CColor, CRect, CVec2, PluginHost, UranPlugin};
    // Gry nie muszą zależeć od winit — przyciski myszy dostają z fasady.
    pub use winit::event::MouseButton;

    /// Wygodny alias typowy dla uchwytów tekstur.
    pub type Texture = crate::asset::Handle<crate::asset::Image>;
    /// Wygodny alias typowy dla czcionek.
    pub type Font = crate::asset::Handle<crate::asset::FontData>;
}
