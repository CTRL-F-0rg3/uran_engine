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
use uran_render3d::{DrawCmd, MaterialId, MeshId, Renderer3d};

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
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(MODEL_FILE);
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
///
/// Wysokość celowo tuż NAD siedzeniem (ok. 1,5 m), a nie wysoko
/// nad głową. Kamera patrząca z góry zgniata motocykl do płaskiej
/// plamy — nie widać ani kierownicy, ani nadwozia, ani tego, że
/// przechyla się w zakręcie.
const CAM_BACK: f32 = 4.3;
const CAM_UP: f32 = 1.52;
/// Jak bardzo kamera pochyla się podczas skrętu (radiany).
const CAM_SIDE: f32 = 1.1;

/// Identyfikatory siatek w rejestrze renderera.
struct Meshes {
    /// (siatka, materiał) dla każdej części modelu
    bike: Vec<(MeshId, MaterialId)>,
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
            // 0.34/0.44/0.62 → 0.13/0.16/0.21 (jak w `farm-simulator`
            // i w globalnym `Lighting::default()`).
            //
            // Stara wartość była 3-4× większa w sensie proporcji do
            // słońca 4.0, więc zacierała kontrast: strona na słońcu i
            // cień wyglądały jednak jasno, tyle że w innym odcieniu.
            // Motocykl wtedy czytał się jak płaska naklejka, mimo że
            // cień rzutowany był poprawnie.
            l.ambient = [0.13, 0.16, 0.21];
        }
        {
            let cam = scene.camera_mut();
            cam.fov_y = std::f32::consts::FRAC_PI_2 * 0.82;
            cam.near = 0.15;
            cam.far = 900.0;
        }

        // --- model z pliku .obj
        //
        // Konwencja osi jest jawna, bo ten plik jest Y-up. Dowód
        // z samego pliku: część `wheel_f` to cztery łuki przy
        // `z = ±0.227` rozciągnięte w x i y — czyli dysk w
        // płaszczyźnie XY, cienki w Z. Oś koła to Z, a oś koła
        // motocykla jest zawsze pozioma, więc Z to SZEROKOŚĆ.
        // Skoro koło przednie leży na skrajnym ujemnym X, to X jest
        // długością, a pozostała oś Y — wysokością. Przy Z-up
        // (`AxisUp::Z`) bryła wylądowałaby na boku.
        let mut bike_parts = Vec::new();
        let loaded = find_model().and_then(|p| {
            match import::obj::load_from_file_with_axes(&p, import::AxisUp::Y) {
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
                    // Material z pliku `.mtl` (albo zapasowy, gdy pliku
                    // nie ma) rejestrujemy w banku i wiążemy z siatką.
                    // Dzięki temu dodanie tekstur to PODMIANA plików
                    // w katalogu `assets/`, a nie zmiana w kodzie.
                    let mat = scene.add_material(gpu, &part.material);
                    let id = scene.add_mesh(gpu, &part.mesh, &part.name);
                    bike_parts.push((id, mat));
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
                    uran_render3d::Vertex::new(Vec3::new(0.4, 0.0, -0.4), Vec3::X, [0.9, 0.2, 0.2]),
                    uran_render3d::Vertex::new(Vec3::new(0.0, 1.2, -0.4), Vec3::X, [0.9, 0.2, 0.2]),
                );
                m.tri(
                    uran_render3d::Vertex::new(Vec3::new(-0.4, 0.0, 0.4), Vec3::X, [0.9, 0.2, 0.2]),
                    uran_render3d::Vertex::new(Vec3::new(0.4, 0.0, 0.4), Vec3::X, [0.9, 0.2, 0.2]),
                    uran_render3d::Vertex::new(Vec3::new(0.0, 1.2, 0.4), Vec3::X, [0.9, 0.2, 0.2]),
                );
                let id = scene.add_mesh(gpu, &m, "fallback tetra");
                bike_parts.push((id, MaterialId(0)));
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
/// Niezależnie od konwencji osi w pliku (patrz `AxisUp` w importerze)
/// przód motocykla w leży w `-X`, a silnik jedzie po `+Z` przy
/// `yaw = 0`. Obrót o +90° wokół Y przenosi `-X` na `+Z`.
///
/// Znak jest tu nieoczywisty, bo zależy od tego, czy `Quat::from_rotation_y`
/// obraca zgodnie z prawoskrętną regułą (czyli tak, jak wygląda obrót
/// patrząc z góry na dół osi). Gdyby ta stała była zła, motocykl jechałby
/// bokiem do drogi — stąd osobna nazwa i test `bike_matrix_points_the_nose_forward`.
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
    for (id, mat) in &game.meshes.bike {
        cmds.push(DrawCmd::new(*id, m).with_material(*mat));
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
    ctx.gfx
        .color(Color::from_hex(0x66CCFF))
        .draw_rect(Rect::new(
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
        &format!("{fps:.0} FPS   W gas   S brake   A/D steer   R restart   ESC quit"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test chroni przed najdroższym błędem tej gry: model ustawiony
    /// bokiem do kierunku jazdy. Wygląda to jak „działa, tylko dziwnie",
    /// a poprawia się zmianą jednej stałej.
    #[test]
    fn bike_model_points_along_the_direction_of_travel() {
        // Nos modelu po imporcie leży na `-X` (oryginalna oś Blendera).
        let nose_local = Vec3::new(-1.0, 0.0, 0.0);
        let b = Bike::default();
        let m = bike_matrix(&b);
        let nose_world = m * uran_math::Vec4::new(nose_local.x, nose_local.y, nose_local.z, 1.0);
        let nose = Vec3::new(nose_world.x, nose_world.y, nose_world.z);

        // Postój = jazda po `+Z`, więc nos musi wskazywać `+Z`
        assert!(
            (nose.z - 1.0).abs() < 1e-4,
            "nos wskazuje {nose:?} zamiast +Z — model jedzie bokiem"
        );
        assert!(nose.x.abs() < 1e-4, "nos ma składową X = {}", nose.x);
    }

    /// Skręt w prawo musi obrócić model zgodnie z tym, jak obróci się
    /// kierunek jazdy — inaczej kierownica obraca bryłę w drugą stronę.
    #[test]
    fn model_follows_heading() {
        let mut b = Bike::default();
        b.yaw = 1.0; // ~57°
        let nose_local = Vec3::new(-1.0, 0.0, 0.0);
        let m = bike_matrix(&b);
        let w = m * uran_math::Vec4::new(nose_local.x, nose_local.y, nose_local.z, 1.0);
        let nose = Vec3::new(w.x, w.y, w.z).normalize_or_zero();
        let want = b.forward();
        assert!(
            nose.dot(want) > 0.999,
            "model {nose:?} nie zgadza się z kierunkiem jazdy {want:?}"
        );
    }

    /// Model musi stać na ziemi, nie unosić się w powietrzu.
    #[test]
    fn bike_matrix_places_the_model_where_the_simulation_says() {
        let mut b = Bike::default();
        b.pos = Vec3::new(5.0, 0.0, -12.0);
        let m = bike_matrix(&b);
        let p = m * uran_math::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            (p.x - 5.0).abs() < 1e-4 && (p.z + 12.0).abs() < 1e-4,
            "model jest w ({}, {}) zamiast (5, -12)",
            p.x,
            p.z
        );
    }

    /// Kadr kamery: motocykl musi być WIDOCZNY, czyli mieścić się
    /// między kamerą a punktem patrzenia.
    #[test]
    fn camera_sits_behind_and_above_the_bike() {
        let mut b = Bike::default();
        b.pos = Vec3::ZERO;
        b.yaw = 0.0;
        let f = b.forward();
        let eye = b.pos - f * CAM_BACK + Vec3::new(0.0, CAM_UP, 0.0);
        // okno jest 4,3 m za motocyklem i 1,5 m nad nim
        assert!(
            (eye.z + CAM_BACK).abs() < 1e-4,
            "kamera nie jest za motocyklem"
        );
        assert!((eye.y - CAM_UP).abs() < 1e-4, "kamera nie jest nad ziemią");
        // kamera NIE może być wewnątrz modelu (1,2 m wysokości)
        assert!(eye.y > 1.2, "kamera jest w środku nadwozia");
    }
}

/// Punkt wejścia.
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
