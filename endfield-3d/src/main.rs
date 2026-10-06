//! Endfield 3D — demo akcji w stylu *Arknights: Endfield*.
//!
//! # Cel
//!
//! Pokazać, że renderer potrafi jednocześnie rysować rysunkową
//! postać i fizyczne otoczenie. Wszystko, co tu jest, służy temu
//! jednemu celowi: otoczenie jest zbudowane tak, żeby każdy efekt
//! (`world.rs`) miał gdzie działać, a postać (`character.rs`) tak,
//! żeby było na czym pokazać rysunkową rampę, SSS i anizotropię.
//!
//! # Warstwy
//!
//! ```text
//!  [3D] scena → HDR + G-Buffer → postfx → ekran
//!  [2D] HUD (renderer 2D, po 3D, w osobnym passie)
//! ```
//!
//! Oba renderery dzielą `Device`, `Queue` i `Surface`, ale zero
//! potoków, buforów i shaderów.
//!
//! Sterowanie: WASD — ruch, mysz — kamera, LPM — ogień,
//! Shift — bieg, Spacja — skok, R — restart.

mod character;
mod game;
mod geometry;
mod world;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_math::{Vec2, Vec3};
use uran_render3d::{model_matrix, DrawCmd, MaterialId, MeshId, Renderer3d};

use character::CharacterMaterials;
use game::{Game as Sim, Input, MAX_AMMO};
use world::palette;

/// Odległość kamery za postacią.
///
/// 4.2 m: bliżej niż w FPS-ach, bo postać ma 1.75 m i przy dalszej
/// kamerze widać tylko plamę. Dalej niż 3 m, bo ramiona gracza
/// zasłaniałyby celownik.
const CAM_BACK: f32 = 4.2;

/// Ograniczenie spojrzenia w pionie, w radianach.
///
/// Bez tego gracz mógłby obrócić kamerę pod siebie i stracić
/// horyzont — a bez horyzontu nie ma nieba, czyli całej atmosfery.
const PITCH_LIMIT: f32 = 1.15;

/// Identyfikatory siatek — rejestrowane raz przy starcie.
struct Meshes {
    floor: MeshId,
    dais: MeshId,
    core_inner: MeshId,
    core_shell: MeshId,
    pillar: MeshId,
    beam: MeshId,
    crate_mesh: MeshId,
    strip: MeshId,
    conduit: MeshId,
    drone_core: MeshId,
    drone_shell: MeshId,
    bolt: MeshId,
    /// Iskra trafienia — kulista, bo rozbłysk nie ma formy.
    spark: MeshId,
    // --- postać ---
    head: MeshId,
    hair: MeshId,
    hair_bright: MeshId,
    torso: MeshId,
    hips: MeshId,
    arm: MeshId,
    leg: MeshId,
    boot: MeshId,
    buckle: MeshId,
    visor: MeshId,
}

/// Materiały całej sceny.
///
/// Trzymane w jednym miejscu, bo skąd pochodzi baza koloru zależy
/// od tego, którą drogą (`Surface::base` czy kolor wierzchołka)
/// dana bryła została zbudowana — patrz `build_materials`.
struct Materials {
    floor: MaterialId,
    structure: MaterialId,
    crate_mat: MaterialId,
    core: MaterialId,
    neon_cool: MaterialId,
    neon_warm: MaterialId,
    alert: MaterialId,
    spark: MaterialId,
    char: CharacterMaterials,
}

/// Stan aplikacji.
struct App3d {
    sim: Sim,
    scene: Arc<Mutex<Renderer3d>>,
    meshes: Meshes,
    mats: Materials,
    /// Kąt kamery wokół Y.
    cam_yaw: f32,
    /// Kąt kamery w pionie.
    cam_pitch: f32,
    font: Option<Font>,
}

/// Materiał emisyjny — neon, listwa, iskra.
///
/// Osobna funkcja, bo takich materiałów w scenie jest pięć, a każdy
/// różni się tylko dwoma liczbami. Bazę ustawiamy na niemal czerń:
/// przy jasnej bazie pod emisją powstaje biała plama zamiast
/// kolorowego światła, a rampa rysunkowa na takiej bazie daje
/// poszarpane krawędzie zamiast gładkiej poświaty.
fn emissive_mat(
    bank: &mut uran_render3d::MaterialBank,
    gpu: &uran_render::GpuContext,
    color: [f32; 3],
    gain: f32,
) -> MaterialId {
    bank.add_surface(
        &gpu.device,
        &gpu.queue,
        &uran_render3d::Surface {
            base: [0.02, 0.02, 0.02],
            emissive: color,
            diffuse_gain: gain,
            // Emisja sama w sobie nie przechodzi przez rampę, ale
            // `stylize = 0` pilnuje, żeby baza pod spodem była płaska.
            stylize: 0.0,
            roughness: 0.5,
            ..Default::default()
        },
    )
}

impl App3d {
    /// Buduje renderer 3D, scenę i materiały na gotowym urządzeniu.
    ///
    /// `samples` MUSI być równe temu, co wybrał renderer 2D. Przy
    /// różnych liczbach próbek wgpu odrzuci bind group błędem o
    /// niedopasowanym formacie, który nie mówi wprost, że chodzi
    /// o MSAA.
    fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
        let mut scene = Renderer3d::new(gpu, format, samples);

