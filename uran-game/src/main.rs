//! Demo silnika: mała arena z shooterem + armia piechoty na GPU.
//!
//! Uruchomienie: `cargo run -p uran-game`
//! Sterowanie: WASD/strzałki — ruch, mysz — celowanie, LPM — strzał,
//! RMB — nowy punkt zgrupowania piechoty, F — uzupełnienie z rezerwy,
//! P — pauza, R — restart, Esc — wyjście.

mod infantry;

use std::f32::consts::TAU;

use uran_engine::prelude::*;
use uran_render::TextAlign;

use crate::infantry::Infantry;

/// Rozmiar świata (arena); kamera pokazuje całość.
const ARENA: Rect = Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0));
const PLAYER_SPEED: f32 = 320.0;
const PLAYER_RADIUS: f32 = 18.0;
const ENEMY_SPEED: f32 = 95.0;
const ENEMY_RADIUS: f32 = 15.0;
const BULLET_SPEED: f32 = 620.0;
const BULLET_RADIUS: f32 = 4.0;
const SPAWN_INTERVAL: f32 = 0.55;
const MAX_ENEMIES: usize = 90;
const MAX_BULLETS: usize = 200;
const MAX_PARTICLES: usize = 500;

/// Wszystko, czego demo potrzebuje, w jednym stanie gry.
struct Game {
    player_pos: Vec2,
    player_hp: f32,
    fire_cooldown: f32,
    spawn_timer: f32,
    score: i32,
    game_over: bool,
    paused: bool,
    shake: f32,
    /// lewy przycisk myszy (do strzelania przytrzymanym)
    shooting: bool,

    enemies: Vec<Vec2>,
    enemy_hp: Vec<f32>,
    bullets: Vec<Vec2>,
    particles: Vec<Particle>,
    /// animacja przejścia kolorowego „trafienia"
    hit_flash: f32,

    /// Armia piechoty symulowana na GPU.
    infantry: Infantry,
    font: Option<Font>,
    time: f32,
    /// Stan generatora liczb losowych (xorshift32).
    rng_state: u32,
}

/// Cząsteczka efektu (śmieci, iskry po strzale).
struct Particle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    color: Color,
    size: f32,
}

impl Game {
    fn new() -> Self {
        Self {
            player_pos: Vec2::ZERO,
            player_hp: 100.0,
            fire_cooldown: 0.0,
            spawn_timer: 0.0,
            score: 0,
            game_over: false,
            paused: false,
            shake: 0.0,
            shooting: false,
            enemies: Vec::new(),
            enemy_hp: Vec::new(),
            bullets: Vec::new(),
            particles: Vec::new(),
            hit_flash: 0.0,
            infantry: Infantry::new(infantry::InfantryConfig::default()),
            font: None,
            time: 0.0,
            rng_state: 0x1234_5678,
        }
    }

    fn reset(&mut self) {
        let font = self.font;
        *self = Self::new();
        self.font = font;
    }

    /// Prosty generator (xorshift32) — bez zewnętrznej zależności.
    fn rand(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32
    }

    fn rand_range(&mut self, min: f32, max: f32) -> f32 {
        min + self.rand() * (max - min)
    }
}

