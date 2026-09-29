//! Stardew Demo — mała farma budowana na module tilemapy silnika Uran.
//!
//! Uruchomienie: `cargo run -p stardew-demo`
//!
//! Sterowanie:
//! * `WASD` / strzałki — ruch,
//! * `1..6` — wybór narzędzia (grabie, podlewanka, nasiona ×3, sierp),
//! * `E` — użyj narzędzia na kaflu przed graczem,
//! * `B` — tryb budowy: stawianie/usuwanie kafli w ograniczonym zakresie,
//! * `Tab` — następny dzień (uprawy rosną),
//! * `Esc` — wyjście.
//!
//! # Architektura
//!
//! * [`farm`] — uprawy: sadzenie, wzrost, podlewanie, zbiór,
//! * [`player`] — ruch, kolizje z tilemapą, animacja,
//! * `uran-tilemap` (moduł silnika) — mapa w XML, warstwy, kolizje,
//! * `main.rs` — stan gry, narzędzia, rysowanie i HUD.

mod farm;
mod player;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_render::TextAlign;
use uran_tilemap::TileMap;

use crate::farm::{Crop, CropKind, Farm};
use crate::player::Player;

/// Ścieżka mapy (względem `assets/`).
const MAP_PATH: &str = "maps/farm.xml";
/// Ścieżka arkusza roślin.
const CROPS_ASSET: &str = "vectoraith_tileset_farming_sim_essentials/Original/16x16/Tilesets (Modular)/vectoraith_tileset_farmingsims_crops_dense_spring.png";
/// Ścieżka sprite'a farmera.
const FARMER_ASSET: &str =
    "vectoraith_tileset_farming_sim_essentials/Original/32x32/Sprites/$farmer_32x32.png";

/// Szybkość chodu (jednostki świata na sekundę).
const WALK_SPEED: f32 = 34.0;
/// Ile jednostek świata zajmuje kafel (kafel z XML ma 16 px, a sprite farmera
/// w atlasie 32 px — ta skala wyrównuje oba, więc gracz ma rozmiar kafla).
const WORLD_SCALE: f32 = 2.0;
/// Ile trwa dzień w grze (s) — po tym czasie rośliny rosną.
const DAY_LENGTH: f32 = 60.0;
/// Startowe złoto.
const START_GOLD: u32 = 250;
/// Startowa energia.
const MAX_ENERGY: f32 = 100.0;

/// Kafel ziemi uprawnej w warstwie `ground` (arkusz terenu, kolumna 6).
const GROUND_TILLED: u32 = 6;
/// Kafel dekoracyjny stawiany w trybie budowy (płot z arkusza budynków).
const DECOR_FENCE: u32 = 16 * 5 + 10;

/// Narzędzia w pasku (kolejność = skróty klawiszowe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Grabie — przekopuje trawę w ziemię uprawną.
    Hoe,
    /// Podlewanka.
    Can,
    /// Nasiona: rodzaj uprawy.
    Seeds(CropKind),
    /// Sierp — zbiera dojrzałe.
    Scythe,
}

impl Tool {
    /// Wszystkie narzędzia w kolejności paska.
    const ALL: [Tool; 6] = [
        Tool::Hoe,
        Tool::Can,
        Tool::Seeds(CropKind::Radish),
        Tool::Seeds(CropKind::Wheat),
        Tool::Seeds(CropKind::Corn),
        Tool::Scythe,
    ];

    /// Nazwa do HUD-u.
    fn label(self) -> String {
        match self {
            Tool::Hoe => "Grabie".into(),
            Tool::Can => "Podlewanka".into(),
            Tool::Seeds(k) => format!("Nasiona: {}", k.name()),
            Tool::Scythe => "Sierp".into(),
        }
    }

    /// Koszt energii za użycie (użycie narzędzia ma być wyborem, nie spamem).
    fn energy_cost(self) -> f32 {
        match self {
            Tool::Can => 2.0,
            _ => 1.0,
        }
    }

