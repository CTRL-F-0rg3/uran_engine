//! Import modeli 3D: `.obj` + `.mtl` z teksturami.
//!
//! ## Obsługiwane formaty
//!
//! | Format | Status |
//! |--------|--------|
//! | `.obj` + `.mtl` + `.png` | ✅ pełne wsparcie |
//! | `.glb` / `.gltf` | ❌ osobny zadanie (patrz `Dlaczego nie glTF`) |
//! | `.fbx`, `.blend` | ❌ właścicielskie formaty Blendera |
//!
//! ## Dlaczego nie glTF od razu
//!
//! `.gltf` to JSON, ale tekstury w `.glb` bywają osadzone w BIN-chunku,
//! a w `.gltf` osobno wraz z `images[].uri` i `samplers`/`filters`.
//! To osobny czytnik z własnymi testami na bajtach. Dorzucenie go „na
//! szybko" bez pokrycia testami oznaczałoby debugowanie formatu
//! na plikach użytkownika, więc świadomie zostawiamy to osobno.
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

pub mod mtl;
pub mod obj;

pub use mtl::{Material, MaterialLib};
pub use obj::{load_from_file, parse_obj, ImportedModel, ModelPart, ObjScene};
