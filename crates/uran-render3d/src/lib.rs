//! Renderer 3D — osobny podsystem, współdzielący z rendererem 2D tylko
//! `Device`/`Queue`/`Surface` (patrz `uran_render::scene3d`).
//!
//! Świadomie NIC tu nie dziedziczy po kodzie 2D: własne shadery, własny
//! depth buffer, własne potoki, własna kamera. Jedyna rzecz wspólna to
//! urządzenie GPU i obietnica klatki.

pub mod camera;
pub mod mesh;
pub mod postfx;
pub mod scene;

pub use camera::{Camera3d, Ray};
pub use mesh::{box_corners, model_matrix, GpuMesh, InstanceModel, Mesh, SceneUniform, Vertex};
pub use postfx::{PostFx, PostSettings};
pub use scene::{DrawCmd, Lighting, MeshId, Renderer3d};