    /// Kolor ikony w HUD.
    fn color(self) -> Color {
        match self {
            Tool::Hoe => Color::from_hex(0xB07A4A),
            Tool::Can => Color::from_hex(0x4AA3D8),
            Tool::Seeds(CropKind::Radish) => Color::from_hex(0xE2574C),
            Tool::Seeds(CropKind::Wheat) => Color::from_hex(0xE0C24A),
            Tool::Seeds(CropKind::Corn) => Color::from_hex(0x7BC043),
            Tool::Scythe => Color::from_hex(0xC0C0C8),
        }
    }
}

/// Komunikat na ekranie (tymczasowy tekst z akcją gracza).
struct Message {
    text: String,
    /// Czas do zniknięcia (s).
    ttl: f32,
}

/// Zamienia polskie znaki na ASCII.
///
/// Czcionka może nie zawierać pełnego alfabetu (w repo nie ma fontu gry,
/// a czcionki systemowe różnią się między maszynami), a wtedy zamiast litery
/// widałby „krzak" z kwadracików. Transliteracja gwarantuje czytelny HUD
/// niezależnie od tego, jaki plik TTF w końcu się wczyta.
fn ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'ą' | 'Ą' => out.push('a'),
            'ć' | 'Ć' => out.push('c'),
            'ę' | 'Ę' => out.push('e'),
            'ł' | 'Ł' => out.push('l'),
            'ń' | 'Ń' => out.push('n'),
            'ó' | 'Ó' => out.push('o'),
            'ś' | 'Ś' => out.push('s'),
            'ź' | 'Ź' => out.push('z'),
            'ż' | 'Ż' => out.push('z'),
            '—' | '–' => out.push('-'),
            '„' | '"' => out.push('"'),
            other => out.push(other),
        }
    }
    out
}

/// Stan gry: mapa, gracz, uprawy i zasoby.
struct Game {
    /// Mapa wczytana z XML (wspólna dla wszystkich systemów).
    map: TileMap,
    /// Indeks warstwy podłogi (`ground`) — tam gracz przekopuje ziemię.
    ground: usize,
    /// Indeks warstwy dekoracji (`decor`) — tam buduje się w trybie budowy.
    decor: usize,
    player: Player,
    farm: Farm,
    /// Wybrane narzędzie.
    tool: usize,
    /// Złoto.
    gold: u32,
    /// Energia (spada przy pracy, regeneruje się w nocy).
    energy: f32,
    /// Numer dnia (startuje od 1).
    day: u32,
    /// Czas w bieżącym dniu (s).
    day_time: f32,
    /// Tryb budowy: stawianie/usuwanie kafli.
    build_mode: bool,
    /// Komunikat na ekranie.
    message: Option<Message>,
    /// Arkusz roślin (do rysowania upraw).
    crops_tileset: uran_tilemap::Tileset,
    /// Tekstura farmera.
    farmer: Texture,
    /// Czy czcionka się wczytała (bez niej HUD jest pusty).
    font: Option<Font>,
    /// Czy `Tab` przeskakuje do następnego dnia.
    loaded: bool,
}

impl Game {
    fn new(
        map: TileMap,
        crops_tileset: uran_tilemap::Tileset,
        farmer: Texture,
        font: Option<Font>,
    ) -> Self {
        let ground = map.layer_index("ground").unwrap_or(0);
        let decor = map.layer_index("decor").unwrap_or(0);
        // `spawn` z XML-a jest w pikselach kafla (16 px), a gracz chodzi po
        // kaflach 32 px — bez skalowania startowałby w lewym górnym rogu.
        let spawn = map
            .spawn
            .map(|p| p * WORLD_SCALE)
            .unwrap_or_else(|| map.world_rect().scaled(WORLD_SCALE).center());
        Self {
            map,
            ground,
            decor,
            player: Player::at(spawn),
            farm: Farm::new(),
            tool: 0,
            gold: START_GOLD,
            energy: MAX_ENERGY,
            day: 1,
            day_time: 0.0,
            build_mode: false,
            message: None,
            crops_tileset,
            farmer,
            font,
            loaded: true,
        }
    }