        // --- kolor nieba ---
        //
        // Wartość trafia w cały kadryf, a gradient dochodzi dopiero w
        // shaderze nieba. Mgła musi mieć zbliżony odcień, inaczej na
        // horyzoncie widać pas, w którym mgła jest innego koloru niż
        // niebo.
        scene.set_clear_color([0.34, 0.47, 0.65, 1.0]);

        // --- oświetlenie: późne popołudnie, słońce nisko z boku ---
        {
            let l = scene.lighting_mut();
            // Kąt ok. 40° daje długie, czytelne cienie. Przy słońcu
            // prosto w głowę bryły są płaskie i nie widać formy.
            l.light_dir = Vec3::new(0.55, 0.58, 0.60).normalize_or_zero();
            // Ciepłe, lekko złotawe. Czysta biel wygląda w
            // industrialnej scenie jak oświetlenie fluorescencyjne,
            // a nie jak słońce.
            l.light_color = [1.0, 0.90, 0.76];
            // 6.5: słońce musi WYRAŹNIE dominować nad otoczeniem.
            //
            // Powód jest w dyfuzji Lambertowskiej: `albedo · N·L / π`
            // dzieli energię przez π, więc do osiągnięcia jasnego dnia
            // natężenie musi być rzędu 6, nie 3. Stąd 6.5.
            //
            // Uwaga na albedo: `palette::FLOOR` = 0.26 podane jest
            // w sRGB, więc liniowo jest 0.055. Podłoga w słońcu
            // wychodzi `0.055 · 0.58 / π · 6.5` = 0.063 → 87/255,
            // czyli jasny, ale nie prześwietlony beton.
            l.intensity = 6.5;
            // Ambient NIESIE NIEBO — chłodny, wyraźnie błękitny. To on
            // oświetla cienie; gdyby był szary, cień wyglądałby jak
            // brak światła, a nie brak słońca.
            //
            // 0.15/0.19/0.26 → 0.45/0.57/0.78 (×3).
            //
            // ## Dlaczego poprzednia wartość była zbyt mała
            //
            // Wcześniejszy komentarz uzasadniał 0.15/0.19/0.26
            // „mniejszą proporcją do słońca", ale ta proporcja nigdy
            // nie została przeliczona poprawnie. `palette::FLOOR`
            // = 0.26 jest podane w **sRGB**, a shader przelicza je na
            // liniowe, więc albedo podłogi to `srgb_to_linear(0.26)`
            // = **0.055**, nie 0.26.
            //
            // Wkład samego otoczenia w podłogę to zatem:
            // `srgb_to_linear(0.15) · 0.055` = `0.0194 · 0.055`
            // = **0.00107** liniowo. Przy `AGX_MIN_EV = -12.47`
            // wypada to na **1/255** — cień był dosłownie czarny.
            //
            // Podbicie o ×3 daje `0.0574 · 0.055` = 0.00316, czyli
            // **21/255** przy 23% jasności plamy słonecznej (93/255).
            // To standardowy udział cienia w rysunku rysunkowym:
            // czytelna forma, ale wciąż wyraźnie ciemniejsza od słońca.
            // Przy ×4 cień wchodzi na 39/255 i kontrast między
            // plamą a cieniem zanika.
            l.ambient = [0.45, 0.57, 0.78];
        }

        // --- stylizacja: miara hybrydy ---
        {
            let s = scene.stylization_mut();
            // 0.62 działa jak mnożnik rysunkowości CAŁEJ sceny.
            // Otoczenie ma w materiałach `stylize ≈ 0.1`, więc daje
            // 0.62 × 0.1 = 0.06, czyli prawie czyste PBR. Skóra ma
            // 0.85, więc daje 0.53 — wyraźna rampa. To jest dokładnie
            // ten układ, o który chodzi w stylu Endfield.
            s.amount = 0.62;
            // 0.08 miękkości progu: rampa ma miękkie przejście. Niżej
            // krawędź zaczyna migotać przy ruchu kamery.
            s.softness = 0.08;
            // SSS sceny mnoży SSS materiału. 0.7 zostawia skórze
            // widoczne ciepłe krawędzie, ale nie zamienia jej w
            // podświetlonego klusza.
            s.sss = 0.7;
        }

        // --- atmosfera ---
        {
            let a = scene.atmosphere_mut();
            // Fizyczne niebo (Rayleigh + Mie). Bez niego nie ma horyzontu,
            // a bez horyzontu mgła nie ma na czym kończyć się.
            a.sky = true;
            // 0.012 -> 0.0016 (wartość domyślna silnika).
            //
            // 0.0035 dawało `1 - e^(-0.35)` = 29,5% mgły na 100 m, a
            // test `mgla_domyslna_jest_delikatna_i_nie_rozjasnia_dystansu`
            // wymaga poniżej 20%. Przy 0.0016 to 14,8%, czyli mgła
            // ledwie zaznacza dystans i nie rozlewa się po podłodze.
            //
            // Ten sam wzór liczy `Atmosphere::default()`, więc demo
            // dostaje wartość z silnika zamiast własnej — zmiana
            // domyślnej działa tu automatycznie.
            a.fog_density = 0.0016;
            // 0.46/0.55/0.68 -> 0.32/0.38/0.46.
            //
            // Mgla musi byc CIEMNIEJSZA od nieba inaczej dystans
            // sie nie zanika, tylko rozjasnia: jasna warstwa
            // mgly na ciemnym tle wyglada jak bledy, a caly kadr
            // ciagnie ku blademu błękitowi. Luma mgly 0.37
            // kontra niebo 0.46 - horyzont oddala obiekty, a nie
            // rozmywa je.
            a.fog_color = [0.32, 0.38, 0.46];
        }

