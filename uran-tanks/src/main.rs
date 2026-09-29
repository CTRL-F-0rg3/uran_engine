//! Uran Tanks — FPS z czołgami, w stylu World of Tanks.
//!
//! # Jak to działa
//!
//! Świat 3D rysuje `uran-render3d` we WŁASNYM render passie z własnym
//! buforem głębokości. HUD, celownik i panele idą normalnym rendererem 2D
//! silnika (`uran-render`) w DRUGIM passie z `LoadOp::Load` — czyli
//! nakładają się na świat 3D, a nie chodzą pod nim.
//!
//! Oba renderery dzielą jedno `Device`, jedną `Queue` i jedną
//! `Surface`, ale zero potoków, buforów i shaderów.
//!
//! Sterowanie: WASD / S — gaz, A/D — skręt kadłuba, mysz — wieża,
//! LPM — ogień, R — restart.

mod geometry;
mod tank;
mod world;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_math::{Vec2, Vec3};
use uran_render3d::{DrawCmd, MeshId, Renderer3d};

use geometry::{palette, tank_cupola, tank_gun, tank_hull, tank_track, tank_turret};
use tank::Tank;
use world::World;

/// Ile przeciwników na mapie.
const ENEMY_COUNT: usize = 5;

/// Odległość kamery za kadłubem.
///
/// Dalej niż w FPS-ach, bo czołg ma 6,4 m długości: przy 11 m widać
/// tylko dach kadłuba i nie ma pojęcia, gdzie się stoją przeciwnicy.
const CAM_BACK: f32 = 17.0;
/// Wysokość kamery nad ziemią.
const CAM_UP: f32 = 6.5;

/// Identyfikatory siatek — rejestrowane raz przy starcie.
struct Meshes {
    hull: MeshId,
    track_l: MeshId,
    track_r: MeshId,
    turret: MeshId,
    gun: MeshId,
    cupola: MeshId,
    ground: MeshId,
    shell: MeshId,
    spark: MeshId,
}

/// Stan gry dzielony między systemami.
struct Game {
    world: World,
    /// Renderer 3D w `Arc<Mutex<..>>` — tę samą kopię dostaje proxy
    /// podpięte do renderera 2D (patrz `SceneProxy`).
    scene: Arc<Mutex<Renderer3d>>,
    meshes: Meshes,
    font: Option<Font>,
    time: f32,
}

impl Game {
    /// Buduje świat i renderer 3D na gotowym urządzeniu.
    fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
        let mut scene = Renderer3d::new(gpu, format, samples);
        // niebo: chłodny błękit, pasujący do mgły w shaderze
        scene.set_clear_color([0.47, 0.58, 0.70, 1.0]);

        let world = World::new(ENEMY_COUNT);
        let ground = build_ground(&world.terrain);

        let meshes = Meshes {
            hull: scene.add_mesh(gpu, &tank_hull(palette::PLAYER), "hull"),
            track_l: scene.add_mesh(gpu, &tank_track(-1.0), "track L"),
            track_r: scene.add_mesh(gpu, &tank_track(1.0), "track R"),
            turret: scene.add_mesh(gpu, &tank_turret(palette::PLAYER), "turret"),
            gun: scene.add_mesh(gpu, &tank_gun(), "gun"),
            cupola: scene.add_mesh(gpu, &tank_cupola(palette::PLAYER), "cupola"),
            ground: scene.add_mesh(gpu, &ground, "ground"),
            shell: scene.add_mesh(gpu, &geometry::box_mesh(
                Vec3::ZERO,
                Vec3::splat(0.22),
                [1.0, 0.85, 0.45],
            ), "shell"),
            spark: scene.add_mesh(gpu, &geometry::box_mesh(
                Vec3::ZERO,
                Vec3::splat(0.16),
                [1.0, 0.7, 0.25],
            ), "spark"),
        };