    /// Komunikat z komunikatem (zastępuje poprzedni).
    fn say(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            ttl: 2.6,
        });
    }

    /// Aktualnie wybrane narzędzie.
    fn tool(&self) -> Tool {
        Tool::ALL[self.tool.min(Tool::ALL.len() - 1)]
    }

    /// Cały świat gry w jednostkach, w których chodzi gracz (z skalą!).
    ///
    /// `TileMap::world_rect()` podaje prostokąt w pikselach kafla (16 px),
    /// a gracz chodzi po kaflach 32 px. Bez przeskalowania kamera
    /// ograniczałaby się do połowy mapy i gracz mógłby wyjść poza krawędź.
    fn world_bounds(&self) -> Rect {
        self.map.world_rect().scaled(WORLD_SCALE)
    }

    /// Rozmiar kafla w świecie (z mapy × skalę powiększenia).
    fn tile_size(&self) -> f32 {
        self.map.tile_size * WORLD_SCALE
    }

    /// Kafel, na którym gracz stoi.
    fn player_cell(&self) -> (i32, i32) {
        self.world_to_cell(self.player.position)
    }

    /// Kafel **przed** graczem (tam działa narzędzie).
    fn target_cell(&self) -> (i32, i32) {
        let (x, y) = self.player_cell();
        (x + self.player.facing, y)
    }

    /// Przeliczenie świata na współrzędne kafla (uwzględnia skalę).
    fn world_to_cell(&self, p: Vec2) -> (i32, i32) {
        let s = self.tile_size();
        ((p.x / s).floor() as i32, (p.y / s).floor() as i32)
    }

    /// Użycie wybranego narzędzia na kaflu przed graczem.
    fn use_tool(&mut self) {
        let (x, y) = self.target_cell();
        // Wszystko poza dozwolonym obszarem jest blokowane **od razu** —
        // gracz dostaje komunikat, że to nie jego pole, zamiast dziwnie
        // przekopywać ziemię na dziedzińcu.
        if !self.map.can_edit(x, y) {
            self.say("Tutaj nie możesz pracować — twoje pole jest ogrodzone");
            return;
        }
        if self.energy <= 0.0 {
            self.say("Za mało energii! Odpocznij do jutra");
            return;
        }

        let tool = self.tool();

        match tool {
            Tool::Hoe => {
                // Ziemia uprawna to kafel 6 w arkuszu terenu (brązowa).
                if self.farm.is_occupied(x, y) {
                    self.say("Tu już coś rośnie");
                } else if self.map.tile_at(self.ground, x, y) == GROUND_TILLED {
                    self.say("Ta ziemia jest już przekopana");
                } else if self.map.set_tile(self.ground, x, y, GROUND_TILLED) {
                    self.energy -= tool.energy_cost();
                    self.say("Przekopałeś ziemię");
                }
            }
            Tool::Can => {
                if self.farm.water(x, y) {
                    self.energy -= tool.energy_cost();
                    self.say("Podlałeś roślinę");
                } else if self.farm.get(x, y).is_some() {
                    self.say("Tu już podlane");
                } else {
                    self.say("Tu nic nie rośnie");
                }
            }
            Tool::Seeds(kind) => {
                let cost = kind.seed_cost();
                if self.farm.is_occupied(x, y) {
                    self.say("To pole jest już obsadzone");
                } else if self.map.tile_at(self.ground, x, y) != GROUND_TILLED {
                    self.say("Najpierw przekop ziemię grabiami");
                } else if self.gold < cost {
                    self.say("Za mało złota na nasiona");
                } else if self.farm.plant(x, y, kind) {
                    self.gold -= cost;
                    self.energy -= tool.energy_cost();
                    self.say(format!("Posadziłeś: {}", kind.name()));
                }
            }
            Tool::Scythe => {
                let ripe = self.farm.get(x, y).map(Crop::is_ripe).unwrap_or(false);
                if !ripe {
                    self.say(if self.farm.is_occupied(x, y) {
                        "Ta roślina jeszcze nie urosła"
                    } else {
                        "Tu nic nie rośnie"
                    });
                } else if let Some(crop) = self.farm.remove(x, y) {
                    let value = crop.kind.yield_value();
                    self.gold += value;
                    self.energy -= tool.energy_cost();
                    self.say(format!("Zebrałeś {} (+{} zł)", crop.kind.name(), value));
                }
            }
        }
    }

    /// Przejście do następnego dnia: uprawy rosną, energia wraca.
    fn next_day(&mut self) {
        let ripe = self.farm.advance_day();
        self.day += 1;
        self.day_time = 0.0;
        self.energy = MAX_ENERGY;
        if ripe > 0 {
            self.say(format!("Dzień {} — {ripe} upraw dojrzało!", self.day));
        } else {
            self.say(format!("Dzień {} — pora na poranną pracę", self.day));
        }
    }

    /// Stawianie/usuwanie kafla w trybie budowy (w ograniczonym zakresie).
    fn build_at(&mut self, x: i32, y: i32, place: bool) {
        if !self.map.can_edit(x, y) {
            self.say("Budować można tylko na swoim polu");
            return;
        }
        if self.energy <= 0.0 {
            self.say("Za mało energii na budowę");
            return;
        }
        // `decor` to warstwa dekoracji — budujemy na niej, żeby nie zniszczyć
        // podłogi, po której gracz chodzi.
        if place {
            if self.farm.is_occupied(x, y) {
                self.say("Najpierw zbierz roślinę");
                return;
            }
            if self.map.set_tile(self.decor, x, y, DECOR_FENCE) {
                self.energy -= 1.0;
                self.say("Postawiłeś płot");
            }
        } else if self
            .map
            .set_tile(self.decor, x, y, uran_tilemap::TILE_EMPTY)
        {
            self.energy -= 1.0;
            self.say("Zebrałeś płot");
        }
    }
}

