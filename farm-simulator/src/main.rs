//! Farm Simulator — pole, sadzenie, czekanie, zbiór.
//!
//! # Architektura
//!
//! Gra stoi na trzech modułach:
//!
//! * [`farm`] — mechanika: siatka działek i pętla sadz → rośnie → zbierz,
//! * [`geometry`] — bryły budowane w kodzie (trawa, roślina, postać),
//! * [`assets`] — budynki wczytywane z plików `.obj` Quaternius.
//!
//! `main.rs` trzyma tylko warstwę gry: wejście, kamerę, listę
//! rysowania i HUD. Cała logika rozgrywki jest testowana w `farm.rs`,
//! a geometria w `geometry.rs` — oba moduły działają bez GPU.
//!
//! # Sterowanie
//!
//! `W/A/S/D` — chodzenie, `E` — sadz lub zbierz, `R` — nowa farma,
//! `Esc` — wyjście.

mod assets;
mod farm;
mod geometry;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_math::{Mat4, Vec3};
use uran_render3d::{DrawCmd, MaterialId, MeshId, Renderer3d, model_matrix};

use crate::farm::{Farm, TILE, tile_at, tile_center};
use crate::geometry::palette;

/// Rozmiar świata w metrach. Kwadratowy, bo mgła i kadr cienia
/// zakładają się na jeden wymiar — przy prostokącie trzeba by liczyć
/// dwa różne promienie.
const WORLD: f32 = 90.0;

/// Jak daleko kamera jest za graczem.
const CAM_BACK: f32 = 8.5;
/// Jak wysoko nad graczem.
const CAM_UP: f32 = 5.4;
/// Jak szybko gracz chodzi (m/s).
const WALK_SPEED: f32 = 5.0;
/// Największa odległość gracza od środka mapy.
const PLAY_LIMIT: f32 = WORLD * 0.5 - 2.0;

/// Jedna część wczytanego budynku: lista (siatka, materiał).
struct Prop {
    parts: Vec<(MeshId, MaterialId)>,
    pos: Vec3,
    yaw: f32,
}

/// Siatki zarejestrowane raz przy starcie.
struct Meshes {
    ground: MeshId,
    soil: MeshId,
    player: MeshId,
    /// Roślina w trzech fazach: sadzonka, w połowie, dojrzała.
    plant: [MeshId; 3],
    /// Budynki i płot wokół farmy.
    buildings: Vec<Prop>,
}

/// Stan gry dzielony między systemami.
struct Game {
    scene: Arc<Mutex<Renderer3d>>,
    meshes: Meshes,
    farm: Farm,
    /// Pozycja gracza na płaszczyźnie.
    player_pos: Vec3,
    /// Czas świata (mgła i animacje).
    time: f32,
    /// Czy wszystkie działki są dojrzałe.
    won: bool,
    font: Option<Font>,
}

impl Game {
    /// Buduje świat i renderer 3D na gotowym urządzeniu.
    fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
        let mut scene = Renderer3d::new(gpu, format, samples);
        // Niebo: jasny błękit. Kolor podajemy w sRGB, bo shader sceny
        // miesza z nim mgłę i musi zgadzać się z gradientem nieba.
        scene.set_clear_color([0.58, 0.74, 0.90, 1.0]);
        {
            let l = scene.lighting_mut();
            // Słońce ~48° nad horyzontem i wyraźnie z boku. Niski kąt
            // daje długie cienie, które pokazują, gdzie jest pole
            // uprawne, a gdzie zwykła trawa.
            l.light_dir = Vec3::new(0.42, 0.66, 0.62).normalize();
            // Ciepłe, lekko złotawe — popołudniowe słońce nad polem.
            l.light_color = [1.0, 0.95, 0.84];
            l.ambient = [0.36, 0.45, 0.60];
        }
        {
            let cam = scene.camera_mut();
            // 60° wystarczy: szersze pole widzenia zniekształca
            // budynki przy krawędziach ekranu.
            cam.fov_y = std::f32::consts::FRAC_PI_3;
            cam.near = 0.1;
            cam.far = 400.0;
        }

        // --- teren i pole ---
        let ground = geometry::ground(WORLD, 6.0);
        let ground = scene.add_mesh(gpu, &ground, "trawa");