        Self {
            world,
            scene: Arc::new(Mutex::new(scene)),
            meshes,
            font: None,
            time: 0.0,
        }
    }

    /// Restart: nowy świat, ta sama scena 3D.
    fn restart(&mut self) {
        self.world = World::new(ENEMY_COUNT);
    }
}

/// Siatka terenu: regularna plansza, na której każdy wierzchołek
/// dostaje wysokość z `Terrain`.
///
/// Świadomie gęsta (krok 4 jednostki) — przy rzadszej siatce pagórki
/// wyglądają jak ostre kryształy, a czołgi „wiszą" nad terenem.
fn build_ground(terrain: &world::Terrain) -> uran_render3d::Mesh {
    use uran_render3d::{Mesh, Vertex};
    use uran_math::Vec3;

    let half = world::ARENA_HALF;
    let step = 4.0;
    let n = (half * 2.0 / step) as i32;
    let mut mesh = Mesh::new();

    for gz in 0..n {
        for gx in 0..n {
            let x0 = -half + gx as f32 * step;
            let z0 = -half + gz as f32 * step;
            let x1 = x0 + step;
            let z1 = z0 + step;

            let p00 = Vec3::new(x0, terrain.height(x0, z0), z0);
            let p10 = Vec3::new(x1, terrain.height(x1, z0), z0);
            let p11 = Vec3::new(x1, terrain.height(x1, z1), z1);
            let p01 = Vec3::new(x0, terrain.height(x0, z1), z1);

            // Dwa trójkąty na kwadrat, wiersze NAPRZEMIENNIE — inaczej
            // ukośna siatka daje widoczny wzór „schodków".
            //
            // Kolejność wierzchołków daje normalną W GÓRĘ: dla trójkąta
            // (p00, p11, p10) wektor `p11 - p00` poprzedza `p10 - p00`,
            // więc iloczyn kieruje się do +Y. Odwrócenie (p00, p10, p11)
            // daje -Y i cała ziemia znika przy back-face culling.
            let c = palette::GROUND;
            if (gx + gz) % 2 == 0 {
                mesh.tri(
                    Vertex::new(p00, Vec3::Y, c),
                    Vertex::new(p11, Vec3::Y, c),
                    Vertex::new(p10, Vec3::Y, c),
                );
                mesh.tri(
                    Vertex::new(p00, Vec3::Y, c),
                    Vertex::new(p01, Vec3::Y, c),
                    Vertex::new(p11, Vec3::Y, c),
                );
            } else {
                mesh.tri(
                    Vertex::new(p00, Vec3::Y, c),
                    Vertex::new(p01, Vec3::Y, c),
                    Vertex::new(p10, Vec3::Y, c),
                );
                mesh.tri(
                    Vertex::new(p10, Vec3::Y, c),
                    Vertex::new(p01, Vec3::Y, c),
                    Vertex::new(p11, Vec3::Y, c),
                );
            }
        }
    }
    mesh
}

/// Pozycja i kąty pod pojedynczy czołg (do złożenia z komponentami).
struct TankPose {
    hull: Vec3,
    hull_yaw: f32,
    /// Środek układu wieży.
    turret: Vec3,
    turret_yaw: f32,
    pitch: f32,
}

impl TankPose {
    fn of(t: &Tank) -> Self {
        Self {
            hull: t.pos,
            hull_yaw: t.heading,
            turret: t.turret_position(),
            turret_yaw: t.turret_yaw,
            pitch: t.gun_pitch,
        }
    }
}

