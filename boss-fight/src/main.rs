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

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_render::TextAlign;

use crate::boss::{Attack, Boss, BossConfig};
use crate::player::{Player, PlayerConfig};
use crate::projectile::{BULLET_LIFE, Owner, Projectile};
use crate::tilemap::{MAP_H, MAP_W, TILE, Tile, TileMap};

/// Świat gry w jednostkach (kafle × [`TILE`]).
const WORLD: Rect = Rect::new(
    Vec2::new(0.0, 0.0),
    Vec2::new(MAP_W as f32 * TILE, MAP_H as f32 * TILE),
);

/// Maksymalna liczba pocisków naraz. Bez limitu deszcz w fazie 3
/// potrafiłby wygenerować tysiące obiektów i zabić grę samą pamięcią.
const MAX_BULLETS: usize = 400;

/// Jak długo gracz jest nietykalny po trafieniu (s).
const HURT_COOLDOWN: f32 = 1.1;
/// Ile zdrowia gracz ma na start.
const PLAYER_MAX_HP: f32 = 100.0;
/// Czas między strzałami gracza (s).
const ATTACK_COOLDOWN: f32 = 0.22;
/// Jak mocno strzał gracza rani bossa.
const PLAYER_DAMAGE: f32 = 26.0;

/// Buduje arenę: podłoga, ściany boczne i platformy.
///
/// Rysujemy ją **ręcznie**, a nie generatorem, bo walka z bossem
/// potrzebuje czytelnej, zaplanowanej areny, a nie losowego labiryntu.
/// Kolejność platform jest celowa: wysokie po lewej (do unikania),
/// niskie po prawej (do zejścia), a podłoga w środku zostawia miejsce
/// na ground slam.
fn build_arena() -> TileMap {
    let mut grid: Vec<Vec<Tile>> = vec![vec![Tile::Empty; MAP_W]; MAP_H];

    // Podłoga na dole i sufit na górze — bez sufitu gracz mógłby
    // wyskoczyć z mapy podczas deszczu.
    grid[0] = vec![Tile::Solid; MAP_W];
    grid[MAP_H - 1] = vec![Tile::Solid; MAP_W];
    // Ściany boczne: gracz nie wychodzi poza mapę.
    for row in grid.iter_mut() {
        row[0] = Tile::Solid;
        row[MAP_W - 1] = Tile::Solid;
    }

    // Platformy: cztery poziomy schodkowo, żeby dać się unikać
    // ataków w powietrzu i wchodzić na nie skokiem. Wszystkie
    // mieszczą się w 40 kolumnach (0 i 39 to ściany).
    let platforms: &[(usize, usize, usize)] = &[
        (4, 2, 6),   // niskie, lewe — miejsce na rozbieg
        (7, 9, 7),   // środkowe
        (10, 18, 6), // wysokie, prawe
        (5, 27, 8),  // najdalsze, nad spawnem bossa
    ];
    for &(y, x0, len) in platforms {
        for x in x0..(x0 + len).min(MAP_W - 1) {
            grid[y][x] = Tile::Platform;
        }
    }

    // Kolce na podłodze — groźba, która uczy pilnowania podłoża
    // zamiast tylko reagowania na pociski. Zostawiamy szczelinę
    // szeroką na skok, żeby nie były nie do przejścia.
    for &(x0, len) in &[(15usize, 3usize), (30, 3)] {
        for x in x0..(x0 + len).min(MAP_W - 1) {
            grid[1][x] = Tile::Spike;
        }
    }

    let refs: Vec<&[Tile]> = grid.iter().map(|r| r.as_slice()).collect();
    TileMap::from_rows(&refs)
}

/// Wszystko, czego gra potrzebuje, w jednym stanie.
struct Game {
    map: TileMap,
    player: Player,
    player_cfg: PlayerConfig,
    player_hp: f32,
    /// Ile zostało nietykalności (s) — własny zegar, bo `Player`
    /// trzyma go dla efektów, a HUD potrzebuje wartości globalnej.
    hurt_cd: f32,
    /// Czy gracz właśnie przyjął cios (do animacji).
    flash: f32,

    boss: Boss,
    boss_cfg: BossConfig,

