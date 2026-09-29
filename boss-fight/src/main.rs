//! Boss Fight — walka z jednym potężnym bossem na tilemapie 2D.
//!
//! Uruchomienie: `cargo run -p boss-fight`
//! Sterowanie: `A/D` — ruch, `spacja` — skok, `W` — atak mieczem,
//! `S` — tarcza, `R` — restart, `Esc` — wyjście.
//!
//! # Architektura
//!
//! * [`tilemap`] — mapa kafelków i kolizje ze światem,
//! * [`player`] — ruch, skok (z coyote time i jump bufferem),
//! * [`boss`] — ataki, fazy i zdrowie bossa,
//! * [`projectile`] — pociski gracza i bossa,
//! * `main.rs` — wejście, rysowanie, HUD.

mod boss;
mod player;
mod projectile;
mod tilemap;