/// Ruch, strzelanie i kolizje (logika gry).
fn update(ctx: &mut Ctx, game: &mut Game) {
    let dt = ctx.dt();

    // globalne sterowanie
    if ctx.input.just_pressed(Key::Escape) {
        std::process::exit(0);
    }
    if ctx.input.just_pressed(Key::KeyP) {
        game.paused = !game.paused;
    }
    if ctx.input.just_pressed(Key::KeyR) {
        game.reset();
    }
    if game.paused || game.game_over {
        if game.game_over && ctx.input.just_pressed(Key::Space) {
            game.reset();
        }
        return;
    }

    // --- ruch gracza (WASD + strzałki) ---
    let dir = Vec2::new(
        ctx.input.axis(Key::KeyA, Key::KeyD) + ctx.input.axis(Key::ArrowLeft, Key::ArrowRight),
        ctx.input.axis(Key::KeyS, Key::KeyW) + ctx.input.axis(Key::ArrowDown, Key::ArrowUp),
    );
    let dir = if dir.length_squared() > 1.0 {
        dir.normalize()
    } else {
        dir
    };
    game.player_pos += dir * PLAYER_SPEED * dt;
    // arena jest prostokątem — pilnujemy granic
    game.player_pos.x = game
        .player_pos
        .x
        .clamp(ARENA.min.x + PLAYER_RADIUS, ARENA.max.x - PLAYER_RADIUS);
    game.player_pos.y = game
        .player_pos
        .y
        .clamp(ARENA.min.y + PLAYER_RADIUS, ARENA.max.y - PLAYER_RADIUS);

    // --- piechota: rozkazy + parametry do GPU (96 B na klatkę) ---
    game.infantry.update(ctx, dt);

    // --- celowanie: pozycja myszy w przestrzeni świata ---
    let aim = ctx
        .camera
        .screen_to_world(ctx.input.mouse_position(), ctx.window.size);

    // --- strzał ---
    game.shooting = ctx.input.primary_pressed();
    game.fire_cooldown -= dt;
    if game.shooting && game.fire_cooldown <= 0.0 && game.bullets.len() < MAX_BULLETS {
        let direction = (aim - game.player_pos).normalize_or_zero();
        game.bullets
            .push(game.player_pos + direction * (PLAYER_RADIUS + BULLET_RADIUS));
        game.fire_cooldown = 0.13;
        game.shake = (game.shake + 1.2).min(3.0);
        spawn_particles(
            &mut game.particles,
            game.player_pos + direction * 20.0,
            direction,
            4,
            Color::from_hex(0xFFD070),
            2.5,
            0.18,
        );
    }

    // --- pociski lecą w stronę aktualnego celu ---
    for bullet in game.bullets.iter_mut() {
        *bullet += (aim - game.player_pos).normalize_or_zero() * BULLET_SPEED * dt;
    }
    let bounds = ARENA.expand_by(40.0, 40.0);
    game.bullets.retain(|b| bounds.contains(*b));

    // --- kolizje pocisków z wrogami ---
    let mut i = 0;
    while i < game.bullets.len() {
        let bullet = game.bullets[i];
        let hit = game
            .enemies
            .iter()
            .position(|enemy| (*enemy - bullet).length() < ENEMY_RADIUS + BULLET_RADIUS);
        if let Some(index) = hit {
            let enemy_pos = game.enemies[index];
            game.bullets.remove(i);
            game.enemy_hp[index] -= 1.0;
            game.score += 10;
            spawn_particles(
                &mut game.particles,
                enemy_pos,
                Vec2::ZERO,
                10,
                Color::from_hex(0xFF6655),
                3.0,
                0.35,
            );
            if game.enemy_hp[index] <= 0.0 {
                game.enemies.remove(index);
                game.enemy_hp.remove(index);
                game.score += 50;
                game.shake = (game.shake + 4.0).min(8.0);
                // losowy kierunek liczymy PRZED pożyczeniem `game.particles`
                let burst = Vec2::new(game.rand_range(-1.0, 1.0), game.rand_range(-1.0, 1.0));
                spawn_particles(
                    &mut game.particles,
                    enemy_pos,
                    burst,
                    22,
                    Color::from_hex(0xFFAA33),
                    3.5,
                    0.55,
                );
            }
        } else {
            i += 1;
        }
    }

    // --- wrogowie gonią gracza ---
    for enemy in game.enemies.iter_mut() {
        *enemy += (game.player_pos - *enemy).normalize_or_zero() * ENEMY_SPEED * dt;
    }

    // --- kontakt wroga z graczem ---
    for enemy in game.enemies.iter() {
        if (*enemy - game.player_pos).length() < ENEMY_RADIUS + PLAYER_RADIUS {
            game.player_hp -= 45.0 * dt;
            game.hit_flash = 1.0;
            game.shake = 6.0;
        }
    }
    if game.player_hp <= 0.0 {
        game.player_hp = 0.0;
        game.game_over = true;
        spawn_particles(
            &mut game.particles,
            game.player_pos,
            Vec2::ZERO,
            40,
            Color::from_hex(0x66CCFF),
            4.0,
            0.8,
        );
    }

    // --- odradzanie wrogów przy krawędziach areny ---
    game.spawn_timer -= dt;
    if game.spawn_timer <= 0.0 && game.enemies.len() < MAX_ENEMIES {
        game.spawn_timer = SPAWN_INTERVAL;
        let pos = match game.rand_range(0.0, 4.0) as i32 {
            0 => Vec2::new(game.rand_range(ARENA.min.x, ARENA.max.x), ARENA.min.y),
            1 => Vec2::new(game.rand_range(ARENA.min.x, ARENA.max.x), ARENA.max.y),
            2 => Vec2::new(ARENA.min.x, game.rand_range(ARENA.min.y, ARENA.max.y)),
            _ => Vec2::new(ARENA.max.x, game.rand_range(ARENA.min.y, ARENA.max.y)),
        };
        game.enemies.push(pos);
        game.enemy_hp.push(1.0);
    }

    // --- cząsteczki ---
    for p in game.particles.iter_mut() {
        p.life -= dt;
        p.pos += p.vel * dt;
        p.vel *= 1.0 - 3.0 * dt; // lekkie tarcie
    }
    game.particles.retain(|p| p.life > 0.0);
    if game.particles.len() > MAX_PARTICLES {
        let excess = game.particles.len() - MAX_PARTICLES;
        game.particles.drain(0..excess);
    }

    // --- wygaszanie efektów ---
    game.shake *= 1.0 - 6.0 * dt;
    game.hit_flash *= 1.0 - 4.0 * dt;
}