    bullets: Vec<Projectile>,
    /// Ile zostało do następnego strzału gracza (s).
    attack_cd: f32,
    /// Czy gracz trzyma przycisk ataku.
    attacking: bool,

    /// Trzęsienie kamery.
    shake: f32,
    /// Czas świata (animacje).
    time: f32,
    /// Czy walka zakończona (wygrana albo porażka).
    over: bool,
    won: bool,
    /// Licznik klatek od startu — do zrzutu ekranu.
    frames: u32,
    font: Option<Font>,
}

impl Game {
    fn new() -> Self {
        let boss_cfg = BossConfig::default();
        let boss_pos = Vec2::new(WORLD.center().x + 320.0, TILE * 6.0);
        let mut player = Player::default();
        player.pos = Vec2::new(TILE * 3.0, TILE);
        player.grounded = true;
        Self {
            map: build_arena(),
            player,
            player_cfg: PlayerConfig::default(),
            player_hp: PLAYER_MAX_HP,
            hurt_cd: 0.0,
            flash: 0.0,
            boss: Boss::new(boss_pos, &boss_cfg),
            boss_cfg,
            bullets: Vec::new(),
            attack_cd: 0.0,
            attacking: false,
            shake: 0.0,
            time: 0.0,
            over: false,
            won: false,
            frames: 0,
            font: None,
        }
    }

    fn reset(&mut self) {
        let font = self.font;
        *self = Self::new();
        self.font = font;
    }
}