/// System logiki: wejście, ruch gracza, upływ dnia, komunikaty.
fn update(ctx: &mut Ctx, game: &mut Game) {
    if !game.loaded {
        return;
    }
    let dt = ctx.dt().min(0.05);

    // --- Wyjście ---
    if ctx.input.just_pressed(Key::Escape) {
        ctx.rendering_enabled = false;
    }

    // --- Wybór narzędzia (1..6) ---
    let tool_keys = [
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
    ];
    for (i, key) in tool_keys.iter().enumerate() {
        if ctx.input.just_pressed(*key) {
            game.tool = i;
            game.say(format!("Wybrano: {}", Tool::ALL[i].label()));
        }
    }
    // Kółko myszy przewija pasek narzędzi.
    if ctx.input.scroll_delta().y != 0.0 {
        let n = Tool::ALL.len() as i32;
        let dir = if ctx.input.scroll_delta().y > 0.0 {
            -1
        } else {
            1
        };
        game.tool = ((game.tool as i32 + dir).rem_euclid(n)) as usize;
    }

    // --- Tryb budowy ---
    if ctx.input.just_pressed(Key::KeyB) {
        game.build_mode = !game.build_mode;
        game.say(if game.build_mode {
            "Tryb budowy: LPM stawia, PPM zdejmuje (tylko na twoim polu)"
        } else {
            "Koniec trybu budowy"
        });
    }

    // --- Użycie narzędzia / budowanie ---
    if game.build_mode {
        let (tx, ty) = game.target_cell();
        if ctx.input.primary_just_pressed() {
            game.build_at(tx, ty, true);
        }
        if ctx.input.mouse_just_pressed(MouseButton::Right) {
            game.build_at(tx, ty, false);
        }
    } else if ctx.input.just_pressed(Key::KeyE) {
        game.use_tool();
    }

    // --- Następny dzień ---
    if ctx.input.just_pressed(Key::Tab) {
        game.next_day();
    }

    // --- Ruch gracza ---
    let mut axis = Vec2::new(
        ctx.input.axis(Key::KeyA, Key::KeyD) + ctx.input.axis(Key::ArrowLeft, Key::ArrowRight),
        ctx.input.axis(Key::KeyS, Key::KeyW) + ctx.input.axis(Key::ArrowDown, Key::ArrowUp),
    );
    // Normalizacja, żeby ruch po skosie nie był szybszy niż w linii.
    if axis.length() > 1.0 {
        axis = axis.normalize();
    }
    game.player.moving = axis != Vec2::ZERO;
    let world = game.world_bounds();
    let tile = game.tile_size();
    // Hitbox w jednostach świata jest mniejszy niż kafel, więc przesuwamy
    // go razem z graczem — inaczej gracz „zaczepiałby" o kafle obok.
    // `overlaps_solid` liczy w pikselach kafla (16 px), a gracz chodzi
    // po kaflach 32 px — dzielimy jego prostokat przez skale swiata.
    let is_solid = |r: Rect| game.map.overlaps_solid(r / WORLD_SCALE);
    game.player
        .move_by(axis * WALK_SPEED * dt * (tile / 32.0), world, is_solid);
    game.player.tick_anim(dt);

    // --- Kamera podąża za graczem ---
    ctx.camera.smooth_follow(game.player.position, dt, 8.0);
    // Bez tego przy krawedzi mapy widac "czarny" pas tla gry.
    ctx.camera
        .clamp_to_bounds(game.world_bounds(), ctx.window.size);

    // --- Upływ dnia ---
    game.day_time += dt;
    if game.day_time >= DAY_LENGTH {
        game.next_day();
    }
    // Energia powoli wraca w ciągu dnia (odpoczynek).
    if game.energy < MAX_ENERGY {
        game.energy = (game.energy + 2.0 * dt).min(MAX_ENERGY);
    }

    // --- Komunikaty ---
    if let Some(msg) = game.message.as_mut() {
        msg.ttl -= dt;
        if msg.ttl <= 0.0 {
            game.message = None;
        }
    }
}