/// Składa listę obiektów 3D na bieżącą klatkę.
///
/// Kolejność ma znaczenie tylko dla czytelności kodu — głębią i tak
/// zajmuje się depth buffer, a potok nie ma mieszania alfa.
fn build_draw_list(game: &mut Game) {
    use uran_math::Vec3;
    use uran_render3d::model_matrix;

    let mut cmds: Vec<DrawCmd> = Vec::with_capacity(128);
    let one = [1.0; 4];
    let m = &game.meshes;
    // teren
    cmds.push(DrawCmd::new(m.ground, model_matrix(
        Vec3::ZERO, 0.0, 0.0, 0.0, Vec3::ONE,
    )));

    // przeciwnicy (za dziesiątkami, żeby gracz był na wierzchu)
    for e in game.world.enemies.iter().filter(|e| e.is_alive()) {
        let p = TankPose::of(e);
        let tint = [palette::ENEMY[0], palette::ENEMY[1], palette::ENEMY[2], 1.0];
        push_tank(&mut cmds, m, p, &tint);
    }

    // pociski i iskry
    for s in game.world.shells.iter() {
        let tint = if s.from_player {
            [1.0, 0.90, 0.50, 1.0]
        } else {
            [1.0, 0.45, 0.35, 1.0]
        };
        cmds.push(DrawCmd::tinted(
            m.shell,
            model_matrix(s.pos, 0.0, 0.0, 0.0, Vec3::ONE),
            tint,
        ));
    }
    for (pos, _, _) in game.world.sparks.iter() {
        cmds.push(DrawCmd::new(
            m.spark,
            model_matrix(*pos, 0.0, 0.0, 0.0, Vec3::ONE),
        ));
    }

    // gracz na końcu — w razie wątku głębi trasa nie ma znaczenia
    if game.world.player.is_alive() {
        let p = TankPose::of(&game.world.player);
        push_tank(&mut cmds, m, p, &one);
    }

    if let Ok(mut scene) = game.scene.lock() {
        scene.set_commands(cmds);
    }
}
/// Dokłada komponenty jednego czołgu (kadłub, gąsiennice, wieża, lufa).
fn push_tank(
    cmds: &mut Vec<DrawCmd>,
    m: &Meshes,
    p: TankPose,
    tint: &[f32; 4],
) {
    use uran_math::Vec3;
    use uran_render3d::model_matrix;

    // kadłub i gąsiennice obracają się razem
    let body = model_matrix(p.hull, p.hull_yaw, 0.0, 0.0, Vec3::ONE);
    cmds.push(DrawCmd::tinted(m.hull, body, *tint));
    cmds.push(DrawCmd::tinted(m.track_l, body, *tint));
    cmds.push(DrawCmd::tinted(m.track_r, body, *tint));

    // wieża obraca się niezależnie, ale stoi w miejscu
    let turret = model_matrix(p.turret, p.turret_yaw, 0.0, 0.0, Vec3::ONE);
    cmds.push(DrawCmd::tinted(m.turret, turret, *tint));
    cmds.push(DrawCmd::tinted(m.cupola, turret, *tint));

    // lufa: pochylenie w osi X względem wieży
    let gun = model_matrix(p.turret, p.turret_yaw, p.pitch, 0.0, Vec3::ONE);
    cmds.push(DrawCmd::tinted(m.gun, gun, *tint));
}

/// Ustawia kamerę za kadłubem gracza (widok trzecioosobowy).
///
/// Celujemy w wieżę i trochę WYŻEJ niż w lufę, żeby kamera patrzyła
/// lekko w dół na przeciwników stojących na terenie. Celowanie dokładnie
/// wzdłuż lufy dawałoby kadr „ziemia tuż pod nosem" przy jej pochyleniu.
fn update_camera(game: &mut Game) {
    let t = &game.world.player;
    let eye = t.pos
        + Vec3::new(0.0, CAM_UP, 0.0)
        - t.forward() * CAM_BACK;
    // punkt, w który patrzymy: 10 m przed wieżą, lekko w górę
    let look = t.turret_position() + t.aim_dir() * 10.0 + Vec3::new(0.0, 1.0, 0.0);

    if let Ok(mut scene) = game.scene.lock() {
        let cam = scene.camera_mut();
        cam.position = eye;
        cam.target = look;
    }
}

