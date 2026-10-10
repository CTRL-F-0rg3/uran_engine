//! Tworzenie nowego projektu gry (odpowiednik `cargo new <nazwa> --bin`)
//! z dołączonym silnikiem skopiowanym ze wskazanej ścieżki.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::templates;

/// Katalogi silnika kopiowane do `engine/crates/` — tylko to, czego
/// potrzebuje fasada `uran-engine` (render 2D, ECS, asset, tilemapa).
const ENGINE_CRATES: [&str; 6] = [
    "uran-math",
    "uran-asset",
    "uran-ecs",
    "uran-core",
    "uran-render",
    "uran-tilemap",
];

/// Parametry tworzenia projektu.
pub struct Scaffold {
    /// Nazwa pakietu (trafi do `Cargo.toml` i nazwy katalogu).
    pub name: String,
    /// Ścieżka do katalogu silnika (workspace z `uran-engine/` i `crates/`).
    pub engine: Option<PathBuf>,
    /// Katalog, w którym ma powstać projekt (domyślnie bieżący).
    pub at: PathBuf,
}

/// Tworzy nowy projekt i zwraca ścieżkę do jego katalogu głównego.
pub fn scaffold(s: &Scaffold) -> Result<PathBuf, String> {
    if !is_valid_package(&s.name) {
        return Err(format!(
            "nazwa projektu `{}` jest niepoprawna — używaj liter, cyfr, `-` i `_` (nie zaczynaj od cyfry)",
            s.name
        ));
    }

    let root = s.at.join(&s.name);
    if root.exists() {
        return Err(format!("katalog `{}` już istnieje", root.display()));
    }

    fs::create_dir_all(root.join("src/elements")).map_err(io)?;
    fs::create_dir_all(root.join("assets/maps")).map_err(io)?;

    if let Some(engine) = &s.engine {
        copy_engine(engine, &root)?;
    }

    fs::write(root.join("Cargo.toml"), templates::cargo_toml(&s.name)).map_err(io)?;
    fs::write(root.join("src/main.rs"), templates::main_rs(&s.name)).map_err(io)?;
    fs::write(root.join("src/object.rs"), templates::central_object_rs()).map_err(io)?;
    fs::write(root.join("src/init.rs"), templates::central_init_rs()).map_err(io)?;
    fs::write(root.join("src/elements/mod.rs"), templates::elements_mod_rs()).map_err(io)?;

    Ok(root)
}

/// Kopiuje sam silnik (bez gier, dokumentacji i `target/`) do `root/engine/`.
fn copy_engine(engine: &Path, root: &Path) -> Result<(), String> {
    let facade = engine.join("uran-engine/Cargo.toml");
    let core = engine.join("crates/uran-core/Cargo.toml");
    if !facade.is_file() || !core.is_file() {
        return Err(format!(
            "`{}` nie wygląda na katalog silnika Uran (brak `uran-engine/Cargo.toml` lub `crates/uran-core/Cargo.toml`)",
            engine.display()
        ));
    }

    let dst = root.join("engine");
    fs::create_dir_all(&dst).map_err(io)?;

    for c in ENGINE_CRATES {
        copy_dir(&engine.join("crates").join(c), &dst.join("crates").join(c))?;
    }
    copy_dir(&engine.join("uran-engine"), &dst.join("uran-engine"))?;

    fs::write(dst.join("Cargo.toml"), templates::engine_workspace_toml()).map_err(io)?;
    Ok(())
}

/// Kopiuje katalog rekurencyjnie, pomijając `target/` i `.git/`.
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(io)?;
    for entry in fs::read_dir(src).map_err(io)? {
        let entry = entry.map_err(io)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        let to = dst.join(entry.file_name());
        let ty = entry.file_type().map_err(io)?;
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), &to).map_err(io)?;
        }
    }
    Ok(())
}

fn is_valid_package(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn io(e: io::Error) -> String {
    e.to_string()
}