        let mats = build_materials(gpu, &mut scene);
        let meshes = build_mesh_ids(gpu, &mut scene);

        Self {
            sim: Sim::new(),
            scene: Arc::new(Mutex::new(scene)),
            meshes,
            mats,
            // Kamera startuje za postacią, lekko z góry: pierwszy kadr
            // ma pokazywać sylwetkę NA TLE sceny, a nie z bliska.
            cam_yaw: 0.0,
            cam_pitch: 0.16,
            font: None,
        }
    }
}

/// Buduje komplet materiałów sceny.
///
/// Osobna funkcja, bo `App3d::new` robi już dużo: ustawia światło,
/// niebo i mgłę. Wydzielenie materiałów sprawia też, że czytelnik
/// widzi w jednym miejscu CAŁY zestaw i może ocenić hybrydę
/// rysunkowo/fizyczną bez przewijania.
///
/// # Dlaczego kolor siedzi w `Surface::base`, a nie w wierzchołkach
///
/// `MaterialBank::add_surface` ustawia `base.w = 0`, więc shader bierze
/// bazę z koloru WIERZCHOŁKA. Dlatego cała geometria w tym demo ma
/// BIAŁE wierzchołki, a kolor jest tutaj. Gdyby było odwrotnie, kolor
/// z `palette::` cicho by przepadł — a taki błąd jest bardzo trudny do
/// zlokalizowania, bo geometria wygląda poprawnie.
///
/// Druga droga (`MaterialBank::add`, pliki `.mtl`) działa odwrotnie —
/// opisana w `material.rs`. Nie mieszamy obu w jednym demo, bo
/// mieszanie daje błędy koloru bez żadnego komunikatu błędu.
fn build_materials(gpu: &uran_render::GpuContext, scene: &mut Renderer3d) -> Materials {
    let bank = &mut scene.materials;
    let mut add = |s: uran_render3d::Surface| bank.add_surface(&gpu.device, &gpu.queue, &s);

    // Podłoga: niska chropowatość, bo to ona odbija postać i listwy
    // (SSR). Przy 0.6+ odbicie znika i podłoga wygląda jak plastik.
    //
    // Szew NIE ma osobnego materiału, mimo że chce być matowy:
    // kolor wierzchołka steruje całą siatką naraz. Dostanie
    // osobnego materiału wymagałoby rozbicia podłogi na dwie siatki
    // i rysowania ich osobno — a zysk byłby ledwie widoczny, bo szew
    // i tak jest wąski. Świadomie świadome odstępstwo od reguły
    // „osobny wygląd = osobny materiał", zapisane tutaj, żeby ktoś
    // nie szukał go później w kodzie.
    let floor = add(uran_render3d::Surface {
        base: palette::FLOOR,
        roughness: 0.22,
        stylize: 0.10,
        // JEDYNY materiał w tej scenie sterowany kolorem wierzchołków:
        // szew i radialny gradient muszą zmieniać się na jednej bryle.
        // `base` zostaje jako kolor awaryjny (np. gdyby ktoś narysował
        // tę siatkę materiałem bez flagi).
        vertex_color: true,
        ..Default::default()
    });
    // Konstrukcja: metal, ale stonowany. Pełna metaliczność dałaby
    // ciemne bryły bez odbicia otoczenia, bo w PBR dyfuzja znika
    // przy metalu.
    let structure = add(uran_render3d::Surface {
        base: palette::STRUCTURE,
        roughness: 0.42,
        metallic: 0.85,
        stylize: 0.08,
        ..Default::default()
    });
    // Skrzynia: inny odcień, żeby w kadrze było coś innego niż szarość.
    let crate_mat = add(uran_render3d::Surface {
        base: palette::CRATE,
        roughness: 0.65,
        metallic: 0.2,
        stylize: 0.12,
        ..Default::default()
    });
    // Rdzeń reaktora: HDR przekraczające próg bloom (1.15), więc daje
    // poświatę i staje się głównym źródłem sceny.
    let core = add(uran_render3d::Surface {
        base: [0.05, 0.03, 0.02],
        emissive: palette::CORE,
        diffuse_gain: 3.2,
        stylize: 0.0,
        ..Default::default()
    });

    let neon_cool = emissive_mat(bank, gpu, palette::NEON_COOL, 2.4);
    let neon_warm = emissive_mat(bank, gpu, palette::NEON_WARM, 2.0);
    let alert = emissive_mat(bank, gpu, palette::ALERT, 1.8);
    // Iskra trafienia: najjaśniejsza rzecz w grze po rdzeniu, żeby
    // gracz zauważył strzał nawet w kącie ekranu.
    let spark = emissive_mat(bank, gpu, [1.0, 0.95, 0.80], 5.0);

    let char = character::build(bank, gpu);

    Materials {
        floor,
        structure,
        crate_mat,
        core,
        neon_cool,
        neon_warm,
        alert,
        spark,
        char,
    }
}