/// Czytanie wejścia i krok symulacji.
fn update(ctx: &mut Ctx, game: &mut Game) {
    let dt = ctx.dt();
    game.time += dt;
    if let Ok(mut scene) = game.scene.lock() {
        scene.advance(dt);
    }

    if ctx.input.just_pressed(Key::Escape) {
        std::process::exit(0);
    }
    if ctx.input.just_pressed(Key::KeyR) {
        game.restart();
        return;
    }

    // --- mysz: obrót wieży
    // `mouse_delta` jest limitowany przez silnik, więc skok kursora po
    // refocus nie obróci czołgu o pół obrotu.
    let delta = ctx.input.mouse_delta();
    // czułość: 0.004 rad na piksel to wygodne tempo przy 1000 DPI
    let sens = 0.004;
    game.world.player.aim(-delta.x * sens, delta.y * sens);

    // --- gaz i skręt kadłuba
    let throttle = ctx.input.axis(Key::KeyS, Key::KeyW);
    let steer = ctx.input.axis(Key::KeyA, Key::KeyD);
    game.world.update(dt, throttle, steer);

    // --- ogień
    if ctx.input.mouse_pressed(MouseButton::Left) {
        game.world.player_fire();
    }

    // --- scena 3D na bieżącą klatkę
    update_camera(game);
    build_draw_list(game);
}