/// Dodaje cząsteczki rozrzucone wokół `pos`, przesunięte w kierunku `base_dir`.
fn spawn_particles(
    particles: &mut Vec<Particle>,
    pos: Vec2,
    base_dir: Vec2,
    count: usize,
    color: Color,
    speed: f32,
    life: f32,
) {
    for i in 0..count {
        let angle = (i as f32 / count.max(1) as f32) * TAU;
        let jitter = Vec2::new(angle.cos(), angle.sin()) * 0.8;
        particles.push(Particle {
            pos,
            vel: (base_dir + jitter).normalize_or_zero() * speed * (0.5 + (i % 5) as f32 * 0.15),
            life,
            max_life: life,
            color,
            size: 3.0,
        });
    }
}

/// Rysowanie sceny (tryb natychmiastowy przez `ctx.gfx`).
fn draw(ctx: &mut Ctx, game: &mut Game) {
    // trzepaśnięcie kamery: losowy przesuw zanikający w czasie
    let shake = game.shake;
    let ox = (game.rand() - 0.5) * shake;
    let oy = (game.rand() - 0.5) * shake;

    // --- armia: marker punktu zgrupowania (sama armia rysuje się
    // automatycznie z bufora GPU, poza draw listą) ---
    game.infantry.draw_rally_marker(ctx);

    // --- arena ---
    ctx.gfx
        .color(Color::from_hex(0x141821))
        .draw_rect(Rect::from_center(Vec2::ZERO, ARENA.size()));
    ctx.gfx.layer(-20);
    let grid = 120.0;
    let mut gx = ARENA.min.x;
    while gx <= ARENA.max.x {
        ctx.gfx.color(Color::from_hex(0x1B2130)).draw_line(
            Vec2::new(gx + ox, ARENA.min.y + oy),
            Vec2::new(gx + ox, ARENA.max.y + oy),
            1.0,
        );
        gx += grid;
    }
    let mut gy = ARENA.min.y;
    while gy <= ARENA.max.y {
        ctx.gfx.color(Color::from_hex(0x1B2130)).draw_line(
            Vec2::new(ARENA.min.x + ox, gy + oy),
            Vec2::new(ARENA.max.x + ox, gy + oy),
            1.0,
        );
        gy += grid;
    }
    ctx.gfx
        .layer(-10)
        .color(Color::from_hex(0x2B3A50))
        .draw_rect_outline(ARENA, 4.0);

    // --- cząsteczki ---
    ctx.gfx.layer(0);
    for p in game.particles.iter() {
        let alpha = (p.life / p.max_life).clamp(0.0, 1.0);
        ctx.gfx
            .translate(Vec2::new(ox, oy))
            .color(p.color.with_alpha(alpha))
            .draw_circle(p.pos, p.size * alpha + 0.5);
    }

    // --- wrogowie ---
    ctx.gfx.layer(10);
    for enemy in game.enemies.iter() {
        let pos = *enemy + Vec2::new(ox, oy);
        ctx.gfx
            .color(Color::from_hex(0xFF4D5E))
            .draw_circle(pos, ENEMY_RADIUS);
        ctx.gfx
            .color(Color::from_hex(0xFF9AA4))
            .draw_circle(pos + Vec2::new(0.0, 5.0), ENEMY_RADIUS * 0.35);
    }
    // --- gracz ---
    ctx.gfx.layer(30);
    let player = game.player_pos + Vec2::new(ox, oy);
    // poświata wokół gracza (additive)
    ctx.gfx.blend(BlendMode::Additive);
    ctx.gfx
        .color(Color::from_hex(0x2E6BFF).with_alpha(0.22))
        .draw_circle(player, PLAYER_RADIUS * 2.4);
    ctx.gfx.blend(BlendMode::Alpha);
    let player_color = Color::from_hex(0x3D7BFF).lerp(Color::from_hex(0xFF5566), game.hit_flash);
    ctx.gfx
        .color(player_color)
        .draw_circle(player, PLAYER_RADIUS);
    ctx.gfx
        .color(Color::WHITE.with_alpha(0.7))
        .draw_circle(player + Vec2::new(-5.0, 6.0), PLAYER_RADIUS * 0.32);

    // lufa w stronę kursora + celownik
    let aim = ctx
        .camera
        .screen_to_world(ctx.input.mouse_position(), ctx.window.size);
    let to_aim = (aim - game.player_pos).normalize_or_zero();
    ctx.gfx.color(Color::from_hex(0xE8EEFF)).draw_line(
        player + to_aim * PLAYER_RADIUS * 0.6,
        player + to_aim * (PLAYER_RADIUS + 12.0),
        6.0,
    );
    let aim_w = aim + Vec2::new(ox, oy);
    ctx.gfx
        .layer(40)
        .color(Color::from_hex(0xFFFFFF).with_alpha(0.65));
    ctx.gfx.draw_ring(aim_w, 11.0, 2.0);
    for a in 0..4 {
        let angle = a as f32 * (TAU / 4.0) + game.time * 0.5;
        let dir = Vec2::new(angle.cos(), angle.sin());
        ctx.gfx
            .draw_line(aim_w + dir * 15.0, aim_w + dir * 20.0, 2.0);
    }

    draw_hud(ctx, game);
}

