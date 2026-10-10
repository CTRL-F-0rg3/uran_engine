//! Szablony plików generowanych przez CLI (`uran new` i `uran element`).

/// `Cargo.toml` nowej gry — zależność od skopiowanego silnika (`engine/`).
pub fn cargo_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
uran-engine = {{ path = "engine/uran-engine" }}
"#
    )
}

/// Punkt wejścia gry: okno + system startowy (init) + system klatki (object).
pub fn main_rs(name: &str) -> String {
    r#"//! {NAME} — gra na silniku Uran.
//!
//! Uruchomienie: `cargo run`.

mod elements;
mod init;
mod object;

use uran_engine::prelude::*;

fn main() {
    App::new()
        .window(windowed(1280, 720).title("{NAME}"))
        .add_startup_system(|ctx: &mut Ctx| {
            init::init_all(ctx);
        })
        .add_system(|ctx: &mut Ctx| {
            object::update_all(ctx);
            if ctx.input.just_pressed(Key::Escape) {
                std::process::exit(0);
            }
        })
        .run();
}
"#
    .replace("{NAME}", name)
}

/// Centralny `src/object.rs` — spina `object::update` wszystkich elementów.
pub fn central_object_rs() -> String {
    r#"//! Centralny rejestr obiektów — wywołuje `object::update` każdego elementu.
//!
//! Nowy element dodajesz przez `uran element <nazwa>` (albo ręcznie:
//! moduł w `elements/mod.rs` + wywołanie poniżej).

use uran_engine::prelude::*;

use crate::elements;

/// Aktualizuje wszystkie elementy gry (co klatkę).
pub fn update_all(ctx: &mut Ctx) {
    // [uran:calls]
}
"#
    .to_string()
}

/// Centralny `src/init.rs` — spina `init::init` wszystkich elementów.
pub fn central_init_rs() -> String {
    r#"//! Centralny punkt startowy — wywołuje `init::init` każdego elementu.

use uran_engine::prelude::*;

use crate::elements;

/// Inicjalizuje wszystkie elementy gry (raz, na starcie).
pub fn init_all(ctx: &mut Ctx) {
    // [uran:calls]
}
"#
    .to_string()
}

/// `src/elements/mod.rs` — rejestr modułów elementów.
pub fn elements_mod_rs() -> String {
    r#"//! Rejestr elementów gry.
//!
//! Każdy element to podfolder `elements/<nazwa>/` z `object.rs`, `init.rs`
//! i skryptami `data/`. Dodawaj moduły ręcznie albo przez `uran element <nazwa>`.

// [uran:mods]
"#
    .to_string()
}

/// Workspace silnika kopiowany do `engine/` — tylko to, czego potrzebuje 2D.
pub fn engine_workspace_toml() -> String {
    r#"[workspace]
resolver = "2"
members = [
    "crates/uran-core",
    "crates/uran-math",
    "crates/uran-ecs",
    "crates/uran-asset",
    "crates/uran-render",
    "crates/uran-tilemap",
    "uran-engine",
]

[workspace.dependencies]
wgpu = "0.19"
winit = "0.30"
glam = "0.25"
ab_glyph = "0.2"
image = { version = "0.25", default-features = false, features = ["png"] }
pollster = "0.3"
bytemuck = { version = "1.14", features = ["derive"] }

uran-core = { path = "crates/uran-core" }
uran-math = { path = "crates/uran-math" }
uran-ecs = { path = "crates/uran-ecs" }
uran-asset = { path = "crates/uran-asset" }
uran-render = { path = "crates/uran-render" }
uran-tilemap = { path = "crates/uran-tilemap" }
uran-engine = { path = "uran-engine" }
"#
    .to_string()
}

/// `elements/<nazwa>/mod.rs`.
pub fn element_mod_rs(name: &str) -> String {
    format!("//! Element `{name}`.\n\npub mod data;\npub mod init;\npub mod object;\n")
}

/// `elements/<nazwa>/object.rs` — wywołuje `update` wszystkich skryptów `data/`.
pub fn element_object_rs(name: &str, ids: &[String]) -> String {
    let mut out = format!(
        "//! Obiekt elementu `{name}` — spinający skrypty z `data/`.\n\n\
         use uran_engine::prelude::*;\n\n\
         use super::data;\n\n\
         /// Aktualizuje element co klatkę (wywołuje skrypty `data/a1..z9`).\n\
         pub fn update(ctx: &mut Ctx) {{\n"
    );
    for id in ids {
        out.push_str(&format!("    data::{id}::update(ctx);\n"));
    }
    out.push_str("}\n");
    out
}

/// `elements/<nazwa>/init.rs` — wywołuje `init` wszystkich skryptów `data/`.
pub fn element_init_rs(name: &str, ids: &[String]) -> String {
    let mut out = format!(
        "//! Inicjalizacja elementu `{name}`.\n\n\
         use uran_engine::prelude::*;\n\n\
         use super::data;\n\n\
         /// Inicjalizuje element (wywołuje `init` skryptów `data/a1..z9`).\n\
         pub fn init(ctx: &mut Ctx) {{\n"
    );
    for id in ids {
        out.push_str(&format!("    data::{id}::init(ctx);\n"));
    }
    out.push_str("}\n");
    out
}

/// `elements/<nazwa>/data/mod.rs`.
pub fn data_mod_rs(name: &str, ids: &[String]) -> String {
    let mut out = format!(
        "//! Skrypty elementu `{name}`: puste szablony `a1..z9` do wypełnienia.\n"
    );
    for id in ids {
        out.push_str(&format!("pub mod {id};\n"));
    }
    out
}

/// `elements/<nazwa>/data/<id>.rs` — pusty szablon skryptu.
pub fn data_script_rs(name: &str, id: &str) -> String {
    format!(
        r#"//! Skrypt `{id}` elementu `{name}` — pusty szablon.
//!
//! Wypełnij go funkcjami (animacje, efekty, dźwięki, logika) i wywołaj je
//! z `object.rs` / `init.rs` elementu: `init` leci raz na starcie,
//! `update` co klatkę.

use uran_engine::prelude::*;

/// Wywoływane raz podczas inicjalizacji elementu.
pub fn init(_ctx: &mut Ctx) {{
    // TODO: wypełnij
}}

/// Wywoływane co klatkę.
pub fn update(_ctx: &mut Ctx) {{
    // TODO: wypełnij
}}
"#,
        id = id,
        name = name
    )
}