/// System rysowania: mapa z tilemapy, uprawy, gracz, HUD.
fn draw(ctx: &mut Ctx, game: &Game) {
    if !game.loaded {
        return;
    }
    let view = ctx.camera.visible_rect(ctx.window.size);
    let tile = game.tile_size();

    // Kafle mapy. Mapa ma `tile_size` z XML (16 px), a gracz chodzi po
    // kaflach 32 px, więc rysujemy świat przeskalowany — inaczej sprite gracza
    // byłby dwa razy większy od kafla pod nim.
    let scale = Mat3::from_scale(Vec2::splat(WORLD_SCALE));
    ctx.gfx.world_space().push_transform(scale);
    game.map.draw(&mut ctx.gfx, view / WORLD_SCALE);
    draw_crops(ctx, game, view / WORLD_SCALE);
    ctx.gfx.pop_transform();

    // --- Podświetlenie kafla przed graczem (zasięg narzędzia) ---
    let (tx, ty) = game.target_cell();
    let highlight = if game.map.can_edit(tx, ty) {
        Color::WHITE.with_alpha(0.28)
    } else {
        Color::from_hex(0xE8467C).with_alpha(0.28)
    };
    ctx.gfx
        .color(highlight)
        .draw_rect(uran_tilemap::TileLayer::cell_rect(
            tx,
            ty,
            tile / WORLD_SCALE,
        ));
    // Gracz (sprite 32x32 w atlasie 3x4 kafli po 32 px).
    draw_player(ctx, game);
    ctx.gfx.pop_transform();

    draw_hud(ctx, game);
}