/// Wejście + krok symulacji.
fn update(ctx: &mut Ctx, game: &mut Game) {
    let dt = ctx.dt();
    game.frames = game.frames.wrapping_add(1);
    if !game.over {
        game.time += dt;
    }

    if ctx.input.just_pressed(Key::Escape) {
        std::process::exit(0);
    }
    if ctx.input.just_pressed(Key::KeyR) {
        game.reset();
    }

    let frozen = game.over;
    let dt = if frozen { 0.0 } else { dt };

    // --- wejście gracza ---
    //
    // `just_pressed` to **wciśnięcie**, `pressed` to trzymanie. Skok
    // potrzebuje obu: wciśnięcia, żeby wystartować, i trzymania, żeby
    // sięgnąć pełnej wysokości.
    let move_x =
        ctx.input.axis(Key::KeyA, Key::KeyD) + ctx.input.axis(Key::ArrowLeft, Key::ArrowRight);
    let jump_pressed = ctx.input.just_pressed(Key::Space);
    let jump_held = ctx.input.pressed(Key::Space);
    let attack_pressed = ctx.input.pressed(Key::KeyJ) || ctx.input.pressed(Key::KeyW);
    game.attacking = attack_pressed;

    game.player.step(
        dt,
        move_x.clamp(-1.0, 1.0),
        jump_pressed,
        jump_held,
        &game.map,
        &game.player_cfg,
    );

    // --- strzał gracza ---
    //
    // Pocisk leci **poziomo** w stronę, w którą gracz patrzy. Skok
    // pod kątem w górę dzięki temu nie jest możliwy, a to utrudnia
    // trafianie bossa szybciej niż chodzenie — w zamian gracz musi
    // podchodzić, co odsłania go na pociski. Taki kompromis daje
    // rozgrywkę zamiast bezmyślnego stawiania pocisków z dystansu.
    if attack_pressed && !frozen {
        game.attack_cd -= dt;
        if game.attack_cd <= 0.0 {
            game.attack_cd = ATTACK_COOLDOWN;
            let c = game.player.center();
            let dir = game.player.facing;
            game.bullets.push(Projectile {
                pos: c + Vec2::new(dir * 16.0, 0.0),
                vel: Vec2::new(dir * 620.0, 0.0),
                owner: Owner::Player,
                damage: PLAYER_DAMAGE,
                size: Vec2::new(18.0, 7.0),
                life: BULLET_LIFE,
                piercing: false,
            });
        }
    }

    // --- boss ---
    game.boss
        .step(dt, game.player.center(), &game.boss_cfg, &mut game.bullets);

    // --- ruch pocisków ---
    //
    // Osobny krok przed kolizjami: `boss::step` tylko **spawnuje**
    // pociski, samo ich poruszanie jest wspólne dla obu stron. Bez
    // tej pętli pociski stałyby w miejscu, a gracz nie mógłby nic
    // trafić ani zostać trafiony.
    for b in &mut game.bullets {
        b.step(dt);
    }

    // --- pociski i kolizje ---
    let boss_rect = game.boss.rect(&game.boss_cfg);
    let player_rect = game.player.rect();
    let mut player_damage = 0.0f32;

    // `retain` zamiast `remove_if`: usuwamy w tej samej klatce, w
    // której pocisk zadał obrażenia, żeby nie dało się „strzelić
    // w klatkę po śmierci".
    game.bullets.retain(|b| {
        let mut keep = true;
        if b.owner == Owner::Player {
            if b.rect().intersects(&boss_rect) {
                game.boss.take_damage(b.damage, &game.boss_cfg);
                game.shake = (game.shake + 2.0).min(9.0);
                if !b.piercing {
                    keep = false;
                }
            }
        } else if b.rect().intersects(&player_rect) {
            // Nietykalność: kilka pocisków w tej samej klatce nie może
            // zabić gracza cztery razy.
            if game.hurt_cd <= 0.0 {
                player_damage = player_damage.max(b.damage);
                game.hurt_cd = HURT_COOLDOWN;
                game.flash = 0.28;
                game.shake = (game.shake + 7.0).min(16.0);
            }
            keep = false;
        }
        // Pocisk poza mapą znika — inaczej kumulowałby się w pamięci.
        keep && !b.is_dead() && b.pos.y > -TILE && b.pos.y < WORLD.top() + TILE * 4.0
    });

    // Limit pocisków: gdy boss przesyła, odrzucamy najstarsze.
    if game.bullets.len() > MAX_BULLETS {
        let excess = game.bullets.len() - MAX_BULLETS;
        game.bullets.drain(0..excess);
    }

    if player_damage > 0.0 {
        game.player_hp = (game.player_hp - player_damage).max(0.0);
        game.player.take_hit();
        if game.player_hp <= 0.0 {
            game.over = true;
        }
    }

    // Kolce ranią niezależnie od pocisków — to groźba środowiskowa.
    if game.hurt_cd <= 0.0 && game.map.touches_spike(game.player.rect()) {
        game.player_hp = (game.player_hp - 18.0).max(0.0);
        game.hurt_cd = HURT_COOLDOWN;
        game.flash = 0.28;
        game.player.take_hit();
        if game.player_hp <= 0.0 {
            game.over = true;
        }
    }

    // --- timery i koniec walki ---
    game.hurt_cd = (game.hurt_cd - dt).max(0.0);
    game.flash = (game.flash - dt).max(0.0);
    game.shake = (game.shake - dt * 22.0).max(0.0);
    if game.boss.is_dead() {
        game.over = true;
        game.won = true;
    }
    // Spadnięcie z areny też znaczy porażkę — nie ma „darmowego” resetu
    // przez skok w otchłań.
    if game.map.is_below_world(game.player.pos) {
        game.over = true;
    }
}

/// Rysowanie całej sceny.
fn draw(ctx: &mut Ctx, game: &Game) {
    // Trzęsienie kamery: losowe przesunięcie zanikające w czasie.
    let (ox, oy) = shake_offset(game);

    // --- tło i kafle ---
    ctx.gfx.layer(-20).color(Color::from_hex(0x0B0D14));
    ctx.gfx.draw_rect(WORLD);
    draw_tiles(ctx, game);

    // --- boss ---
    draw_boss(ctx, game, ox, oy);

    // --- pociski ---
    ctx.gfx.layer(20);
    for b in &game.bullets {
        let p = b.pos + Vec2::new(ox, oy);
        let c = match b.owner {
            Owner::Player => Color::from_hex(0x7FE3FF),
            Owner::Boss => Color::from_hex(0xFF4D6D),
        };
        // Poświata addytywna: pocisk musi być widoczny na tle trawy
        // i w chmurze pocisków bossa.
        ctx.gfx.blend(BlendMode::Additive);
        ctx.gfx
            .color(c.with_alpha(0.22))
            .draw_circle(p, b.size.x * 1.6);
        ctx.gfx.blend(BlendMode::Alpha);
        ctx.gfx
            .rotate_about(p, b.vel.x.atan2(b.vel.y))
            .color(c)
            .draw_rect(Rect::from_size(b.size));
    }

    // --- gracz ---
    draw_player(ctx, game, ox, oy);

    draw_hud(ctx, game);
}