        // Gleba: jedna płyta pod całym polem. Działki różnią się tym,
        // co na nich rośnie, a nie kolorem gruntu — inaczej zamiast
        // uprawy widać by szachownicę.
        let fw = farm::FIELD_W as f32 * TILE;
        let fd = farm::FIELD_D as f32 * TILE;
        // Podnosimy glebę o 2 cm nad trawę: inaczej z poziomu kamery
        // obie powierzchnie zlewają się w jedną płaszczyznę.
        let soil = scene.add_mesh(
            gpu,
            &geometry::quad_y(0.0, 0.0, fw * 0.5, 0.02, palette::SOIL),
            "gleba",
        );

        // --- roślina w trzech fazach wzrostu ---
        let plant = [
            scene.add_mesh(gpu, &geometry::plant(0.12), "roslina 0"),
            scene.add_mesh(gpu, &geometry::plant(0.55), "roslina 1"),
            scene.add_mesh(gpu, &geometry::plant(1.0), "roslina 2"),
        ];

        let player = scene.add_mesh(gpu, &geometry::player(), "gracz");

        // --- płot i budynki wokół farmy ---
        let buildings = build_props(&mut scene, gpu);

        println!(
            "🌾  farma: {}×{} działek, {}×{:.0} m, {}/{} budynków",
            farm::FIELD_W,
            farm::FIELD_D,
            fw,
            fd,
            assets::available_buildings(),
            assets::BUILDINGS.len()
        );

        Self {
            scene: Arc::new(Mutex::new(scene)),
            meshes: Meshes {
                ground,
                soil,
                player,
                plant,
                buildings,
            },
            farm: Farm::new(),
            // Startujemy przy krawędzi pola, żeby od razu widzieć
            // uprawę i budynki w jednym kadrze.
            player_pos: Vec3::new(0.0, 0.0, fd * 0.5 + 5.0),
            time: 0.0,
            won: false,
            font: None,
        }
    }
}

/// Rejestruje wczytany model: część na część, materiał na materiał.
///
/// Materiały deduplikujemy po nazwie: ten sam kolor z `.mtl` występuje
/// w kilku częściach modelu, a każda kopia to osobny bind group
/// i osobne tekstury na karcie.
fn register_model(
    scene: &mut Renderer3d,
    gpu: &uran_render::GpuContext,
    model: uran_render3d::ImportedModel,
) -> Vec<(MeshId, MaterialId)> {
    let mut seen: Vec<(String, MaterialId)> = Vec::new();
    let mut out = Vec::with_capacity(model.parts.len());
    for part in &model.parts {
        let mat = match seen.iter().find(|(n, _)| *n == part.material.name) {
            Some((_, id)) => *id,
            None => {
                let id = scene.add_material(gpu, &part.material);
                seen.push((part.material.name.clone(), id));
                id
            }
        };
        let id = scene.add_mesh(gpu, &part.mesh, &part.name);
        out.push((id, mat));
    }
    out
}