/// Rejestruje wszystkie siatki i zwraca ich identyfikatory.
///
/// Osobno od materiałów, bo kolejność jest inna: tu każda siatka
/// dostaje etykietę (widać ją w `wgpu` podczas debugowania), a
/// materiały nie potrzebują etykiet.
fn build_mesh_ids(gpu: &uran_render::GpuContext, scene: &mut Renderer3d) -> Meshes {
    let w = world::build_meshes();
    let c = character::build_meshes();
    // Iskra trafienia: mała kula. Kulista, bo iskra nie ma formy —
    // kwadratowy rozbłysk czytałby się jako „kafelek" przy krawędzi.
    let spark = geometry::sphere(Vec3::ZERO, 0.16, 8, 10, world::VERTEX_WHITE);

    let mut m = |mesh: &uran_render3d::Mesh, label: &str| scene.add_mesh(gpu, mesh, label);

    Meshes {
        floor: m(&w.floor, "podłoga"),
        dais: m(&w.dais, "podest"),
        core_inner: m(&w.core_inner, "rdzeń — kula"),
        core_shell: m(&w.core_shell, "rdzeń — obudowa"),
        pillar: m(&w.pillar, "słup"),
        beam: m(&w.beam, "belka"),
        crate_mesh: m(&w.crate_mesh, "skrzynia"),
        strip: m(&w.strip, "listwa"),
        conduit: m(&w.conduit, "rura"),
        drone_core: m(&w.drone_core, "dron — rdzeń"),
        drone_shell: m(&w.drone_shell, "dron — skorupa"),
        bolt: m(&w.bolt, "pocisk"),
        spark: m(&spark, "iskra"),

        head: m(&c.head, "postać — głowa"),
        hair: m(&c.hair, "postać — czupryna"),
        hair_bright: m(&c.hair_bright, "postać — grzywa"),
        torso: m(&c.torso, "postać — tułów"),
        hips: m(&c.hips, "postać — biodra"),
        arm: m(&c.arm, "postać — ramię"),
        leg: m(&c.leg, "postać — noga"),
        boot: m(&c.boot, "postać — but"),
        buckle: m(&c.buckle, "postać — klamra"),
        visor: m(&c.visor, "postać — wizjer"),
    }
}

/// Ściąga spojrzenie w pionie do bezpiecznego zakresu.
///
/// Bez tego gracz mógłby obrócić kamerę pod nogi i stracić
/// horyzont — a bez horyzontu nie ma nieba, czyli całej atmosfery.
fn clamp_pitch(p: f32) -> f32 {
    p.clamp(-PITCH_LIMIT, PITCH_LIMIT)
}

/// Ustawia kamerę za postacią.
///
/// Kamera NIE jest dzieckiem postaci: stoi w jej miejscu, patrzy
/// w stronę, którą gracz wybrał myszą, i podąża za nim z
/// opóźnieniem. Gdyby była sztywno związana z kierunkiem postaci,
/// obrót myszą nie działałby wcale.
fn update_camera(app: &mut App3d, dt: f32) {
    let p = &app.sim.player;
    let focus = p.pos + Vec3::new(0.0, 1.35, 0.0);

    // Cel zależy od kąta kamery i pochylenia. Przy pochyleniu w dół
    // kamera schodzi niżej — inaczej patrzyłaby w sufit.
    let (sy, cy) = app.cam_yaw.sin_cos();
    let flat = CAM_BACK * app.cam_pitch.cos();
    let target = focus + Vec3::new(sy * flat, CAM_BACK * app.cam_pitch.sin(), cy * flat);

    // Odrzut kamery po trafieniu. Maleje wykładniczo, więc obraz
    // wraca do spokoju płynnie, a nie skokiem.
    let shake = app.sim.player.shake.max(0.0);
    let jitter = if shake > 0.0 {
        // 0.06 m: przy tej amplitudzie kamera „szarpie", ale kadry
        // nadal da się odczytać. Większa wartość wygląda jak
        // niestabilność, nie jak efekt.
        let a = app.sim.time * 47.0;
        Vec3::new(a.sin() * 0.06, a.cos() * 1.7 * 0.06, 0.0) * shake
    } else {
        Vec3::ZERO
    };

    if let Ok(mut scene) = app.scene.lock() {
        let cam = scene.camera_mut();
        // Wygładzanie: kamera podąża za postacią z opóźnieniem.
        // Bez tego gracz skacze po każdym kroku i kadr jest nieczytelny.
        let k = (12.0 * dt).min(1.0);
        cam.position = cam.position.lerp(target + jitter, k);
        // Cel patrzenia ustawiamy BEZ wygładzania — opóźnienie celu
        // daje efekt „zaciągnięcia", który jest bardziej irytujący niż
        // opóźnienie samej pozycji.
        cam.target = focus;
    }
}