/// HUD: pasek zdrowia, wynik, statystyki renderera, podpowiedzi.
///
/// Rysowany w przestrzeni EKRANU, więc kamera go nie rusza.
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let size = ctx.window.logical_size();
    let Some(font) = game.font else { return };
    ctx.gfx.screen_space().layer(100);

    // --- piechota symulowana na GPU (liczniki z async readbacku) ---
    // Wywołujemy ją tu, w wydzielonym `draw_hud`, bo potrzebuje czcionki
    // gry; statystyki GPU wracają z opóźnieniem, więc HUD nie musi
    // być aktualizowany co klatkę.
    game.infantry.draw_hud(ctx, font, size);
    ctx.gfx.screen_space().layer(100);

    // --- pasek zdrowia ---
    let bar = Rect::new(
        Vec2::new(24.0, size.y - 54.0),
        Vec2::new(304.0, size.y - 30.0),
    );
    ctx.gfx
        .color(Color::from_hex(0x000000).with_alpha(0.45))
        .draw_rect(bar);
    let hp_ratio = (game.player_hp / 100.0).clamp(0.0, 1.0);
    let fill = Rect::new(
        bar.min + Vec2::new(2.0, 2.0),
        Vec2::new(2.0 + (bar.width() - 4.0) * hp_ratio, bar.max.y - 2.0),
    );
    ctx.gfx
        .color(Color::from_hex(0x44DD77).lerp(Color::from_hex(0xFF4455), 1.0 - hp_ratio))
        .draw_rect(fill);

    ctx.gfx.color(Color::from_hex(0xC8D4E8)).draw_text(
        font,
        &format!("HP {}", game.player_hp.ceil() as i32),
        Vec2::new(24.0, size.y - 60.0),
        18.0,
        TextAlign::Left,
    );
    ctx.gfx.color(Color::WHITE).draw_text(
        font,
        &format!("WYNIK {}", game.score),
        Vec2::new(size.x - 24.0, size.y - 40.0),
        24.0,
        TextAlign::Right,
    );

    // statystyki renderera — widać, że batching działa
    // (liczby czytamy PRZED pożyczeniem `ctx.gfx`)
    let fps = ctx.time.fps();
    let ms = ctx.time.delta_millis();
    let draw_calls = ctx.gfx.draw_calls();
    // --- armia: liczniki z GPU (statystyki wracają asynchronicznie,
    // więc mogą być kilka klatek stare) — rysowane w `draw_hud` ---

    ctx.gfx.color(Color::from_hex(0x7A8AA5)).draw_text(
        font,
        &format!("{fps:.0} FPS   {ms:.1} ms   {draw_calls} wywołań rysowania"),
        Vec2::new(24.0, 34.0),
        16.0,
        TextAlign::Left,
    );

    if game.paused {
        overlay(ctx, size, 0x10141D, 0.75);
        ctx.gfx.color(Color::WHITE).draw_text(
            font,
            "PAUZA",
            Vec2::new(size.x * 0.5, size.y * 0.5 + 10.0),
            48.0,
            TextAlign::Center,
        );
        ctx.gfx.color(Color::from_hex(0x9FB0C8)).draw_text(
            font,
            "P — wznów    R — restart    Esc — wyjście",
            Vec2::new(size.x * 0.5, size.y * 0.5 - 40.0),
            18.0,
            TextAlign::Center,
        );
    } else if game.game_over {
        overlay(ctx, size, 0x140A0A, 0.8);
        ctx.gfx.color(Color::from_hex(0xFF6B6B)).draw_text(
            font,
            "KONIEC GRY",
            Vec2::new(size.x * 0.5, size.y * 0.5 + 20.0),
            52.0,
            TextAlign::Center,
        );
        ctx.gfx.color(Color::WHITE).draw_text(
            font,
            &format!("Wynik: {}", game.score),
            Vec2::new(size.x * 0.5, size.y * 0.5 - 20.0),
            28.0,
            TextAlign::Center,
        );
        ctx.gfx.color(Color::from_hex(0x9FB0C8)).draw_text(
            font,
            "SPACJA — jeszcze raz",
            Vec2::new(size.x * 0.5, size.y * 0.5 - 60.0),
            20.0,
            TextAlign::Center,
        );
    } else {
        // Podpowiedź sterowania idzie POD statystykami piechoty (Y=58 i
        // Y=80 w `Infantry::draw_hud`) i POD licznikiem FPS (Y=34).
        // Wszystkie trzy na Y=34 nachodziłyby na siebie.
        ctx.gfx.color(Color::from_hex(0x6C7A90)).draw_text(
            font,
            "WASD — ruch    mysz — celowanie    LPM — strzał    P — pauza    R — restart",
            Vec2::new(size.x * 0.5, 102.0),
            16.0,
            TextAlign::Center,
        );
    }
}

