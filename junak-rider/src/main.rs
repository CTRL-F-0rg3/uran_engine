//! Junak Rider — przejażdżka motocyklem po prostej drodze.
//!
//! Model 3D wczytujemy z pliku `.obj` prosto z Blendera (patrz
//! `uran_render3d::import`), a nie rysujemy bryłkami w kodzie — dzięki
//! temu podmiana modelu na inny motocykl to podmiana pliku, nic więcej.
//!
//! Sterowanie: `W`/`S` gaz i hamulec, `A`/`D` skręt, `R` restart.

mod bike;
mod road;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_math::Mat4;
use uran_render3d::import;
use uran_render3d::mesh::model_matrix;
use uran_render3d::{DrawCmd, MeshId, Renderer3d};

use bike::{Bike, BikeConfig};

/// Ścieżka modelu, szukana w kilku miejscach.
///
/// Ścieżka względna sama w sobie działa tylko z katalogu `junak-rider/`,
/// a `cargo run -p junak-rider` uruchamia grę z katalogu repozytorium —
/// bez fallbacku model by się nie wczytał i dostalibyśmy klocek
/// awaryjny zamiast motocykla.
const MODEL_FILE: &str = "junak m10.obj";

/// Znajduje plik modelu: najpierw obok programu, potem w katalogu crate'a.
fn find_model() -> Option<std::path::PathBuf> {
    let mut tried = Vec::new();

    // 1. ścieżka podana jawnie
    if let Ok(p) = std::env::var("JUNAK_MODEL") {
        let p = std::path::PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        tried.push(p);
    }

    // 2. zwykła ścieżka względna (uruchomienie z katalogu crate'a)
    let rel = std::path::PathBuf::from("assets").join(MODEL_FILE);
    if rel.is_file() {
        return Some(rel);
    }
    tried.push(rel);

    // 3. katalog źródłowy crate'a — niezawodne przy `cargo run` z roota
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join(MODEL_FILE);
    if manifest.is_file() {
        return Some(manifest);
    }
    tried.push(manifest);

    eprintln!("⚠️  nie znaleziono modelu, próbowałem:");
    for p in &tried {
        eprintln!("      {}", p.display());
    }
    None
}

/// Model po imporcie w dzielnikach osi to 2,49 × 1,20 × 1,27 m, czyli
/// realny motocykl. Skalujemy do wysokości 1,2 m, żeby niezależnie od
/// skali eksportu z Blendera model zawsze miał sensowne wymiary.
const BIKE_HEIGHT: f32 = 1.2;

/// Przesunięcie modelu względem początku siatki.
///
/// Import stawia model NA ziemi (min Y = 0) i wyśrodkowuje go w XZ,
/// ale oś obrotu silnika to środek kadła, a nie środek całej bryły —
/// dlatego podnościmy o połowę wysokości siedzenia i cofnęciem w osi Z.
const BIKE_LIFT: f32 = 0.0;

/// Kamera: jak daleko za motocyklem i jak wysoko.
const CAM_BACK: f32 = 6.5;
const CAM_UP: f32 = 2.35;
/// Jak bardzo kamera pochyla się podczas skrętu (radiany).
const CAM_SIDE: f32 = 1.1;

/// Identyfikatory siatek w rejestrze renderera.
struct Meshes {
    bike: Vec<MeshId>,
    asphalt: MeshId,
    verge: MeshId,
    paint: MeshId,
}

struct Game {
    scene: Arc<Mutex<Renderer3d>>,
    meshes: Meshes,
    bike: Bike,
    cfg: BikeConfig,
    /// Przebyta droga (m) — do HUD, nigdy nie rośnie w nieskończoność
    /// w jednej sesji, więc `f32` wystarcza.
    distance: f32,
    top_speed: f32,
    time: f32,
    font: Option<Font>,
}

impl Game {
    fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
        let mut scene = Renderer3d::new(gpu, format, samples);
        // Niskie słońce długo rysuje cień motocykla i podkreśla
        // kształt nadwozia. Bez cienia bryła wygląda jak płaska naklejka.
        scene.set_clear_color([0.45, 0.60, 0.82, 1.0]);
        {
            let l = scene.lighting_mut();
            l.light_dir = Vec3::new(0.35, 0.42, 0.84);
            l.light_color = [1.0, 0.95, 0.86];
            l.ambient = [0.34, 0.44, 0.62];
        }
        {
            let cam = scene.camera_mut();
            cam.fov_y = std::f32::consts::FRAC_PI_2 * 0.82;
            cam.near = 0.15;
            cam.far = 900.0;
        }