/// Przesunięcie kamery przy trzęsieniu.
fn shake_offset(game: &Game) -> (f32, f32) {
    if game.shake <= 0.01 {
        return (0.0, 0.0);
    }
    // Deterministyczne „losowanie" z czasu gry: dwie sinusoidy o
    // różnych częstotliwościach. `sin` zamiast RNG, żeby zrzut
    // ekranu był powtarzalny przy tych samych klatkach.
    let t = game.time * 60.0;
    ((t * 2.7).sin() * game.shake, (t * 3.9).cos() * game.shake)
}

/// Rysuje widoczne kafle: pełne bloki, platformy i kolce.
///
/// Kafle rysujemy wektorowo, bo silnik nie ma tilemapa ani atlasu
/// (patrz [`tilemap`]). Warstwy: tło → kafle → akcje, żeby boss
/// i gracz zawsze byli na wierzchu.
fn draw_tiles(ctx: &mut Ctx, game: &Game) {
    let view = ctx.camera.visible_rect(ctx.window.size);
    let tiles = game.map.visible_tiles(view);
    for (cx, cy, kind) in tiles {
        let r = TileMap::tile_rect(cx, cy);
        match kind {
            // Solidny blok: ciemny kamień z jaśniejszą obwódką na
            // górze, żeby platforma była czytelna jako „stojak”.
            Tile::Solid => {
                ctx.gfx.layer(-15).color(Color::from_hex(0x2A2F3E));
                ctx.gfx.draw_rect(r);
                ctx.gfx
                    .color(Color::from_hex(0x3E4557))
                    .draw_rect(Rect::from_xywh(r.min.x, r.top() - 3.0, r.width(), 3.0));
            }
            // Platforma: cienka listwa z zaostrzonymi końcami — czytelny
            // sygnał „można tu stanąć, ale nie z dołu".
            Tile::Platform => {
                ctx.gfx.layer(-14).color(Color::from_hex(0x6B4F2A));
                ctx.gfx
                    .draw_rect(Rect::from_xywh(r.min.x, r.top() - 8.0, r.width(), 8.0));
                ctx.gfx
                    .color(Color::from_hex(0x8A6836))
                    .draw_rect(Rect::from_xywh(r.min.x, r.top() - 8.0, r.width(), 3.0));
            }
            // Kolec: trójkąt wskazujący w górę — kierunek groźby
            // musi być czytelny bez słowa.
            Tile::Spike => {
                ctx.gfx.layer(-13).color(Color::from_hex(0x8A2B3C));
                let h = r.height() * 0.8;
                ctx.gfx.draw_polygon(&[
                    Vec2::new(r.min.x, r.min.y),
                    Vec2::new(r.center().x, r.min.y + h),
                    Vec2::new(r.max.x, r.min.y),
                ]);
            }
            Tile::Empty => {}
        }
    }
}