/// Przyciemnienie całego ekranu (pauza / koniec gry).
fn overlay(ctx: &mut Ctx, size: Vec2, rgb: u32, alpha: f32) {
    ctx.gfx
        .layer(99)
        .color(Color::from_hex(rgb).with_alpha(alpha))
        .draw_rect(Rect::from_center(size * 0.5, size));
}
/// Ścieżki czcionek sprawdzane w kolejności, gdy w grze brak własnej.
/// Zaczynamy od `assets/` (katalog projektu), potem typowe ścieżki
/// systemowe — dzięki temu demo działa bez dołączania plików binarnych.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/Hack-Regular.ttf",
    "/usr/share/fonts/TTF/MesloLGS-NF-Regular.ttf",
];

/// Wczytuje czcionkę i dopasowuje kamerę do areny.
fn setup(ctx: &mut Ctx, game: &mut Game) {
    // najpierw własna czcionka projektu, potem systemowa
    game.font = ctx.load_font("assets/fonts/UranSans.ttf");
    if game.font.is_none() {
        for path in FONT_FALLBACKS {
            if let Ok(handle) = ctx.assets.load_font(path) {
                game.font = Some(handle);
                break;
            }
        }
    }
    if game.font.is_none() {
        eprintln!("⚠️  nie znaleziono żadnej czcionki TTF — HUD będzie pusty");
    }

    // --- armia piechoty na GPU ---
    // Armia jest alokowana raz (`.gpu_sim_units`), a tutaj wgrywamy ją
    // w poszczególne sloty. Od tej chwili pozycje już nigdy nie wracają na CPU.
    if !game.infantry.install(ctx, ARENA) {
        eprintln!("⚠️  symulacja GPU wyłączona — demo leci bez piechoty");
    }

    // kamera pokazuje całą arenę
    ctx.camera.fit_world(ARENA, ctx.window.size);
    ctx.clear_color = Color::from_hex(0x0A0C12);
    if std::env::var("URAN_DEBUG").is_ok() {
        eprintln!(
            "[uran] okno={:?} logical={:?} kamera: pozycja={:?} zoom={:.3} widoczne={:?}",
            ctx.window.size,
            ctx.window.logical_size(),
            ctx.camera.position,
            ctx.camera.zoom,
            ctx.camera.visible_size(ctx.window.size),
        );
        // kontrola: gdzie ląduje znany punkt świata na ekranie
        for p in [Vec2::ZERO, ARENA.max, Vec2::new(100.0, 0.0)] {
            eprintln!(
                "[uran]   świat {p:?} -> ekran {:?}",
                ctx.camera.world_to_screen(p, ctx.window.size)
            );
        }
    }
}