/// Celownik i HUD w przestrzeni EKRANU (renderer 2D, po 3D).
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let size = ctx.window.logical_size();
    let cx = size.x * 0.5;
    let cy = size.y * 0.5;

    // --- celownik: krzyż z „lukami", jak w czołgówkach
    let tint = if game.world.player.can_fire() {
        Color::from_hex(0x66FF99)
    } else {
        Color::from_hex(0xFF6655)
    };
    let gap = 6.0;
    let len = 10.0;
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let from = Vec2::new(cx + dx * gap, cy + dy * gap);
        let to = Vec2::new(cx + dx * (gap + len), cy + dy * (gap + len));
        ctx.gfx.color(tint).draw_line(from, to, 2.0);
    }

    // --- pasek HP gracza (u dołu ekranu)
    let w = 300.0;
    let bar = Rect::new(Vec2::new(24.0, size.y - 54.0), Vec2::new(24.0 + w, size.y - 32.0));
    ctx.gfx
        .screen_space()
        .layer(100)
        .color(Color::from_hex(0x1B2130))
        .draw_rect(bar);
    let ratio = game.world.player.hp_ratio();
    let fill = Rect::new(
        bar.min + Vec2::new(2.0, 2.0),
        Vec2::new(2.0 + (w - 4.0) * ratio, bar.max.y - 2.0),
    );
    ctx.gfx
        .color(Color::from_hex(0x44DD77).lerp(Color::from_hex(0xFF4455), 1.0 - ratio))
        .draw_rect(fill);

    let Some(font) = game.font else { return };

    // --- teksty
    //
    // UWAGA: tu celowo SAME znaki ASCII. Polskie znaki diakrytyczne
    // (ą ę ć ł ń ó ś ź ż) wywołują błąd atlasu glifów w `uran-render`
    // (patrz `FontState::rasterize` — przy przepełnieniu robi `reset_atlas()`
    // w trakcie klatki, przez co wcześniej wypisane UV-y wskazują na
    // wyczyszczony atlas i sąsiednie glify się zlewają). Dopóki ten błąd
    // nie jest naprawiony, HUD zostaje czytelny bez polskich znaków.
    ctx.gfx
        .screen_space()
        .layer(101)
        .color(Color::from_hex(0xC8D4E8))
        .draw_text(
            font,
            &format!("HP {}", game.world.player.hp.ceil() as i32),
            Vec2::new(24.0, size.y - 62.0),
            18.0,
            TextAlign::Left,
        );

    // --- pasek przeładowania przy celowniku
    let reload_ratio =
        1.0 - game.world.player.reload_left / game.world.player.config.reload;
    if reload_ratio < 1.0 {
        let rw = 90.0;
        let rbar = Rect::new(
            Vec2::new(cx - rw * 0.5, cy + 28.0),
            Vec2::new(cx + rw * 0.5, cy + 34.0),
        );
        ctx.gfx.color(Color::from_hex(0x1B2130)).draw_rect(rbar);
        let f = Rect::new(
            rbar.min + Vec2::new(1.0, 1.0),
            Vec2::new(
                rbar.min.x + 1.0 + (rw - 2.0) * reload_ratio,
                rbar.max.y - 1.0,
            ),
        );
        ctx.gfx.color(Color::from_hex(0xE0A040)).draw_rect(f);
    }

    // --- info: przeciwnicy, cele, FPS
    ctx.gfx
        .color(Color::from_hex(0x8FA6C4))
        .draw_text(
            font,
            &format!(
                "Enemies: {}   hits: {}   kills: {}",
                game.world.enemies_left(),
                game.world.player_hits,
                game.world.enemy_kills
            ),
            Vec2::new(24.0, 26.0),
            16.0,
            TextAlign::Left,
        );

    let fps = ctx.time.fps();
    let objects = game.scene.lock().map(|s| s.last_draw_count).unwrap_or(0);
    ctx.gfx
        .color(Color::from_hex(0x6C7A90))
        .draw_text(
            font,
            &format!(
                "{fps:.0} FPS   3D objects: {objects}   WASD drive   mouse turret   LMB fire   R restart"
            ),
            Vec2::new(24.0, 48.0),
            14.0,
            TextAlign::Left,
        );

    // --- koniec gry
    if game.world.is_over() {
        ctx.gfx
            .screen_space()
            .layer(110)
            .color(Color::from_hex(0x140A0A).with_alpha(0.72))
            .draw_rect(Rect::from_center(size * 0.5, size));
        ctx.gfx
            .color(Color::from_hex(0xFF6B6B))
            .draw_text(
                font,
                "TANK DESTROYED",
                Vec2::new(cx, cy - 10.0),
                44.0,
                TextAlign::Center,
            );
        ctx.gfx
            .color(Color::from_hex(0xC8D4E8))
            .draw_text(
                font,
                &format!(
                    "hits: {}   kills: {}   R restart",
                    game.world.player_hits, game.world.enemy_kills
                ),
                Vec2::new(cx, cy + 30.0),
                18.0,
                TextAlign::Center,
            );
    }
}

/// Czcionki systemowe — sprawdzane w kolejności, gdy brak własnej.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
];

/// Proxy 3D oddawane do renderera 2D.
///
/// Problem własności: renderer 2D musi wywołać `draw(&mut self)` na
/// scenie, a w tym samym czasie system gry czyta i zmienia tę samą
/// scenę (kamera, lista obiektów). Rozwiązanie: obaj trzymamy ten sam
/// `Arc<Mutex<..>>` — proxy blokuje i rysuje, gra blokuje i aktualizuje.
///
/// Dlaczego `Mutex`, a nie `Rc<RefCell>`: `App` wymaga systemów `Send`,
/// więc cały stan gry musi być `Send`, a `Rc` nie jest.
struct SceneProxy(Arc<Mutex<Renderer3d>>);