/// Rysuje bossa: ciało, fazę, wychył i atak.
///
/// **Wychył to element rozgrywki**, nie ozdoba: boss zmienia kolor i
/// zaczyna pulsować, gdy szykuje `GroundSlam` albo `Charge`. Bez tego
/// sygnału gracz nie ma szans odgadnąć ataku wymagającego skoku.
fn draw_boss(ctx: &mut Ctx, game: &Game, ox: f32, oy: f32) {
    let b = &game.boss;
    if b.is_dead() && b.death_time > 2.5 {
        return;
    }
    let cfg = &game.boss_cfg;
    let rect = b.rect(cfg);
    let center = b.pos + Vec2::new(ox, oy);
    let winding = b.winding();
    // Śmierć: boss blednie i rozsypuje się w dół.
    let fade = if b.is_dead() {
        (1.0 - b.death_time / 2.5).clamp(0.0, 1.0)
    } else {
        1.0
    };

    // Kolor zależy od fazy: ciepły w fazie 1, ostrzejszy w fazie 3 —
    // gracz widzi, że robi się niebezpiecznie, bez czytania HUD-a.
    let body = match b.phase {
        1 => Color::from_hex(0x8E3B5A),
        2 => Color::from_hex(0xA3325F),
        _ => Color::from_hex(0xC42A6E),
    }
    .with_alpha(fade);

    // Poświata — boss musi być rozpoznawalny w gęstwie pocisków.
    ctx.gfx.layer(10).blend(BlendMode::Additive);
    ctx.gfx
        .color(body.with_alpha(0.16 * fade))
        .draw_circle(center, rect.width() * 0.85);
    ctx.gfx.blend(BlendMode::Alpha);

    // Wychył: pulsujący pierścień narastający do ataku. Zmiana
    // rozmiaru to informacja „zaraz uderzy".
    if winding {
        let t = 1.0 - (b.windup / crate::boss::WINDUP_TIME).clamp(0.0, 1.0);
        ctx.gfx.color(Color::from_hex(0xFFD166).with_alpha(0.85));
        ctx.gfx
            .draw_ring(center, rect.width() * (0.7 + t * 0.35), 5.0);
    }

    // Korpus.
    ctx.gfx.color(body).draw_rect(rect);
    // Rdzeń: jasne „oko" w środku, pulsuje z czasem świata.
    let pulse = 1.0 + (game.time * 3.0).sin() * 0.12;
    ctx.gfx
        .color(Color::from_hex(0xFFE9F0).with_alpha(0.9 * fade));
    ctx.gfx.draw_circle(center, rect.width() * 0.17 * pulse);
    // „Ucho" w narożniku pokazuje, gdzie boss patrzy.
    let look = (game.player.center() - b.pos).normalize_or_zero();
    ctx.gfx
        .color(Color::from_hex(0xFFD166).with_alpha(fade))
        .draw_circle(center + look * rect.width() * 0.32, rect.width() * 0.09);

    // Pasek zdrowia bossa nad głową — liczba, którą gracz musi
    // śledzi, bo fazy zmieniają się na jej progu.
    let hp_w = rect.width() * 1.1;
    let hp_y = rect.top() + 18.0 + oy;
    let hp_x = center.x - hp_w * 0.5;
    ctx.gfx
        .color(Color::from_hex(0x1A1D28))
        .draw_rect(Rect::from_xywh(hp_x, hp_y, hp_w, 8.0));
    ctx.gfx.color(Color::from_hex(0xE8467C).with_alpha(fade));
    ctx.gfx
        .draw_rect(Rect::from_xywh(hp_x, hp_y, hp_w * b.hp_ratio(cfg), 8.0));
    // Podziałki faz: dwa białe kreski pokazują progi 66% i 33%.
    ctx.gfx.color(Color::from_hex(0xFFFFFF).with_alpha(0.8));
    for p in [cfg.phase2_at, cfg.phase3_at] {
        ctx.gfx
            .draw_rect(Rect::from_xywh(hp_x + hp_w * p, hp_y - 2.0, 2.0, 12.0));
    }
}