/// Rozstawienie płotu i budynków wokół pola.
///
/// Pozycje dobrane tak, żeby z dowolnego miejsca pola widać było
/// co najmniej jeden budynek — puste niebo wygląda jak brakujący świat,
/// a nie łąka otoczona lasem.
fn build_props(scene: &mut Renderer3d, gpu: &uran_render::GpuContext) -> Vec<Prop> {
    let mut out: Vec<Prop> = Vec::new();
    let fw = farm::FIELD_W as f32 * TILE;
    let fd = farm::FIELD_D as f32 * TILE;

    // --- płot wokół pola ---
    //
    // Segment `Fence.obj` ma 5,89 m długości przy naturalnej skali
    // (1,10 m wysokości), więc rozstawiamy go co ~5,5 m z lekkim
    // zakładką. Wcześniejszy krok 2,2 m kładł segmenty JEDEN NA
    // DRUGIM — na zrzucie widać by „kupę belek" zamiast płotu.
    if let Some(model) = assets::load_building("Fence", assets::FENCE_HEIGHT) {
        let fence = register_model(scene, gpu, model);
        // 5,5 m: tyle ma segment pomniejszony o zakładkę, żeby słupki
        // stykały się wizualnie bez szczeliny.
        let step = 5.5f32;
        let half_w = fw * 0.5 + 0.5;
        let half_d = fd * 0.5 + 0.5;
        // `ceil` w górę + `min 2` — pole musi mieć choćby dwa segmenty
        // na bok, inaczej płot wygląda jak jedna belka.
        let nx = (fw / step).ceil().max(2.0) as i32;
        let nz = (fd / step).ceil().max(2.0) as i32;
        // `false` = wiersz wzdłuż X, `true` = wzdłuż Z.
        for (along_z, z_edge, x_edge, yaw, n) in [
            (false, -half_d, 0.0f32, 0.0f32, nx),
            (false, half_d, 0.0, std::f32::consts::PI, nx),
            (true, 0.0, -half_w, std::f32::consts::FRAC_PI_2, nz),
            (true, 0.0, half_w, -std::f32::consts::FRAC_PI_2, nz),
        ] {
            for k in 0..n {
                // `-0.5 .. +0.5` to środek pierwszego i ostatniego
                // segmentu, dzięki czemu płot jest symetryczny.
                let t = (k as f32 + 0.5) / n as f32 - 0.5;
                let pos = if along_z {
                    Vec3::new(x_edge, 0.0, t * n as f32 * step)
                } else {
                    Vec3::new(t * n as f32 * step, 0.0, z_edge)
                };
                out.push(Prop {
                    parts: fence.clone(),
                    pos,
                    yaw,
                });
            }
        }
    }

    // --- budynki ---
    //
    // Każdy ma własny obrót: stodoło prostopadle do młyna czyta się
    // jako wieś, a wszystko równolegle — jako parking.
    // Pole ma 12×12 m, więc budynki stawiamy od 9 m od jego środka
    // w bok — inaczej stodoło (7,7 m szerokości) zasłaniałoby uprawę.
    let places: &[(&str, Vec3, f32)] = &[
        ("BigBarn", Vec3::new(-14.0, 0.0, -11.0), 0.35),
        ("Silo", Vec3::new(-6.0, 0.0, -14.5), 0.0),
        ("Windmill", Vec3::new(14.0, 0.0, -11.0), -0.45),
        ("Well", Vec3::new(4.5, 0.0, 8.0), 0.8),
    ];
    for (file, pos, yaw) in places {
        let Some(spec) = assets::BUILDINGS.iter().find(|b| b.file == *file) else {
            continue;
        };
        let Some(model) = assets::load_building(file, spec.height) else {
            continue;
        };
        out.push(Prop {
            parts: register_model(scene, gpu, model),
            pos: *pos,
            yaw: *yaw,
        });
    }

    out
}

/// Macierz postaci (bez obrotu — chodzi w czterech kierunkach).
fn player_matrix(p: Vec3) -> Mat4 {
    model_matrix(p, 0.0, 0.0, 0.0, Vec3::ONE)
}

/// Kamera trzecioosobowa: za graczem i nad nim.
///
/// Celujemy TUTAJ, a nie w gracza — przy celowaniu w gracza kamera
/// zjeżdża do jego głowy i gracz znika z kadru.
fn update_camera(game: &mut Game) {
    let eye = game.player_pos + Vec3::new(0.0, CAM_UP, CAM_BACK);
    let look = game.player_pos + Vec3::new(0.0, 1.0, 0.0);
    if let Ok(mut scene) = game.scene.lock() {
        let cam = scene.camera_mut();
        cam.position = eye;
        cam.target = look;
    }
}

/// Drobne przesunięcie rośliny w obrębie działki, żeby nie stały
/// w idealnym szyku.
///
/// Używamy liczb „współczynników złotego ciągu" zamiast `rand`, bo:
/// * ten sam `(ix, iz)` daje zawsze to samo przesunięcie — rośliny
///   nie skaczą między klatkami,
/// * nie potrzebujemy stanu generatora w strukturze gry.
fn jitter(ix: usize, iz: usize) -> (f32, f32) {
    let a = 0.55 * (0.618_034 * (ix as f32 * 7.0 + iz as f32 * 3.0).sin());
    let b = 0.55 * (0.618_034 * (ix as f32 * 3.0 + iz as f32 * 7.0).cos());
    (a, b)
}