impl uran_render::Scene3d for SceneProxy {
    fn draw(&mut self, target: &uran_render::Scene3dTarget<'_>) {
        if let Ok(mut scene) = self.0.lock() {
            uran_render::Scene3d::draw(&mut *scene, target);
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if let Ok(mut scene) = self.0.lock() {
            uran_render::Scene3d::resize(&mut *scene, w, h);
        }
    }
}

/// Start: tworzymy renderer 3D na gotowym urządzeniu i podpinamy go
/// jako scenę 3D renderera 2D.
///
/// JEDEN `Device` obsługuje oba renderery — patrz
/// `uran_render::scene3d` wyjaśnienie, dlaczego osobne `Surface` nie
/// wchodzi w grę.
fn setup(ctx: &mut Ctx, game: &mut Game) {
    game.font = ctx.load_font(FONT_FALLBACKS[0]);
    for path in FONT_FALLBACKS {
        if game.font.is_some() {
            break;
        }
        if let Ok(h) = ctx.assets.load_font(path) {
            game.font = Some(h);
        }
    }
}

/// Ścieżka zrzutu z linii poleceń.
fn screenshot_path() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}

fn main() {
    // Stan gry dzielony między systemy — klasyczny „shared game state".
    let game = Arc::new(Mutex::new(None::<Game>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if guard.is_some() {
                return;
            }
            // `Renderer::gpu()` daje dostęp do tego samego urządzenia,
            // którego używa renderer 2D; format i MSAA muszą się zgadzać
            let built = {
                let r = ctx.renderer.as_ref().expect("renderer gotowy");
                Game::new(r.gpu(), r.format(), r.samples())
            };
            *guard = Some(built);
            let g = guard.as_mut().expect("właśnie utworzone");
            setup(ctx, g);
            // podpięcie sceny 3D do renderera 2D
            let proxy = Box::new(SceneProxy(Arc::clone(&g.scene)));
            if let Some(r) = ctx.renderer.as_mut() {
                r.set_scene3d(proxy);
            }
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
            let mut guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_ref() {
                draw_hud(ctx, g);
            }
        }
    };

    App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Tanks — czołgi 3D")
                .background(0x7894B2)
                .vsync(true)
                .samples(1),
        )
        .screenshot(screenshot_path(), 30)
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_faces_upwards() {
        // Cała ziemia znika przy `cull_mode: Back`, jeśli trójkąty mają
        // odwróconą windę. Ten test chroni najdroższy błąd: debugowanie
        // „dlaczego nie ma podłogi" po odrzuceniu 4000 trójkątów.
        let mut rng = world::Rng::new(7);
        let terrain = world::Terrain::new(&mut rng);
        let mesh = build_ground(&terrain);
        assert!(mesh.vertices.len() > 1000, "teren jest pusty: {}", mesh.vertices.len());
        for (i, v) in mesh.vertices.iter().enumerate() {
            assert_eq!(
                v.normal(),
                Vec3::Y,
                "wierzchołek {i} ma normalną {:?} zamiast +Y (wiatrak nie zadziała)",
                v.normal()
            );
        }
    }

    #[test]
    fn ground_covers_the_whole_arena() {
        let mut rng = world::Rng::new(7);
        let terrain = world::Terrain::new(&mut rng);
        let mesh = build_ground(&terrain);
        let min_x = mesh.vertices.iter().map(|v| v.pos().x).fold(f32::MAX, f32::min);
        let max_x = mesh.vertices.iter().map(|v| v.pos().x).fold(f32::MIN, f32::max);
        assert!(min_x <= -world::ARENA_HALF + 0.1, "teren nie dochodzi do lewej krawędzi");
        assert!(max_x >= world::ARENA_HALF - 0.1, "teren nie dochodzi do prawej krawędzi");
    }

    #[test]
    fn ground_follows_terrain_height() {
        // wierzchołki siatki muszą leżeć na powierzchni, inaczej
        // czołgi wiszą w powietrzu albo toną w ziemi
        let mut rng = world::Rng::new(7);
        let terrain = world::Terrain::new(&mut rng);
        let mesh = build_ground(&terrain);
        for v in mesh.vertices.iter().take(200) {
            let p = v.pos();
            let h = terrain.height(p.x, p.z);
            assert!(
                (p.y - h).abs() < 1e-3,
                "wierzchołek {:?} leży {h} od gruntu",
                p
            );
        }
    }
}