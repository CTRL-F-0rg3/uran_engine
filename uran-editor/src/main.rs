//! Edytor graficzny map kafli dla silnika Uran.
//!
//! Uruchomienie:
//! ```text
//! cargo run -p uran-editor -- --map assets/maps/farm.xml
//! cargo run -p uran-editor -- --new 32 24 32 --tileset terrain assets/tilesets/terrain.png 32
//! ```
//!
//! Sterowanie: LPM rysuje kafel, PPM usuwa, ŚPM/Spacja+LPM przesuwa widok,
//! kółko myszy robi zoom, `[`/`]` przełącza warstwy, `Tab` arkusze, `Q`/`E`
//! kafle, `N` nowa warstwa, `H` ukryj warstwę, `F` przełącz kolizję `solid`,
//! `G` siatka, `Ctrl+S` zapis, `Ctrl+O` wczytanie, `Esc` wyjście.

mod editor;

use editor::{Editor, EditorConfig};
use uran_engine::prelude::*;

fn main() {
    let cfg = parse_args();
    let mut editor = Editor::new(cfg);

    App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Editor — edytor map")
                .vsync(true)
                .background(0x131722),
        )
        .add_system(move |ctx: &mut Ctx| {
            editor.update(ctx);
            editor.draw(ctx);
        })
        .run();
}

/// Parsuje argumenty linii poleceń edytora.
fn parse_args() -> EditorConfig {
    let mut cfg = EditorConfig {
        map_path: "assets/maps/untitled.xml".to_string(),
        width: 32,
        height: 24,
        tile_size: 32.0,
        load_map: None,
        tileset: None,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--map" => {
                if let Some(v) = args.get(i + 1) {
                    cfg.load_map = Some(v.clone());
                    cfg.map_path = v.clone();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--new" => {
                if let Some(w) = args.get(i + 1) {
                    cfg.width = w.parse().unwrap_or(32);
                }
                if let Some(h) = args.get(i + 2) {
                    cfg.height = h.parse().unwrap_or(24);
                }
                if let Some(t) = args.get(i + 3) {
                    cfg.tile_size = t.parse().unwrap_or(32.0);
                }
                i += 3;
            }
            "--tileset" => {
                if let (Some(n), Some(p), Some(t)) =
                    (args.get(i + 1), args.get(i + 2), args.get(i + 3))
                {
                    cfg.tileset = Some((n.clone(), p.clone(), t.parse().unwrap_or(16)));
                }
                i += 4;
            }
            "--help" | "-h" => {
                println!(
                    "uran-editor [--map <ścieżka.xml>] [--new <w> <h> [<rozmiar_kafla>]] \
                     [--tileset <nazwa> <plik.png> <rozmiar_kafla_px>]"
                );
                std::process::exit(0);
            }
            _ => i += 1,
        }
    }

    cfg
}