/// Rysuje gracza: ciało, oczy, animacja chodu i nietykalność.
fn draw_player(ctx: &mut Ctx, game: &Game, ox: f32, oy: f32) {
    let p = &game.player;
    let center = p.center() + Vec2::new(ox, oy);
    let rect = p.rect().translate(Vec2::new(ox, oy));
    // Migotanie przy nietykalności: sygnał dla gracza, że cios
    // właśnie przeszedł i że teraz jest bezpieczny.
    let blinking = game.hurt_cd > 0.0;
    let visible = !blinking || (game.time * 22.0).fract() < 0.55;
    if !visible {
        return;
    }

    // Poświata: gracz musi być wyróżniony spośród pocisków.
    ctx.gfx.layer(28).blend(BlendMode::Additive);
    ctx.gfx
        .color(Color::from_hex(0x4FD0FF).with_alpha(0.18))
        .draw_circle(center, rect.width() * 1.1);
    ctx.gfx.blend(BlendMode::Alpha);

    // Widoczność postaci. Gracz ma 20×34 px na 1280×720 — to **mało**,
    // bo arena jest dobrana tak, by 1 kafel = 1 px (patrz `MAP_W`).
    // Dlatego rysujemy go 1,5× i z ciemną obwódką: sylwetka musi
    // być rozpoznawalna wśród pocisków, a nie ginąć w tle.
    let s = PLAYER_DRAW_SCALE;
    let body = Rect::from_center(center, rect.size() * s);
    ctx.gfx
        .color(Color::from_hex(0x08131A))
        .draw_rect(Rect::from_center(center, body.size() + Vec2::splat(3.0)));

    // Kolor zależny od stanu: błękit = zdrowy, czerwony = ranny.
    let c = if game.flash > 0.0 {
        Color::from_hex(0xFF6B7A)
    } else {
        Color::from_hex(0x3DB8E8)
    };
    ctx.gfx.layer(30).color(c);
    ctx.gfx.draw_rect(body);

    // Oczy: przesunięte w stronę patrzenia. To jedyny element, który
    // mówi graczowi (i obserwatorowi), gdzie gracz patrzy.
    let f = p.facing;
    ctx.gfx.color(Color::from_hex(0xE8F6FF));
    for eye_x in [-5.0f32, 5.0] {
        ctx.gfx.draw_circle(
            center + Vec2::new(eye_x * s + f * 2.0, body.height() * 0.5 - 7.0),
            2.6,
        );
    }

    // Lufa przy strzale: krótka linia w kierunku patrzenia.
    if game.attacking {
        ctx.gfx.color(Color::from_hex(0x7FE3FF));
        ctx.gfx.draw_line(
            center + Vec2::new(f * 14.0, 0.0),
            center + Vec2::new(f * 30.0, 0.0),
            4.0,
        );
    }
}

/// Przeskalowanie postaci przy rysowaniu — patrz [`draw_player`].
///
/// Kolizje używają 20×34 px (`player::BODY_W/H`), ale rysunek jest
/// 1,5× większy. Rozjazd jest tu celowy: hitbox musi być **mniejszy**
/// niż sylwetka, inaczej gracze biiliby się o coś, co wygląda, jak
/// jest obok.
const PLAYER_DRAW_SCALE: f32 = 1.5;

/// HUD w przestrzeni ekranu: paski, nazwa ataku, podpowiedzi, koniec.
///
/// UWAGA: tylko ASCII. Polskie diakrytyczne wywołują błąd atlasu glifów
/// w `uran-render` (patrz `draw_hud` w `uran-game`).
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let Some(font) = game.font else { return };
    let size = ctx.window.logical_size();
    ctx.gfx.screen_space();

    // --- pasek zdrowia gracza (lewy górny róg) ---
    let bar = Rect::from_xywh(24.0, 24.0, 240.0, 18.0);
    ctx.gfx.color(Color::from_hex(0x1A1D28)).draw_rect(bar);
    let ratio = (game.player_hp / PLAYER_MAX_HP).clamp(0.0, 1.0);
    ctx.gfx
        .color(if game.hurt_cd > 0.0 {
            Color::from_hex(0xE8467C)
        } else {
            Color::from_hex(0x3DB8E8)
        })
        .draw_rect(Rect::from_xywh(
            bar.min.x,
            bar.min.y,
            bar.width() * ratio,
            bar.height(),
        ));
    ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
        font,
        &format!("HP {}", game.player_hp.ceil() as i32),
        Vec2::new(32.0, 27.0),
        14.0,
        TextAlign::Left,
    );

    // --- nazwa ataku bossa (środek góry) ---
    //
    // Pokazujemy **w czasie wychyłu**, żeby gracz miał czas zaplanować
    // ruch. Sam tekst nie wystarczy — obok niego pulsuje pierścień.
    if game.boss.winding() {
        let label = match game.boss.attack {
            Attack::GroundSlam => "FALA ZIEMI",
            Attack::Charge => "SZARZA",
            other => {
                let _ = other;
                ""
            }
        };
        ctx.gfx.color(Color::from_hex(0xFFD166)).draw_text(
            font,
            label,
            Vec2::new(size.x * 0.5, 26.0),
            26.0,
            TextAlign::Center,
        );
    }

    // --- pasek bossa u góry (środek) ---
    let bw = 460.0;
    let bbar = Rect::from_xywh(size.x * 0.5 - bw * 0.5, 56.0, bw, 14.0);
    ctx.gfx.color(Color::from_hex(0x1A1D28)).draw_rect(bbar);
    ctx.gfx
        .color(Color::from_hex(0xE8467C))
        .draw_rect(Rect::from_xywh(
            bbar.min.x,
            bbar.min.y,
            bbar.width() * game.boss.hp_ratio(&game.boss_cfg),
            bbar.height(),
        ));
    ctx.gfx.color(Color::from_hex(0xFFFFFF).with_alpha(0.85));
    for p in [game.boss_cfg.phase2_at, game.boss_cfg.phase3_at] {
        ctx.gfx.draw_rect(Rect::from_xywh(
            bbar.min.x + bbar.width() * p,
            bbar.min.y - 3.0,
            2.0,
            bbar.height() + 6.0,
        ));
    }
    ctx.gfx.color(Color::from_hex(0xB9C4D4)).draw_text(
        font,
        &format!("FAZA {}", game.boss.phase),
        Vec2::new(size.x * 0.5, 76.0),
        16.0,
        TextAlign::Center,
    );

    // --- podpowiedź (prawy dolny róg) ---
    //
    // **Prawy**, nie lewy: gracz startuje w lewej części areny i
    // podpowiedź w lewym rogu zasłaniała mu sylwetkę. Po jednej
    // stronie jest pole, po drugiej sterowanie.
    ctx.gfx.color(Color::from_hex(0x8FA6C4)).draw_text(
        font,
        "AD ruch   spacja skok   J atak   R restart",
        Vec2::new(size.x - 24.0, size.y - 32.0),
        16.0,
        TextAlign::Right,
    );

    // --- koniec walki ---
    if game.over {
        // Przyciemnienie: sygnalizuje, że scena przestała być żywa.
        ctx.gfx.color(Color::from_hex(0x000000).with_alpha(0.55));
        ctx.gfx.draw_rect(Rect::from_size(size));
        let (msg, col) = if game.won {
            ("BOSS POKONANY", Color::from_hex(0x7FE3A0))
        } else {
            ("PORAZKA  -  R aby sprobowac", Color::from_hex(0xE8467C))
        };
        ctx.gfx.color(col).draw_text(
            font,
            msg,
            Vec2::new(size.x * 0.5, size.y * 0.44),
            40.0,
            TextAlign::Center,
        );
    }
}