/// Rysuje uprawy z własnego arkusza (nie z tilemapy).
///
/// Kafel uprawy dobieramy ze stadium wzrostu, a podlewaną roślinę
/// przyciemniamy — gracz widzi wtedy od razu, co trzeba podlać.
fn draw_crops(ctx: &mut Ctx, game: &Game, view: Rect) {
    let tileset = &game.crops_tileset;
    let tile = game.map.tile_size;
    for (&(x, y), crop) in game.farm.iter() {
        let rect = uran_tilemap::TileLayer::cell_rect(x, y, tile);
        if !view.intersects(&rect) {
            continue;
        }
        let Some(uv) = tileset.uv_for_index(crop.kind.tile_for_stage(crop.stage)) else {
            continue;
        };
        let tint = if crop.watered {
            Color::from_hex(0x9FD8FF)
        } else if crop.is_ripe() {
            Color::WHITE
        } else {
            Color::from_hex(0xD8D8D8)
        };
        ctx.gfx
            .color(tint)
            .draw_texture(tileset.texture(), rect, uv);
    }
}

/// Rysuje sprite'a gracza z atlasu (3 kolumny × 4 klatki chodu, 32 px).
fn draw_player(ctx: &mut Ctx, game: &Game) {
    let frame = game.player.anim_frame().min(3);
    // Kolumna zależy od kierunku (0 = w prawo, 2 = w lewo), wiersz to klatka.
    let col = if game.player.facing < 0 { 2 } else { 0 };
    let uv = UvRect::from_pixels(
        Rect::from_xywh(col as f32 * 32.0, frame as f32 * 32.0, 32.0, 32.0),
        Vec2::new(96.0, 128.0),
    );
    // Sprite stoi „na" kaflu: środek przesuwamy o pół wysokości w górę,
    // żeby stopy lądowały na podłodze, a nie w jej środku.
    // Sprite ma 32 px w atlasie, a kafel 16 px — rysujemy go w rozmiarze
    // kafla, żeby gracz nie był dwa razy większy od otoczenia. Drobne
    // przesunięcie w dół sadzi stopy na podłodze, a nie w jej środku.
    let cell = game.map.tile_size;
    let center = game.player.position + Vec2::new(0.0, cell * 0.12);
    ctx.gfx.color(Color::WHITE).draw_texture(
        game.farmer,
        Rect::from_center(center, Vec2::splat(cell)),
        uv,
    );
}