/// Ustawienia post-processingu dla tej sceny.
///
/// Osobna funkcja, bo to jest jedno miejsce, w którym widać CAŁY
/// look. Każda wartość ma komentarz z uzasadnieniem, bo te liczby
/// decydują o charakterze obrazu bardziej niż geometria.
fn apply_post_settings(app: &mut App3d) {
    let Ok(mut scene) = app.scene.lock() else {
        return;
    };
    let p = scene.post_settings_mut();
    // Nadpisania usunięte: ekspozycja, bloom, winieta, aberracja,
    // ziarno, nasycenie, kontrast, SSAO, SSR, SSS, DoF, anamorficzna
    // poświata, flara i podział tonów są ustawieniami silnika
    // (`PostSettings::default()`), nie decyzją tego demo.
    //
    // Powód: demo nadpisywało m.in. `dof_max_blur = 3.5` (globalnie
    // 0.10) i `ssao_strength = 0.45` (globalnie 0.21). Dopóki te
    // nadpisania istnieją, zmiana czegokolwiek w
    // `PostSettings::default()` nie daje żadnego efektu w tym demo —
    // a to są dokładnie te wartości, którymi ktoś próbował usunąć
    // „mętność" obrazu.
    //
    // Ekspozycja celowo zostaje 1.0 (punkt odniesienia AgX) — nie
    // 1.15. Podnoszenie ekspozycji przy włączonej krzywej AgX
    // przesuwa cały obraz w górę zakresu i zjada rozpiętość
    // tonalną; jasność leży w natężeniu światła i albedo, nie w
    // mnożniku przed tonemapem.
    // --- kontury ekranowe --------------------------------------------
    //
    // Jedyny efekt, ktory to demo nadpisuje: obrys sylwetki jest
    // elementem rysunkowym sceny, a nie ustawieniem silnika.
    p.outline_strength = 0.22;
    p.outline_width = 0.9;
    p.outline_threshold = 0.010;
    p.outline_fade = 0.75;
    // Gestosc marszu promieni slonecznych.
    p.god_ray_density = 6.0;
}

/// Macierz postaci przyklejonej do punktu: obrot wokół Y i pozycja.
fn place(pos: Vec3, yaw: f32, scale: Vec3) -> uran_math::Mat4 {
    model_matrix(pos, yaw, 0.0, 0.0, scale)
}

