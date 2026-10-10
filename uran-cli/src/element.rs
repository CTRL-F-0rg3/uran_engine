//! Tworzenie elementu gry: `src/elements/<nazwa>/` z `object.rs`, `init.rs`
//! i skryptami-szablonami `data/a1..z9`, spiętymi w centralnych plikach.

use std::fs;
use std::io;
use std::path::Path;

use crate::scripts::script_ids;
use crate::templates;

/// Marker w `src/elements/mod.rs`, przed którym dopisujemy `pub mod <nazwa>;`.
const MODS_MARKER: &str = "// [uran:mods]";
/// Marker w centralnych `object.rs` / `init.rs`, przed którym dopisujemy wywołania.
const CALLS_MARKER: &str = "// [uran:calls]";

/// Tworzy element i dopina go do `src/elements/mod.rs`, `src/object.rs` i `src/init.rs`.
pub fn create_element(root: &Path, name: &str) -> Result<(), String> {
    if !is_valid_ident(name) {
        return Err(format!(
            "nazwa elementu `{name}` jest niepoprawna — używaj małych liter, cyfr i `_` (nie zaczynaj od cyfry)"
        ));
    }

    let object_path = root.join("src/object.rs");
    if !object_path.is_file() {
        return Err(format!(
            "`{}` nie wygląda na projekt gry Uran (brak `src/object.rs`) — najpierw uruchom `uran new`",
            root.display()
        ));
    }

    let dir = root.join("src/elements").join(name);
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).map_err(io)?;

    let ids = script_ids();

    fs::write(dir.join("mod.rs"), templates::element_mod_rs(name)).map_err(io)?;
    fs::write(dir.join("object.rs"), templates::element_object_rs(name, &ids)).map_err(io)?;
    fs::write(dir.join("init.rs"), templates::element_init_rs(name, &ids)).map_err(io)?;
    fs::write(data_dir.join("mod.rs"), templates::data_mod_rs(name, &ids)).map_err(io)?;
    for id in &ids {
        fs::write(data_dir.join(format!("{id}.rs")), templates::data_script_rs(name, id))
            .map_err(io)?;
    }

    // Dopięcie do centralnych plików projektu.
    insert_before_marker(
        &root.join("src/elements/mod.rs"),
        MODS_MARKER,
        &format!("pub mod {name};"),
    )?;
    insert_before_marker(
        &object_path,
        CALLS_MARKER,
        &format!("elements::{name}::object::update(ctx);"),
    )?;
    insert_before_marker(
        &root.join("src/init.rs"),
        CALLS_MARKER,
        &format!("elements::{name}::init::init(ctx);"),
    )?;

    Ok(())
}

/// Wstawia `line` przed linią `marker`, zachowując jej wcięcie (albo dopisuje
/// na końcu, gdy markera brak). `line` podajemy **bez** wiodących spacji.
fn insert_before_marker(path: &Path, marker: &str, line: &str) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(io)?;
    let Some(pos) = content.find(marker) else {
        return fs::write(path, format!("{content}{line}\n")).map_err(io);
    };

    // Wcięcie = białe znaki przed markerem w tej samej linii.
    let line_start = content[..pos].rfind('\n').map_or(0, |p| p + 1);
    let indent = &content[line_start..pos];
    let updated = format!(
        "{}{indent}{line}\n{indent}{marker}{}",
        &content[..line_start],
        &content[pos + marker.len()..]
    );
    fs::write(path, updated).map_err(io)
}

fn is_valid_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn io(e: io::Error) -> String {
    e.to_string()
}
