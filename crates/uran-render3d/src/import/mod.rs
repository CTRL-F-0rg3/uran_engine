//! Import modeli 3D: `.obj` + `.mtl` z teksturami oraz `.glb` (glTF 2.0).
//!
//! ## Obsługiwane formaty
//!
//! | Format | Status | Uwagi |
//! |--------|--------|-------|
//! | `.obj` + `.mtl` + `.png` | ✅ pełne wsparcie | statyczne bryły |
//! | `.glb` (glTF 2.0) | ✅ siatka + materiały + szkielet | patrz `gltf` |
//! | `.gltf` + osobne pliki | ❌ | wymaga czytania z dysku |
//! | `.fbx`, `.blend` | ❌ | właścicielskie formaty Blendera |
//!
//! ## Dlaczego glTF jest osobnym modułem
//!
//! Oba formaty mają wspólne tylko „czytają wierzchołki". glTF przy okazji
//! niesie szkielet, wagi skinningu i materiały PBR, więc ma własny format
//! wierzchołka (`gltf::SkinVertex`, 64 B) i własny potok renderujący
//! (`crate::skin`). Mieszanie tego z `Vertex` (44 B) rozszerzyłoby każdą
//! bryłę świata o 55% rozmiaru wierzchołka — za cenę, której płaciłby
//! kod w ogóle niemający postaci.
//!
//! ## Ścieżka użycia
//!
//! ```no_run
//! let mut model = uran_render3d::import::obj::load_from_file("bike.obj")?;
//! model.scale_to_height(1.2);          // motocykl ~1.2 m wysoki
//! for part in &model.parts {
//!     println!("{} — {} materiał: {}", part.name,
//!              part.mesh.vertices.len(), part.material.name);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```no_run
//! let ch = uran_render3d::import::gltf::load_from_file("postac.glb")?;
//! println!("{} kości, {} siatek", ch.joints.len(), ch.meshes.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod gltf;
pub mod json;
pub mod mtl;
pub mod obj;

pub use mtl::{Material, MaterialLib};
pub use obj::{
    load_from_file, load_from_file_with_axes, parse_obj, AxisUp, ImportedModel, ModelPart, ObjScene,
};