/// Składa listę obiektów 3D na bieżącą klatkę.
///
/// Kolejność ma znaczenie tylko dla czytelności — głębią zajmuje się
/// depth buffer, a potok nie ma mieszania alfa.
fn build_draw_list(game: &mut Game) {
    let mut cmds: Vec<DrawCmd> = Vec::with_capacity(160);
    let one = model_matrix(Vec3::ZERO, 0.0, 0.0, 0.0, Vec3::ONE);
    let m = &game.meshes;

    // Trawa i gleba leżą nieruchomo.
    cmds.push(DrawCmd::new(m.ground, one));
    cmds.push(DrawCmd::new(m.soil, one));

    // Płot i budynki.
    for prop in &m.buildings {
        let mat = model_matrix(prop.pos, prop.yaw, 0.0, 0.0, Vec3::ONE);
        for (id, mat_id) in &prop.parts {
            cmds.push(DrawCmd::new(*id, mat).with_material(*mat_id));
        }
    }

    // Rośliny: dla każdej zasadzonej działki dobieramy siatkę wg
    // wzrostu. Trzy siatki zamiast skaliowania jednej, bo skalowanie
    // zniekształcałoby proporcje łodygi.
    for iz in 0..farm::FIELD_D {
        for ix in 0..farm::FIELD_W {
            let tile = game.farm.get(ix, iz);
            if tile == farm::Tile::Empty {
                continue;
            }
            let g = tile.growth();
            let stage = if tile.is_ripe() {
                2
            } else if g < 0.5 {
                0
            } else {
                1
            };
            let (jx, jz) = jitter(ix, iz);
            let pos = tile_center(ix, iz) + Vec3::new(jx, 0.0, jz);
            cmds.push(DrawCmd::new(m.plant[stage], player_matrix(pos)));
        }
    }

    // Gracz.
    cmds.push(DrawCmd::new(m.player, player_matrix(game.player_pos)));

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
        game.farm = Farm::new();
        game.won = false;
        game.player_pos = Vec3::new(0.0, 0.0, farm::FIELD_D as f32 * TILE * 0.5 + 5.0);
    }

    // --- ruch gracza ---
    //
    // `axis(a, b)` daje -1/+1. Normalizujemy wektor ruchu, bo inaczej
    // chodzenie po skosie byłoby szybsze niż po prostej.
    let dir = Vec3::new(
        ctx.input.axis(Key::KeyA, Key::KeyD),
        0.0,
        ctx.input.axis(Key::KeyS, Key::KeyW),
    );
    if dir.length_squared() > 1e-6 {
        let p = game.player_pos + dir.normalize() * (WALK_SPEED * dt);
        // Ograniczenie do kwadratu świata: bez niego gracz wychodzi
        // poza teren i kamera pokazuje pustą mgłę.
        game.player_pos = Vec3::new(
            p.x.clamp(-PLAY_LIMIT, PLAY_LIMIT),
            0.0,
            p.z.clamp(-PLAY_LIMIT, PLAY_LIMIT),
        );
    }

    // --- interakcja z polem ---
    if ctx.input.just_pressed(Key::KeyE) {
        if let Some((ix, iz)) = tile_at(game.player_pos) {
            game.farm.interact(ix, iz);
        } else {
            game.farm.message = "Stan na polu".into();
            game.farm.message_time = 1.2;
        }
    }

    // Rośliny dojrzewają niezależnie od tego, czy gracz patrzy.
    game.farm.step(dt);
    if !game.won && game.farm.is_won() {
        game.won = true;
    }

    update_camera(game);
    build_draw_list(game);
}