/// Dokłada jedną bryłę postaci, obracając ją wokół jej osi.
/// Składa listę obiektów 3D na bieżącą klatkę.
///
/// Kolejność ma znaczenie tylko dla czytelności kodu — głębią i tak
/// zajmuje się depth buffer, a potok nie ma mieszania alfa.
fn build_draw_list(app: &mut App3d) {
    let mut cmds: Vec<DrawCmd> = Vec::with_capacity(96);
    let m = &app.meshes;
    let mat = &app.mats;
    let time = app.sim.time;

    // --- podłoga i podest ---
    cmds.push(
        DrawCmd::new(m.floor, model_matrix(Vec3::ZERO, 0.0, 0.0, 0.0, Vec3::ONE))
            .with_material(mat.floor),
    );
    cmds.push(
        DrawCmd::new(m.dais, model_matrix(Vec3::ZERO, 0.0, 0.0, 0.0, Vec3::ONE))
            .with_material(mat.structure),
    );

    // --- reaktor ---
    //
    // Rdzeń pulsuje: promień ±3% z okresem 2.4 s. Puls daje scenie
    // „życie" i przy okazji animuje anamorficzną poświatę, która
    // inaczej wyglądałaby jak statyczny rozbłysk.
    let pulse = 1.0 + 0.03 * (time * std::f32::consts::TAU / 2.4).sin();
    cmds.push(
        DrawCmd::new(
            m.core_inner,
            model_matrix(
                Vec3::new(0.0, 3.4, 0.0),
                time * 0.25,
                0.0,
                0.0,
                Vec3::splat(pulse),
            ),
        )
        .with_material(mat.core),
    );
    cmds.push(
        DrawCmd::new(
            m.core_shell,
            model_matrix(Vec3::ZERO, time * 0.12, 0.0, 0.0, Vec3::ONE),
        )
        .with_material(mat.structure),
    );

    // --- otoczenie: słupy, belki, skrzynie, listwy, rury ---
    //
    // Rozmieszczenie jest celowo NIESYMETRYCZNE. Symetryczny układ
    // wygląda jak kratka testowa; niesymetria sugeruje, że platforma
    // jest czymś używanym, a nie właśnie postawionym.
    for (i, (x, z, yaw)) in [
        (-14.0, -10.0, 0.4f32),
        (-14.0, 10.0, -0.2),
        (14.0, -9.0, 1.1),
        (13.0, 11.0, 0.8),
        (-6.0, 18.0, 0.0),
        (7.0, 18.0, 0.0),
        (-20.0, 2.0, 0.6),
        (20.0, -3.0, 1.4),
    ]
    .iter()
    .enumerate()
    {
        let tall = 4.0 + (i % 3) as f32 * 1.6;
        let scale = Vec3::new(1.0, tall / 4.0, 1.0);
        cmds.push(
            DrawCmd::new(m.pillar, place(Vec3::new(*x, tall, *z), *yaw, scale))
                .with_material(mat.structure),
        );
    }
    // Belki stropowe: przeplatają się nad słupami i dają kadrze
    // poziome linie, które czytają się jako „hala".
    for (x, z, yaw) in [
        (-14.0, 0.0f32, 0.0),
        (14.0, 0.0, 0.0),
        (0.0, 18.0, std::f32::consts::FRAC_PI_2),
    ] {
        cmds.push(
            DrawCmd::new(m.beam, place(Vec3::new(x, 7.6, z), yaw, Vec3::ONE))
                .with_material(mat.structure),
        );
    }
    // Skrzynie: pod kątem, nie osiowo. Równa siatka wygląda jak
    // rozmieszczenie proceduralne, a nie jak ładunek.
    for (x, z, yaw) in [
        (-9.0f32, -16.0f32, 0.5f32),
        (-7.6, -16.4, 0.5),
        (16.0, 6.0, -0.7),
    ] {
        cmds.push(
            DrawCmd::new(m.crate_mesh, place(Vec3::new(x, 0.5, z), yaw, Vec3::ONE))
                .with_material(mat.crate_mat),
        );
    }
    // Rury: biegną wzdłuż krawędzi podestu.
    for (x, z, yaw) in [
        (-9.0f32, 5.2f32, 0.0f32),
        (9.0, -5.2, 0.0),
        (5.2, 0.0, std::f32::consts::FRAC_PI_2),
    ] {
        cmds.push(
            DrawCmd::new(
                m.conduit,
                place(Vec3::new(x, 0.75, z), yaw, Vec3::splat(1.4)),
            )
            .with_material(mat.structure),
        );
    }
    // Listwy emisyjne: podświetlają krawędzie podestu. Dwa kolory
    // naprzemiennie, żeby okolica nie była jednokolorowa.
    for (i, a) in [
        0.0f32,
        std::f32::consts::FRAC_PI_3,
        std::f32::consts::TAU / 3.0,
    ]
    .iter()
    .enumerate()
    {
        let r = 6.4;
        let pos = Vec3::new(a.cos() * r, 0.55, a.sin() * r);
        let neon = if i % 2 == 0 {
            mat.neon_cool
        } else {
            mat.neon_warm
        };
        cmds.push(
            DrawCmd::new(
                m.strip,
                place(pos, a + std::f32::consts::FRAC_PI_2, Vec3::ONE),
            )
            .with_material(neon),
        );
    }

    // --- przeciwnicy ---
    //
    // Skorupa i rdzeń to dwie bryły, bo rdzeń musi ŚWIECIĆ przez
    // szczeliny pancerza. Przy trafieniu skorupa jaśnieje na biało
    // (`flash`) — jedyny sposób, żeby gracz zobaczył trafienie bez
    // licznika na HUD-zie.
    for drone in &app.sim.drones {
        let flash = (drone.flash / 0.18).clamp(0.0, 1.0);
        let shell_tint = [1.0 + flash * 3.0, 1.0 + flash * 3.0, 1.0 + flash * 3.0, 1.0];
        cmds.push(
            DrawCmd::tinted(
                m.drone_shell,
                place(drone.pos, drone.yaw, Vec3::ONE),
                shell_tint,
            )
            .with_material(mat.structure),
        );
        let core = drone.pos + Vec3::new(0.0, 0.10, 0.0);
        cmds.push(
            DrawCmd::new(m.drone_core, place(core, drone.yaw * 2.0, Vec3::ONE))
                .with_material(mat.alert),
        );
    }

    // --- pociski ---
    //
    // Obracamy pocisk wzdłuż jego osi, żeby wydłużona bryła wskazywała
    // kierunek lotu. Bez obrotu jest zwyczajnym klockiem, a gracz nie
    // widzi, czy pocisk leci w jego stronę, czy od niego.
    for bolt in &app.sim.bolts {
        // Kąt obrotu wokół Y z kierunku lotu, plus 90°, bo siatka
        // pocisku jest wydłużona w osi Z.
        let yaw = bolt.dir.x.atan2(bolt.dir.z) + std::f32::consts::FRAC_PI_2;
        cmds.push(
            DrawCmd::new(m.bolt, place(bolt.pos, yaw, Vec3::ONE)).with_material(mat.neon_warm),
        );
    }

    // --- iskry trafień ---
    for im in &app.sim.impacts {
        let k = im.fade();
        // Iskra rośnie przy zniknięciu? Nie — maleje, jak ogień.
        cmds.push(
            DrawCmd::new(m.spark, place(im.pos, 0.0, Vec3::splat(0.6 + k * 0.8)))
                .with_material(mat.spark),
        );
    }

    push_character(&mut cmds, app);

    if let Ok(mut scene) = app.scene.lock() {
        scene.set_commands(cmds);
    }
}