        // --- model z pliku .obj
        let mut bike_parts = Vec::new();
        let loaded = find_model().and_then(|p| {
            match import::obj::load_from_file(&p) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("⚠️  nie udało się wczytać `{}`: {e}", p.display());
                    None
                }
            }
        });

        match loaded {
            Some(mut model) => {
                model.scale_to_height(BIKE_HEIGHT);
                let s = model.bounds_size();
                println!(
                    "🛵  model: {} części, {:.2} × {:.2} × {:.2} m",
                    model.parts.len(),
                    s.x,
                    s.y,
                    s.z
                );
                for part in &model.parts {
                    bike_parts.push(scene.add_mesh(gpu, &part.mesh, &part.name));
                }
            }
            None => {
                // Gra ma działać nawet bez pliku — pokazujemy wtedy
                // czerwony klocek, żeby było widać, że czegoś brakuje,
                // zamiast dostawać pusty kadr.
                let mut m = uran_render3d::Mesh::new();
                m.tri(
                    uran_render3d::Vertex::new(
                        Vec3::new(-0.4, 0.0, -0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                    uran_render3d::Vertex::new(
                        Vec3::new(0.4, 0.0, -0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                    uran_render3d::Vertex::new(
                        Vec3::new(0.0, 1.2, -0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                );
                m.tri(
                    uran_render3d::Vertex::new(
                        Vec3::new(-0.4, 0.0, 0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                    uran_render3d::Vertex::new(
                        Vec3::new(0.4, 0.0, 0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                    uran_render3d::Vertex::new(
                        Vec3::new(0.0, 1.2, 0.4),
                        Vec3::X,
                        [0.9, 0.2, 0.2],
                    ),
                );
                bike_parts.push(scene.add_mesh(gpu, &m, "fallback tetra"));
            }
        }

        // --- droga
        let r = road::build_road();
        let asphalt = scene.add_mesh(gpu, &r.asphalt, "asfalt");
        let verge = scene.add_mesh(gpu, &r.verge, "pobocze");
        let paint = scene.add_mesh(gpu, &r.paint, "oznakowanie");

        Self {
            scene: Arc::new(Mutex::new(scene)),
            meshes: Meshes {
                bike: bike_parts,
                asphalt,
                verge,
                paint,
            },
            bike: Bike::default(),
            cfg: BikeConfig::default(),
            distance: 0.0,
            top_speed: 0.0,
            time: 0.0,
            font: None,
        }
    }
}

/// Obrot modelu względem osi obrotu silnika.
///
/// Po imporcie model jest ustawiony oryginalnie z Blendera: przód
/// wskazuje `-X`. Silnik jedzie po `+Z` przy `yaw = 0`, więc trzeba
/// obrócić bryłę o 90° wokół Y. Gdyby ta stała była zła, motocykl
/// jechałby bokiem do drogi — stąd osobna nazwa i komentarz.
const BIKE_YAW_OFFSET: f32 = std::f32::consts::FRAC_PI_2;

/// Macierz modelu motocykla w świecie.
///
/// Przechył to `roll` wokół osi Z lokalnej (osi kierunku jazdy), więc
/// motocykl kładzie się w bok, a nie pochyla się do przodu.
fn bike_matrix(b: &Bike) -> Mat4 {
    model_matrix(
        b.pos + Vec3::new(0.0, BIKE_LIFT, 0.0),
        b.yaw + BIKE_YAW_OFFSET,
        0.0,
        b.lean,
        Vec3::ONE,
    )
}

/// Kamera podąża za motocyklem.
fn update_camera(game: &mut Game) {
    let b = &game.bike;
    let f = b.forward();
    // Kamerę trzymamy ZA motocykłem, ale nie na jego osi: przy pełnym
    // skręcie kamera zjechałaby z drogi. Dlatego liczymy ją względem
    // kierunku jazdy, a nie względem pozycji bezwzględnej.
    let side = b.right() * (b.steer / game.cfg.max_steer) * CAM_SIDE;
    let eye = b.pos - f * CAM_BACK + side + Vec3::new(0.0, CAM_UP, 0.0);
    // Celujemy przed motocykl, nie w niego — inaczej kamera patrzyłaby
    // w ogon i drogi by nie było widać.
    let look = b.pos + f * 14.0 + Vec3::new(0.0, 1.2, 0.0);

    if let Ok(mut scene) = game.scene.lock() {
        let cam = scene.camera_mut();
        cam.position = eye;
        cam.target = look;
    }
}

/// Lista rysowania na bieżącą klatkę.
fn build_draw_list(game: &mut Game) {
    let mut cmds = Vec::with_capacity(game.meshes.bike.len() + 3);
    let m = bike_matrix(&game.bike);
    for id in &game.meshes.bike {
        cmds.push(DrawCmd::new(*id, m));
    }
    // Droga leży w miejscu — jej transformacja to tylko przesunięcie
    // świata, a ono wynika z cofnięcia motocykla (patrz `update`).
    let z = game.bike.pos.z;
    let road = model_matrix(Vec3::new(0.0, 0.0, z), 0.0, 0.0, 0.0, Vec3::ONE);
    cmds.push(DrawCmd::new(game.meshes.verge, road));
    cmds.push(DrawCmd::new(game.meshes.asphalt, road));
    cmds.push(DrawCmd::new(game.meshes.paint, road));

    if let Ok(mut scene) = game.scene.lock() {
        scene.set_commands(cmds);
    }
}

/// Krok symulacji + wejście.
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
        // Pozycję czytamy PRZED pożyczką — `reset` bierze `&mut bike`,
        // a `game.bike.pos.z` w tej samej wyrażeniu byłoby drugim
        // pożyczeniem tego samego pola.
        let z = game.bike.pos.z;
        bike::reset(&mut game.bike, z);
        game.distance = 0.0;
        game.top_speed = 0.0;
    }

    // `axis(a, b)` daje -1/+1; przy W (gaz) ma być dodatnia.
    let throttle = ctx.input.axis(Key::KeyS, Key::KeyW);
    let steer = ctx.input.axis(Key::KeyA, Key::KeyD);
    bike::step(&mut game.bike, dt, throttle, steer, &game.cfg);

    // --- przebyta droga i rekord
    //
    // Liczymy ze *zmian* pozycji, a nie z samej prędkości: przy cofaniu
    // prędkość jest ujemna i dystans malałby wtedy, gdy jedziemy do
    // przodu. Używamy tu wartości bezwzględnej.
    game.distance += game.bike.speed.abs() * dt;
    game.top_speed = game.top_speed.max(game.bike.speed);

    // --- nieskończona droga: przesuwamy świat
    //
    // Droga jest jednym pasmem zbudowanym w miejscu. Gdy motocykl
    // zje daleko, cofamy go o stały krok — droga „przesuwa się" pod
    // nim bez rysowania czegokolwiek nowego.
    if game.bike.pos.z > road::RECENTER_AT {
        game.bike.pos.z -= road::RECENTER_STEP;
    } else if game.bike.pos.z < -road::RECENTER_AT {
        game.bike.pos.z += road::RECENTER_STEP;
    }

    update_camera(game);
    build_draw_list(game);
}

/// Predkosciomierz i wskaznik skretu.
///
/// HUD rysujemy w przestrzeni EKRANU (renderer 2D) — tak samo jak
/// w `uran-tanks`, i z tej samego powodu: 2D nie przechodzi przez
/// post-processing 3D i nie zostaje zniekształcony przez ACES.
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let size = ctx.window.logical_size();
    let kmh = game.bike.speed.abs() * 3.6;

    // --- wskaznik skretu: pasek u dołu ekranu
    //
    // Skręt w prawo daje ujemne `steer`, a na ekranie chcemy, żeby
    // kierownica szła w prawo. Dlatego `steer` odwracamy znakiem.
    let steer = -game.bike.steer / game.cfg.max_steer;
    let half = size.x * 0.18;
    let cx = size.x * 0.5;
    let y = size.y - 46.0;
    ctx.gfx
        .screen_space()
        .layer(100)
        .color(Color::from_hex(0x141A24))
        .draw_rect(Rect::new(
            Vec2::new(cx - half, y),
            Vec2::new(cx + half, y + 10.0),
        ));
    let knob = half * steer;
    let tint = if steer.abs() > 0.85 {
        Color::from_hex(0xFFB347)
    } else {
        Color::from_hex(0x66DDAA)
    };
    ctx.gfx.color(tint).draw_rect(Rect::new(
        Vec2::new(cx + knob - 3.0, y - 2.0),
        Vec2::new(cx + knob + 3.0, y + 12.0),
    ));

    // --- pasek gazu
    let t = (ctx.input.axis(Key::KeyS, Key::KeyW) + 1.0) * 0.5;
    ctx.gfx
        .color(Color::from_hex(0x3A4A63))
        .draw_rect(Rect::new(
            Vec2::new(24.0, size.y - 74.0),
            Vec2::new(24.0 + 220.0, size.y - 64.0),
        ));
    ctx.gfx.color(Color::from_hex(0x66CCFF)).draw_rect(Rect::new(
        Vec2::new(26.0, size.y - 72.0),
        Vec2::new(26.0 + 216.0 * t, size.y - 66.0),
    ));

    let Some(font) = game.font else { return };

    // --- predkosciomierz
    //
    // UWAGA: tylko znaki ASCII. Polskie diakrytyczne wywoluja blad
    // atlasu glifow w `uran-render` przy przepelnieniu (patrz `draw_hud`
    // w uran-tanks), wiec HUD zostaje czytelny bez polskich znakow.
    ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
        font,
        &format!("{kmh:.0}"),
        Vec2::new(cx, size.y - 118.0),
        54.0,
        TextAlign::Center,
    );
    ctx.gfx.color(Color::from_hex(0x7A8CA8)).draw_text(
        font,
        "km/h",
        Vec2::new(cx, size.y - 60.0),
        18.0,
        TextAlign::Center,
    );

    // Wskazujemy promień skrętu tylko przy wyraźnym wychyle — przy
    // kierownicy wprost jest nieskończony i `format!` wypisałoby `inf`.
    let radius = game.bike.turn_radius(&game.cfg);
    let radius_txt = if radius.is_finite() {
        format!("R {:.0} m", radius)
    } else {
        "R -".to_string()
    };

    ctx.gfx.color(Color::from_hex(0x8FA6C4)).draw_text(
        font,
        &format!(
            "distance {:.0} m    top {:.0} km/h    {}    parts {}",
            game.distance,
            game.top_speed * 3.6,
            radius_txt,
            game.meshes.bike.len()
        ),
        Vec2::new(24.0, 26.0),
        16.0,
        TextAlign::Left,
    );

    let fps = ctx.time.fps();
    ctx.gfx.color(Color::from_hex(0x6C7A90)).draw_text(
        font,
        &format!(
            "{fps:.0} FPS   W gas   S brake   A/D steer   R restart   ESC quit"
        ),
        Vec2::new(24.0, 48.0),
        14.0,
        TextAlign::Left,
    );
}

/// Czcionki systemowe — sprawdzane w kolejności, gdy brak wlasnej.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
];

/// Ścieżka zrzutu z linii poleceń.
fn screenshot_path() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
}

/// Proxy 3D oddawane do renderera 2D.
///
/// Renderer 2D musi wywołać `draw` na scenie, a w tej samej chwili
/// gra czyta i zmienia tę samą scenę (kamera, lista rysowania).
/// Rozwiązanie: obaj trzymamy `Arc<Mutex<..>>` — proxy blokuje i rysuje,
/// gra blokuje i aktualizuje.
///
/// Dlaczego `Mutex`, a nie `Rc<RefCell>`: `App` wymaga systemów `Send`,
/// a `Rc` nie jest `Send`.
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

/// Wczytuje czcionkę — bez niej HUD działa, ale bez tekstu.
fn setup(ctx: &mut Ctx, game: &mut Game) {
    for path in FONT_FALLBACKS {
        if let Ok(h) = ctx.assets.load_font(path) {
            game.font = Some(h);
            return;
        }
    }
}

fn main() {
    // Stan gry dzielony między systemy — oba blokują ten sam mutex.
    let game = Arc::new(Mutex::new(None::<Game>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if guard.is_some() {
                return;
            }
            // JEDEN `Device` obsługuje oba renderery (patrz
            // `uran_render::scene3d`), a `samples` MUSI się zgadzać —
            // inaczej hdr-owa tekstura post-processingu nie przejdzie
            // walidacji wgpu.
            let built = {
                let r = ctx.renderer.as_ref().expect("renderer gotowy");
                Game::new(r.gpu(), r.format(), r.samples())
            };
            *guard = Some(built);
            let g = guard.as_mut().expect("właśnie utworzone");
            setup(ctx, g);
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
            let guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_ref() {
                draw_hud(ctx, g);
            }
        }
    };

    App::new()
        .window(
            windowed(1280, 720)
                .title("Junak Rider")
                .background(0x7BA3CC)
                .vsync(true)
                .samples(1),
        )
        .screenshot(screenshot_path(), 30)
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}