/// Ścieżki czcionek próbowane po kolei.
///
/// `UranSans.ttf` **nie ma w repozytorium** — to nazwa projektowa,
/// nie istniejący plik. Dlatego lista kończy się na czcionkach
/// systemowych (`/usr/share/fonts/...`), które są na każdej Linuksie
/// z pakietem `fonts-dejavu`. Bez tej listy HUD jest pusty i wygląda
/// jak błąd renderowania.
///
/// Ten sam zestaw co w `uran-game` — jedna lista w całym repo
/// oznacza jedno miejsce do poprawki.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/TTF/Hack-Regular.ttf",
    "/usr/share/fonts/TTF/MesloLGS-NF-Regular.ttf",
];

/// Jednorazowa konfiguracja: czcionka i kamera.
fn setup(ctx: &mut Ctx, game: &mut Game) {
    if game.font.is_none() {
        for path in FONT_FALLBACKS {
            if let Ok(h) = ctx.assets.load_font(path) {
                game.font = Some(h);
                break;
            }
        }
    }
    if game.font.is_none() {
        eprintln!("⚠️  nie znaleziono czcionki TTF — HUD bedzie pusty");
    }
    ctx.clear_color = Color::from_hex(0x05070C);
    // Kamera pokazuje całą arenę: walka z bossem musi być czytelna
    // w całości, a nie śledzić gracza jak w platformówce.
    ctx.camera.fit_world(WORLD, ctx.window.size);
}

fn main() {
    let game = Arc::new(Mutex::new(Game::new()));

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
            update(ctx, &mut guard);
        }
    };
    let draw_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let guard = game.lock().expect("lock gry");
            draw(ctx, &guard);
        }
    };

    App::new()
        // Zrzut po 90 klatkach (~1,5 s walki) — używany w testach
        // regresji wizualnej (`--screenshot /tmp/shot.png`).
        .screenshot(screenshot_path(), 90)
        .window(
            windowed(1280, 720)
                .title("Boss Fight")
                .background(0x05070C)
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