/// Dokłada bryły postaci z aktualną animacją.
///
/// Postać jest zbudowana z osobnych siatek dla każdego stawu, bo
/// jedna skinned siatka wymagałaby macierzy kości i zupełnie innego
/// potoku. Przy jednej postaci i kilkunastu częściach ręczne
/// składanie macierzy jest tańsze i czytelniejsze.
fn push_character(out: &mut Vec<DrawCmd>, app: &App3d) {
    let p = &app.sim.player;
    let m = &app.meshes;
    let mat = &app.mats.char;
    let stance = app.sim.stance();

    // Faza chodu: dwie nogi są w przeciwfazie, więc `sin` i `-sin`.
    // Amplituda zależy od prędkości, bo sztywne nogi przy chodzącym
    // tułowiu wyglądają jak manekin.
    let stride = stance.stride();
    let gait = p.gait;
    let leg_l = gait.sin() * stride;
    let leg_r = -gait.sin() * stride;

    // Skłon tułowia do przodu przy biegu. To przechyła sylwetkę
    // i jest najtańszym sposobem na zrobienie wrażenia „mocy".
    let lean = stance.lean();
    let bob = (gait * 2.0).sin() * stride * 0.045;
    let origin = p.pos + Vec3::new(0.0, bob, 0.0);

    // Macierz bazowa: pozycja stóp + obrót postaci + skłon.
    let base =
        model_matrix(origin, p.yaw, 0.0, 0.0, Vec3::ONE) * uran_math::Mat4::from_rotation_x(lean);

    let mut part = |mesh: MeshId, material: MaterialId, offset: Vec3, extra: uran_math::Mat4| {
        let mtx = base * uran_math::Mat4::from_translation(offset) * extra;
        out.push(DrawCmd::new(mesh, mtx).with_material(material));
    };

    // --- tors i głowa ---
    part(m.torso, mat.suit, Vec3::ZERO, uran_math::Mat4::IDENTITY);
    part(m.hips, mat.suit_dark, Vec3::ZERO, uran_math::Mat4::IDENTITY);
    part(m.buckle, mat.metal, Vec3::ZERO, uran_math::Mat4::IDENTITY);
    part(m.head, mat.skin, Vec3::ZERO, uran_math::Mat4::IDENTITY);
    part(m.hair, mat.hair, Vec3::ZERO, uran_math::Mat4::IDENTITY);
    part(
        m.hair_bright,
        mat.hair_bright,
        Vec3::ZERO,
        uran_math::Mat4::IDENTITY,
    );
    part(m.visor, mat.visor, Vec3::ZERO, uran_math::Mat4::IDENTITY);

    // --- kończyny ---
    //
    // Obrót jest wokół osi X, a nie Z: siatki rąk i nóg sięgają w dół
    // (oś -Y), więc obrót wokół X macha nimi w przód-tył. Obrót
    // wokół Z byłby obrotem na boki, czyli dygotaniem.
    let swing = |a: f32| uran_math::Mat4::from_rotation_x(a);
    let rig = character::rig();
    for (side, leg_swing) in [(-1.0f32, leg_l), (1.0, leg_r)] {
        // Noga: biodro → kolano. Obrót w biodrze, potem kolano
        // zginane w przeciwną stronę (jak u ludzi).
        let hip = Vec3::new(side * 0.075, rig.hip_y, 0.0);
        let thigh = swing(leg_swing);
        out.push(
            DrawCmd::new(m.leg, base * uran_math::Mat4::from_translation(hip) * thigh)
                .with_material(mat.suit),
        );
        // But stoi przy kostce i NIE dziedziczy obrotu nogi — inaczej
        // stopa kopałaby w ziemię przy każdym kroku.
        let ankle = base
            * uran_math::Mat4::from_translation(hip)
            * thigh
            * uran_math::Mat4::from_translation(Vec3::new(0.0, -rig.hip_y + rig.ankle_y, 0.0));
        out.push(DrawCmd::new(m.boot, ankle).with_material(mat.suit_dark));

        // Ramię: bark → łokieć, z przeciwfazą.
        let shoulder = Vec3::new(side * 0.175, rig.shoulder_y, 0.0);
        out.push(
            DrawCmd::new(
                m.arm,
                base * uran_math::Mat4::from_translation(shoulder) * swing(leg_swing * 0.75),
            )
            .with_material(mat.suit),
        );
    }
}

/// Czcionki systemowe — sprawdzane po kolei, gdy brak własnej.
///
/// Ta sama lista co w pozostałych grach repozytorium, żeby dodanie
/// czcionki nie wymagało poprawiania czterech plików.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
];

/// Czytanie wejścia i krok symulacji.
fn update(ctx: &mut Ctx, app: &mut App3d) {
    if ctx.input.just_pressed(Key::KeyR) {
        app.sim = Sim::new();
    }

    // --- mysz: obrót kamery ---
    //
    // `mouse_delta` jest limitowany przez silnik, więc skok kursora
    // po refocus nie obróci kamery o pół obrotu.
    let d = ctx.input.mouse_delta();
    // 0.0032 rad/piksel: 1000 DPI daje wtedy ~0.2 rad na milimetr,
    // czyli obrót o 90° wymaga ruchu mysą przez ~8 cm. Pływa,
    // ale nie jest „nerwowy".
    const SENS: f32 = 0.0032;
    app.cam_yaw -= d.x * SENS;
    app.cam_pitch = clamp_pitch(app.cam_pitch + d.y * SENS);

    // --- ruch ---
    let input = Input {
        // `axis(a, b)` zwraca −1 dla `a`, +1 dla `b`. Ten sam
        // schemat co w `uran-tanks`, żeby sterowanie w całym repo
        // było spójne.
        move_dir: [
            ctx.input.axis(Key::KeyA, Key::KeyD),
            ctx.input.axis(Key::KeyS, Key::KeyW),
        ],
        // `pressed` (nie `just_pressed`) — ogień jest ciągły przy
        // trzymaniu przycisku, a `just_pressed` dałby po jednym
        // pocisku na kliknięcie i przy automatycznym powtarzaniu
        // systemu wejścia strzelanie byłoby niemożliwe.
        fire: ctx.input.mouse_pressed(MouseButton::Left),
        sprint: ctx.input.pressed(Key::ShiftLeft),
        jump: ctx.input.just_pressed(Key::Space),
    };
    let dt = ctx.dt();
    app.sim.update(dt, &input, app.cam_yaw);

    update_camera(app, dt);
    if let Ok(mut scene) = app.scene.lock() {
        // Czas świata napędza puls rdzenia i animację nieba; bez tego
        // scena wyglądałaby jak zrzut z edytora.
        scene.advance(dt);
    }
    build_draw_list(app);
}