/// HUD: stan pola, liczniki i podpowiedź sterowania.
///
/// Rysujemy w przestrzeni EKRANU (renderer 2D), tak jak w `uran-tanks`
/// i `junak-rider`, i z tej samego powodu: 2D nie przechodzi przez
/// post-processing 3D, więc tekst nie zostaje zniekształcony przez ACES
/// ani rozmyty przez bloom.
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let size = ctx.window.logical_size();
    let (empty, growing, ready) = game.farm.counts();

    // --- pasek stanu pola (u góry ekranu) ---
    ctx.gfx
        .screen_space()
        .layer(100)
        .color(Color::from_hex(0x1B2430))
        .draw_rect(Rect::new(
            Vec2::new(size.x * 0.5 - 190.0, 14.0),
            Vec2::new(size.x * 0.5 + 190.0, 44.0),
        ));
    // Proporcja paska to `ready / wszystkie`: to stan, który gracz
    // realnie zmienia — sadzenie samo w sobie nie kończy pracy.
    let total: f32 = (empty + growing + ready).max(1) as f32;
    let t = ready as f32 / total;
    ctx.gfx
        .color(Color::from_hex(0xE8C547))
        .draw_rect(Rect::new(
            Vec2::new(size.x * 0.5 - 188.0, 38.0),
            Vec2::new(size.x * 0.5 - 188.0 + 376.0 * t, 42.0),
        ));

    // Wskaźnik działki pod graczem: pokazuje, co stanie się po `E`.
    if let Some((ix, iz)) = tile_at(game.player_pos) {
        let (hint, c) = match game.farm.get(ix, iz) {
            farm::Tile::Empty => ("[E] Zasadz", Color::from_hex(0x9BE07A)),
            farm::Tile::Ready => ("[E] Zbierz", Color::from_hex(0xF2C14E)),
            farm::Tile::Growing { .. } => ("Rosnie...", Color::from_hex(0xB9C4D4)),
        };
        if let Some(font) = game.font {
            ctx.gfx.color(c).draw_text(
                font,
                hint,
                Vec2::new(size.x * 0.5, size.y * 0.5 - 90.0),
                26.0,
                TextAlign::Center,
            );
        }
    }

    // --- komunikat akcji ---
    if game.farm.message_time > 0.0 {
        if let Some(font) = game.font {
            ctx.gfx.color(Color::from_hex(0xF4F7FB)).draw_text(
                font,
                &game.farm.message,
                Vec2::new(size.x * 0.5, size.y * 0.5 - 130.0),
                22.0,
                TextAlign::Center,
            );
        }
    }

    let Some(font) = game.font else { return };

    // --- liczniki (lewy górny róg) ---
    //
    // UWAGA: tylko znaki ASCII. Polskie diakrytyczne wywołują błąd
    // atlasu glifów w `uran-render` przy przepełnieniu (patrz
    // `draw_hud` w uran-tanks), więc HUD zostaje czytelny bez nich.
    ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
        font,
        &format!("Zebrano: {}", game.farm.harvested),
        Vec2::new(24.0, 24.0),
        24.0,
        TextAlign::Left,
    );
    ctx.gfx.color(Color::from_hex(0xB9C4D4)).draw_text(
        font,
        &format!("Puste {empty}  Rosnie {growing}  Gotowe {ready}"),
        Vec2::new(24.0, 54.0),
        18.0,
        TextAlign::Left,
    );

    // --- sterowanie (lewy dolny róg) ---
    ctx.gfx.color(Color::from_hex(0x8FA6C4)).draw_text(
        font,
        "WASD ruch   E sadz/zbierz   R nowa farma",
        Vec2::new(24.0, size.y - 32.0),
        16.0,
        TextAlign::Left,
    );

    // --- wygrana ---
    if game.won {
        ctx.gfx
            .screen_space()
            .layer(101)
            .color(Color::from_hex(0x141A24))
            .draw_rect(Rect::new(
                Vec2::new(0.0, size.y * 0.5 - 60.0),
                Vec2::new(size.x, size.y * 0.5 + 60.0),
            ));
        ctx.gfx.color(Color::from_hex(0xF2C14E)).draw_text(
            font,
            "POLE DOJRZALE - ZBIERASZ!",
            Vec2::new(size.x * 0.5, size.y * 0.5 - 12.0),
            40.0,
            TextAlign::Center,
        );
        ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
            font,
            "R - nowa farma",
            Vec2::new(size.x * 0.5, size.y * 0.5 + 26.0),
            20.0,
            TextAlign::Center,
        );
    }
}

/// Proxy renderera 3D podpinany do renderera 2D (patrz `uran-render`).
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

/// Punkt wejścia.
fn main() {
    let game = Arc::new(Mutex::new(None::<Game>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if guard.is_some() {
                return;
            }
            // JEDEN `Device` obsługuje oba renderery, a `samples` MUSI
            // się zgadzać — inaczej hdr-owa tekstura post-processingu
            // nie przejdzie walidacji wgpu.
            let built = {
                let r = ctx.renderer.as_ref().expect("renderer gotowy");
                Game::new(r.gpu(), r.format(), r.samples())
            };
            *guard = Some(built);
            let g = guard.as_mut().expect("właśnie utworzone");
            g.font = ctx.assets.load_font("assets/fonts/UranSans.ttf").ok();
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
        // Zrzut po 40 klatkach: tyle trzeba, żeby world mgły i cieni
        // się ustabilizowały. Używany w testach regresji wizualnej
        // (`FARM_SHOT=1 cargo run -p farm-simulator`).
        .screenshot(
            std::env::var("FARM_SHOT").ok().map(std::path::PathBuf::from),
            40,
        )
        .window(
            windowed(1280, 720)
                .title("Farm Simulator")
                .background(0x94BDE6)
                .vsync(true)
                .samples(1),
        )
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}