fn main() {
    // Stan gry współdzielony między systemami. Systemy muszą żyć tak długo
    // jak aplikacja, więc `game` opakowujemy w `Arc` (klasyczny wzorzec
    // "shared game state" — tutaj na jednym wątku, mutex dla bezpieczeństwa).
    let game = std::sync::Arc::new(std::sync::Mutex::new(Game::new()));

    // systemy: `setup` raz, potem logika i rysowanie co klatkę
    let setup_system = {
        let game = std::sync::Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            setup(ctx, &mut guard);
        }
    };
    let update_system = {
        let game = std::sync::Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            update(ctx, &mut guard);
        }
    };
    let draw_system = {
        let game = std::sync::Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            draw(ctx, &mut guard);
        }
    };

    App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Engine — Arena 2D")
                .background(0x0A0C12)
                .vsync(true)
                .samples(1),
        )
        // Armia 100k żołnierzy liczona na GPU: bufory alokowane raz
        // przy starcie okna, potem CPU wysyła tylko 96 B parametrów.
        .gpu_sim_units(infantry::ARMY_CAPACITY)
        // `--screenshot <ścieżka>` zapisuje klatkę i kończy grę (do testów)
        .screenshot(screenshot_path(), 20)
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}

/// Ścieżka zrzutu z linii poleceń (albo `None`, gdy uruchamiamy „normalnie”).
fn screenshot_path() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}