/// HUD w przestrzeni EKRANU (renderer 2D, po 3D).
///
/// Celownik jest celowo minimalny: cztery krótkie linie, bez pierścieni
/// i bez liczb. W kadrze z mocnymi konturami i poświatą każdy dodatkowy
/// element HUD-u walczyłby o uwagę z samą grą.
fn draw_hud(ctx: &mut Ctx, app: &App3d) {
    let size = ctx.window.logical_size();
    let cx = size.x * 0.5;
    let cy = size.y * 0.5;

    // --- celownik ---
    //
    // Kolor zależy od amunicji: pusty magazyn musi być widoczny
    // natychmiast, a nie dopiero po strzale.
    let tint = if app.sim.player.ammo > 0 {
        Color::from_hex(0x9FE8FF)
    } else {
        Color::from_hex(0xFF6B5A)
    };
    let gap = 5.0;
    let len = 9.0;
    for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let from = Vec2::new(cx + dx * gap, cy + dy * gap);
        let to = Vec2::new(cx + dx * (gap + len), cy + dy * (gap + len));
        ctx.gfx.color(tint).draw_line(from, to, 2.0);
    }

    // --- amunicja: jedna kropka na nabój ---
    //
    // Kropki zamiast paska: widać ile zostało i widać, kiedy broń się
    // ładuje, a pasek wymagałby liczenia przy odczycie.
    let ammo = app.sim.player.ammo;
    for i in 0..MAX_AMMO {
        let lit = i < ammo;
        let p = Vec2::new(cx - 26.0 + (i as f32) * 11.0, cy + 22.0);
        let r = if lit { 3.0 } else { 2.0 };
        let col = if lit {
            Color::from_hex(0x9FE8FF)
        } else {
            Color::from_hex(0x3A4550)
        };
        ctx.gfx.color(col).draw_circle(p, r);
    }

    // --- wskaźnik odrzutu ---
    //
    // Czerwona obręcz pojawia się w chwili trafienia. Bez niej gracz
    // nie wie, dlaczego sterowanie „nie działa" przez pół sekundy.
    let stun = app.sim.player.stun;
    if stun > 0.0 {
        let a = (stun / game::HITSTUN_TIME).clamp(0.0, 1.0);
        ctx.gfx
            .screen_space()
            .color(Color::from_hex(0xFF4A3A).with_alpha(a * 0.5))
            .draw_ring(Vec2::new(cx, cy), 40.0, 2.0);
    }

    let Some(font) = app.font else { return };
    let g = ctx
        .gfx
        .screen_space()
        .layer(100)
        .color(Color::from_hex(0xCFE2F5));

    // UWAGA: tylko ASCII. Polskie znaki diakrytyczne wywołują błąd
    // atlasu glifów w `uran-render` (przy przepełnieniu robi
    // `reset_atlas()` w trakcie klatki, przez co wcześniej wypisane
    // UV-y wskazują na wyczyszczony atlas i glify się zlewają).
    g.draw_text(
        font,
        &format!(
            "kills {}   drones {}   ammo {}",
            app.sim.kills,
            app.sim.drones.len(),
            ammo
        ),
        Vec2::new(24.0, 40.0),
        18.0,
        TextAlign::Left,
    );
    g.draw_text(
        font,
        "WASD move   mouse camera   LMB fire   Shift run   Space jump   R restart",
        Vec2::new(24.0, 64.0),
        14.0,
        TextAlign::Left,
    );
}

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

/// Start: czcionka + podpięcie sceny 3D do renderera 2D.
///
/// JEDEN `Device` obsługuje oba renderery — patrz
/// `uran_render::scene3d`, dlaczego osobne `Surface` nie wchodzi
/// w grę.
fn setup(ctx: &mut Ctx, app: &mut App3d) {
    for path in FONT_FALLBACKS {
        if app.font.is_some() {
            break;
        }
        if let Ok(h) = ctx.assets.load_font(path) {
            app.font = Some(h);
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

/// Punkt wejścia.
fn main() {
    let game = Arc::new(Mutex::new(None::<App3d>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if guard.is_some() {
                return;
            }
            // `Renderer::gpu()` daje to samo urządzenie, którego używa
            // renderer 2D; format i MSAA muszą się zgadzać.
            let built = {
                let r = ctx.renderer.as_ref().expect("renderer gotowy");
                App3d::new(r.gpu(), r.format(), r.samples())
            };
            *guard = Some(built);
            let g = guard.as_mut().expect("właśnie utworzone");
            setup(ctx, g);
            apply_post_settings(g);
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
                .title("Endfield 3D — industrial action demo")
                .background(0x54687E)
                .vsync(true)
                // `samples(1)` jest WYMAGANE przez post-processing:
                // efekty ekranowe czytają głębokość i G-Buffer jako
                // tekstury, a multisampling ich nie obsługuje.
                // Przy wyższej wartości kopia głębokości byłaby
                // niespójna z kolorem, a zrzut wyglądałby jak plama.
                .samples(1),
        )
        // 90 klatek (~1,5 s): postać zdąży się rozchodzić, pojawią
        // się pociski i iskry, a scena nie zdąży się znudzić.
        .screenshot(screenshot_path(), 90)
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}