/// Rysuje HUD: pasek narzędzi, złoto, energię, dzień i komunikaty.
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let Some(font) = game.font else {
        // Bez czcionki HUD byłby pusty i wyglądałby jak błąd renderowania.
        return;
    };
    let size = ctx.window.size;
    ctx.gfx.screen_space();

    // --- Pasek narzędzi (u dołu ekranu) ---
    let slot = 40.0;
    let bar_w = slot * Tool::ALL.len() as f32;
    let bar_h = slot + 14.0;
    let bar = Rect::from_xywh(
        size.x * 0.5 - bar_w * 0.5,
        size.y - bar_h - 12.0,
        bar_w,
        bar_h,
    );
    ctx.gfx
        .color(Color::from_hex(0x1B1F27).with_alpha(0.82))
        .draw_rounded_rect(bar, 8.0);

    for (i, tool) in Tool::ALL.iter().enumerate() {
        let x = bar.min.x + 7.0 + i as f32 * slot;
        let y = bar.min.y + 7.0;
        let cell = Rect::from_xywh(x, y, slot - 8.0, slot - 8.0);
        let selected = i == game.tool;
        // Ikona narzędzia: kolorowe pole + obwódka aktywnego.
        ctx.gfx
            .color(tool.color().multiply(if selected { 1.0 } else { 0.55 }))
            .draw_rounded_rect(cell, 5.0);
        if selected {
            ctx.gfx.color(Color::WHITE).draw_rect_outline(cell, 2.0);
        }
        ctx.gfx.color(Color::WHITE).draw_text(
            font,
            &(i + 1).to_string(),
            cell.center(),
            14.0,
            TextAlign::Center,
        );
    }

    // --- Panel stanu (u góry po lewej) ---
    let panel = Rect::from_xywh(12.0, size.y - 96.0, 250.0, 84.0);
    ctx.gfx
        .color(Color::from_hex(0x1B1F27).with_alpha(0.82))
        .draw_rounded_rect(panel, 8.0);

    ctx.gfx.color(Color::from_hex(0xE8E8E8)).draw_text(
        font,
        &ascii(&format!(
            "Dzien {}  |  {} zl  |  {} upraw ({} dojrzalych)",
            game.day,
            game.gold,
            game.farm.len(),
            game.farm.ripe_count()
        )),
        Vec2::new(panel.min.x + 12.0, panel.max.y - 14.0),
        15.0,
        TextAlign::Left,
    );

    // Pasek energii: szerokość proporcjonalna do tego, ile jej zostało.
    let bar_rect = Rect::from_xywh(panel.min.x + 12.0, panel.min.y + 22.0, 226.0, 12.0);
    ctx.gfx
        .color(Color::from_hex(0x2A2F3A))
        .draw_rounded_rect(bar_rect, 6.0);
    let fill = Rect::from_xywh(
        bar_rect.min.x,
        bar_rect.min.y,
        bar_rect.width() * (game.energy / MAX_ENERGY).clamp(0.0, 1.0),
        bar_rect.height(),
    );
    let energy_color = if game.energy > 50.0 {
        Color::from_hex(0x7BC043)
    } else if game.energy > 20.0 {
        Color::from_hex(0xE0C24A)
    } else {
        Color::from_hex(0xE8467C)
    };
    ctx.gfx.color(energy_color).draw_rounded_rect(fill, 6.0);

    // Pasek dnia: postęp w ciągu doby.
    let time_rect = Rect::from_xywh(panel.min.x + 12.0, panel.min.y + 42.0, 226.0, 8.0);
    ctx.gfx
        .color(Color::from_hex(0x2A2F3A))
        .draw_rounded_rect(time_rect, 4.0);
    let time_fill = Rect::from_xywh(
        time_rect.min.x,
        time_rect.min.y,
        time_rect.width() * (game.day_time / DAY_LENGTH).clamp(0.0, 1.0),
        time_rect.height(),
    );
    ctx.gfx
        .color(Color::from_hex(0xF0A848))
        .draw_rounded_rect(time_fill, 4.0);

    // --- Podpis aktywnego narzędzia nad paskiem ---
    ctx.gfx.color(Color::from_hex(0xE8E8E8)).draw_text(
        font,
        &ascii(&game.tool().label()),
        Vec2::new(size.x * 0.5, bar.min.y - 12.0),
        16.0,
        TextAlign::Center,
    );

    // --- Komunikat akcji (blednie, gdy znikający) ---
    if let Some(msg) = game.message.as_ref() {
        let alpha = (msg.ttl / 0.6).clamp(0.0, 1.0);
        ctx.gfx.color(Color::WHITE.with_alpha(alpha)).draw_text(
            font,
            &ascii(&msg.text),
            Vec2::new(size.x * 0.5, size.y - bar_h - 44.0),
            18.0,
            TextAlign::Center,
        );
    }

    // --- Podpowiedź o trybie budowy ---
    if game.build_mode {
        ctx.gfx.color(Color::from_hex(0xFFD166)).draw_text(
            font,
            "TRYB BUDOWY - B wyjsdz, LPM postaw, PPM zdejmij",
            Vec2::new(size.x * 0.5, 30.0),
            18.0,
            TextAlign::Center,
        );
    }

    ctx.gfx.world_space();
}

/// Ścieżki czcionek próbowane po kolei (jak w `boss-fight`).
///
/// Ścieżki są **bezwzględne**: asset serwera traktuje ścieżki względne jako
/// relatywne do katalogu assetów, więc `"assets/..."` szukałoby pliku
/// w `assets/assets/...` i nigdy by go nie znalazło. Demo nie dostarcza
/// własnej czcionki, więc zostały tylko ścieżki systemowe — inaczej przy
/// każdym uruchomieniu wypisywało ostrzeżenie o brakującym pliku, którego
/// w repo nie ma.
const FONT_FALLBACKS: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/Hack-Regular.ttf",
];

/// Jednorazowa konfiguracja: mapa, tekstury, czcionka, kamera.
///
/// Wszystko, co może się nie udać, dostaje komunikat zamiast paniki — brak
/// czcionki psuje tylko HUD, brak mapy oznacza czarną planszę z napisem.
fn setup(ctx: &mut Ctx, slot: &mut Option<Game>) {
    if slot.is_some() {
        return;
    }

    // Czcionka HUD-u.
    let mut font = None;
    for path in FONT_FALLBACKS {
        if let Some(h) = ctx.load_font(path) {
            font = Some(h);
            break;
        }
    }
    if font.is_none() {
        eprintln!("⚠️  nie znaleziono czcionki TTF — HUD bedzie pusty");
    }

    // Mapa z pliku XML (wraz z arkuszami kafli).
    let Some(map) = ctx.load_tilemap(MAP_PATH) else {
        eprintln!("❌ nie udało się wczytać `{MAP_PATH}` — gra nie wystartuje");
        return;
    };

    // Arkusz roślin (uprawy rysujemy spoza tilemapy).
    let mut crops = match ctx.load_image(CROPS_ASSET) {
        Some(h) => h,
        None => {
            eprintln!("❌ nie udało się wczytać arkusza roślin");
            return;
        }
    };
    let crops_size = ctx
        .assets
        .image(crops)
        .map(|i| Vec2::new(i.width as f32, i.height as f32))
        .unwrap_or(Vec2::new(256.0, 256.0));
    let crops_tileset = uran_tilemap::Tileset::new(crops, 16, crops_size);
    // Kafle uprawy są przezroczyste poza rośliną, więc rysujemy je na wierzchu
    // podłogi, a nie w jej miejscu.
    let _ = &mut crops;

    // Sprite gracza.
    let Some(farmer) = ctx.load_image(FARMER_ASSET) else {
        eprintln!("❌ nie udało się wczytać sprite'a farmera");
        return;
    };

    ctx.clear_color = Color::from_hex(0x1E2A16);

    let mut game = Game::new(map, crops_tileset, farmer, font);
    game.say("Witaj na farmie! E - uzyj narzedzia, B - tryb budowy");
    // Kamera startuje na graczu, inaczej pokazywałaby środek pustej mapy.
    ctx.camera.position = game.player.position;
    *slot = Some(game);
}

fn main() {
    // `Option<Game>` w `Mutex` pozwala odłożyć inicjalizację do pierwszej
    // klatki: dopiero wtedy `Ctx` ma dostęp do `AssetServer` i renderera.
    let game = Arc::new(Mutex::new(None::<Game>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            setup(ctx, &mut guard);
        }
    };
    let update_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_mut() {
                update(ctx, g);
            }
        }
    };
    let draw_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_ref() {
                draw(ctx, g);
            }
        }
    };

    // `CARGO_MANIFEST_DIR` pozwala znaleźć `stardew-demo/assets` niezależnie od
    // katalogu roboczego — bez tego `cargo run -p stardew-demo` z katalogu
    // workspace kończyło się komunikatem „nie ma pliku `maps/farm.xml`",
    // bo binarka leży w `target/debug/`, a assety w crate'ie demo.
    let assets = uran_asset::find_asset_dir(env!("CARGO_MANIFEST_DIR"));
    println!("📂 assety: {}", assets.display());

    App::new()
        .assets(assets)
        .screenshot(screenshot_path(), 120)
        .window(
            windowed(1280, 720)
                .title("Stardew Demo - farma")
                .background(0x1E2A16)
                .vsync(true)
                .samples(1),
        )
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}

/// Ścieżka zrzutu z linii poleceń (albo `None`).
fn screenshot_path() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}
