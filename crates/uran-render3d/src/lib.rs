//! Renderer 3D — osobny podsystem, współdzielący z rendererem 2D tylko
//! `Device`/`Queue`/`Surface` (patrz `uran_render::scene3d`).
//!
//! Świadomie NIC tu nie dziedziczy po kodzie 2D: własne shadery, własny
//! depth buffer, własne potoki, własna kamera. Jedyna rzecz wspólna to
//! urządzenie GPU i obietnica klatki.
//!
//! # Styl oświetlenia
//!
//! Shader sceny realizuje model hybrydowy PBR + NPR, jaki widać w
//! gatunku „semi-realistic anime" (Arknights: Endfield, Mugen, Night
//! Abyss): warstwa bazowa dyfuzji przechodzi przez miękką rampę
//! rysunkową, a warstwa odbić zostaje w pełni fizyczna. Otoczenie
//! dostaje fizyczne niebo (Rayleigh + Mie), mgłę z rozpraszaniem Mie,
//! a post-processing składa z tego SSR, kontury ekranowe zanikające
//! w świetle, SSS, DoF, anamorficzny bloom, flarę i podział tonów.
//!
//! ## Jak to jest złożone
//!
//! ```text
//!   [pass cieni]  → głębokość z pozycji słońca
//!   [pass główny] → HDR (kolor) + G-Bufer (normalna + chropowatość)
//!                   + niebo jako pełnoekranowy trójkąt
//!   [kopia]       → głębokość do tekstury czytelnej przez postfx
//!   [postfx]      → god rays → SSR → bloom → kompozycja
//! ```

pub mod camera;
pub mod import;
pub mod material;
pub mod mesh;
pub mod postfx;
pub mod scene;
pub mod shadow;

pub use camera::{Camera3d, Ray};
pub use import::{ImportedModel, Material, MaterialLib, ModelPart};
pub use material::{MaterialBank, MaterialId, Surface};
pub use mesh::{box_corners, model_matrix, GpuMesh, InstanceModel, Mesh, SceneUniform, Vertex};
pub use postfx::{PostFx, PostSettings};
pub use scene::{Atmosphere, DrawCmd, Lighting, MeshId, Renderer3d, Stylization};
pub use shadow::{SceneBounds, ShadowMap, ShadowSettings, SHADOW_RES};

#[cfg(test)]
mod shader_tests {
    //! Testy parsujące WGSL.
    //!
    //! Shader jest zwykłym plikiem włączonym przez `include_str!`, więc
    //! literówka w nim nie zatrzymuje kompilacji Rusta — wgpu zgłosi błąd
    //! dopiero przy pierwszym uruchomieniu gry, już po zbudowaniu okna
    //! i załadowaniu assetów. Te testy przenoszą wykrywanie błędów na
    //! `cargo test`, gdzie nie ma GPU i nie trzeba okna.

    use naga::valid::{Capabilities, ValidationFlags, Validator};

    /// Parsuje i waliduje WGSL, zwracając błędy jako tekst.
    ///
    /// Zwraca `Err` z pełnym komunikatem naga, żeby test pokazał nie
    /// tylko „coś jest nie tak", ale gdzie — naga dokleja do komunikatu
    /// numer linii i kolumnę.
    fn validate(label: &str, source: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(source)
            .map_err(|e| format!("{label}: błąd parsowania WGSL:\n{e:?}"))?;
        Validator::new(
            ValidationFlags::all(),
            // `Capabilities::default()` = pusty zestaw, czyli „nic
            // dodatkowego nie jest włączone". To najostrzejsza
            // konfiguracja: wychwyci użycie rozszerzeń, których
            // sterownik może nie mieć.
            Capabilities::default(),
        )
        .validate(&module)
        // naga 0.19 zwraca tu `Result<ModuleInfo, WithSpan<...>>`.
        // `ModuleInfo` nas nie interesuje, więc odrzucamy wynik —
        // interesuje nas tylko wariant błędu.
        .map(|_| ())
        .map_err(|e| format!("{label}: błąd walidacji WGSL:\n{e:?}"))
    }

    #[test]
    fn scene_shader_parses() {
        let src = include_str!("s3d.wgsl");
        if let Err(e) = validate("s3d.wgsl", src) {
            panic!("{e}");
        }
    }

    #[test]
    fn post_shader_parses() {
        let src = include_str!("post.wgsl");
        if let Err(e) = validate("post.wgsl", src) {
            panic!("{e}");
        }
    }

    /// Sprawdza, że oba potoki, których używa `scene.rs`, istnieją
    /// w shaderze sceny pod dokładnie tymi nazwami.
    ///
    /// To chroni przed scenariuszem, w którym ktoś zmieni nazwę
    /// entry pointu w WGSL, a `scene.rs` zostanie stary — wtedy
    /// wgpu odrzuci potok z komunikatem, który nie mówi, o jaką
    /// nazwę chodzi.
    #[test]
    fn scene_shader_exposes_expected_entry_points() {
        let src = include_str!("s3d.wgsl");
        for entry in [
            "fn vs_main",
            "fn vs_shadow",
            "fn vs_sky",
            "fn fs_main",
            "fn fs_sky",
            // Skinning. Te dwa muszą istnieć obok `vs_main`, bo potok
            // postaci różni się wyłącznie punktem wejścia — `fs_main`
            // jest wspólny, więc postać oświetla się dokładnie tak
            // jak bryły.
            "fn vs_skin",
            "fn vs_skin_shadow",
        ] {
            assert!(
                src.contains(entry),
                "s3d.wgsl nie zawiera punktu wejścia `{entry}`"
            );
        }
    }

    /// To samo dla potoków post-processingu.
    #[test]
    fn post_shader_exposes_expected_entry_points() {
        let src = include_str!("post.wgsl");
        for entry in [
            "fn vs_fullscreen",
            "fn fs_bright",
            "fn fs_blur",
            "fn fs_ssr",
            "fn fs_godray",
            "fn fs_ssao",
            "fn fs_ao_blur",
            "fn fs_composite",
        ] {
            assert!(
                src.contains(entry),
                "post.wgsl nie zawiera punktu wejścia `{entry}`"
            );
        }
    }

    /// Każde `@group(N) @binding(M)` w shaderze ma swój wpis w layoucie.
    ///
    /// `wgpu` waliduje to dopiero przy `create_bind_group`, ale błąd
    /// jest wtedy nieczytelny („binding not found"). Ten test porównuje
    /// deklaracje WGSL z layoutami w kodzie Rust.
    #[test]
    fn post_bindings_match_layout_entries() {
        let src = include_str!("post.wgsl");
        // Layout z `postfx.rs`: uniform, src, bloom, sampler,
        // gbuffer, depth, ssr, rays, ao.
        let expected = [0u32, 1, 2, 3, 4, 5, 6, 7, 8];
        for b in expected {
            let needle = format!("@binding({b})");
            assert!(
                src.contains(&needle),
                "post.wgsl nie deklaruje bindingu {b}, a layout go wymaga"
            );
        }
    }

    /// `Surface::base` musi docierać do shadera.
    ///
    /// Bez tego `fs_main` brał kolor wyłącznie z wierzchołka, więc
    /// programowy materiał (`Surface::skin`, `Surface::metal`) był
    /// po cichu ignorowany. Test pilnuje obu rzeczy naraz: że pole
    /// istnieje w uniformie i że `mix` po nim sięga.
    #[test]
    fn material_base_color_reaches_the_shader() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("base: vec4<f32>"),
            "s3d.wgsl: struktura Material nie ma pola `base`"
        );
        assert!(
            src.contains("mat.base.rgb"),
            "s3d.wgsl: fs_main nie używa `mat.base.rgb`, więc kolor \
             materiału z kodu nigdy nie trafi do piksela"
        );
        // Import `.obj` wkleja `Kd` do wierzchołka, więc `base.w`
        // musi wybierać między tymi dwoma źródłami, a nie mnożyć je.
        assert!(
            src.contains("mix(in.color, mat.base.rgb, mat.base.w)"),
            "s3d.wgsl: baza musi być wybierana flagą, nie mnożona"
        );
    }

    /// Skinning musi mieć swój własny binding kości — w grupie 2.
    ///
    /// Gdyby macierz kości leżała w grupie 0 (wspólnej z `models`),
    /// każda statyczna bryła świata musiałaby dostać sztuczny bufor
    /// na 73 macierze. Osobna grupa sprawia, że `DrawCmd` dla ścian
    /// w ogóle jej nie dotyka.
    #[test]
    fn skinning_ma_wlasny_binding_kosci() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("@group(2) @binding(0) var<storage, read> bones"),
            "s3d.wgsl nie deklaruje kości w grupie 2"
        );
        // Wagi muszą być normalizowane w shaderze: plik gwarantuje sumę
        // ~1, ale eksporterzy bywają niedokładni, a nieznormalizowane
        // wagi dają ciemne smugi na krawędziach skóry.
        assert!(
            src.contains("normalized_weights"),
            "s3d.wgsl nie normalizuje wag skinningu"
        );
    }

    /// Efekty nie mogą być agresywne **domyślnie**.
    ///
    /// Ustawienia domyślne są tym, co dostaje demo, które niczego nie
    /// ustawia — a większość ich nie ustawia. Za agresywne wartości
    /// sprawiają, że obraz wygląda „chory" (anamorficzne kreski, szare
    /// plamy AO, plamiste niebo od bloomu) zamiast atrakcyjny.
    ///
    /// Test jest celowo wartościowy, a nie „mniejszy niż poprzednio":
    /// gdyby ktoś podniósł `ssao_strength` do 0.9, test by zadal
    /// pytanie „po co", zamiast przechodzić po cichu.
    #[test]
    /// Ostatnie wartości dobrane odręcznie — traktowane jako kontrakt.
    ///
    /// Test `domyslne_efekty_sa_lagodne` pilnuje tylko górnych progów
    /// („nie za mocno"). Ten pilnuje dokładnych liczb, bo efekt, o który
    /// chodziło, brzmiał wprost: obraz był mocno przepłacony efektami
    /// i każde dalsze podbicie któregokolwiek z tych parametrów wraca
    /// do tego samego problemu.
    ///
    /// Wyjątek: `dof_max_blur` ma minimalne, ale niezerowe granice —
    /// przy 0.0 shader dzieli przez `max(d, 0.001)`, co daje nieskończony
    /// blur na pikselach w płaszczyźnie ogniskowania.
    #[test]
    /// Ambient musi być **dużo mniejszy** od natężenia słońca.
    ///
    /// Obraz wychodził „jak przez mętną soczewkę" właśnie dlatego, że
    /// `ambient` wynosił ~0.4 przy słońcu 4.0. Stosunek poniżej ~0.05
    /// daje realny kontrast: strona w cieniu jest ciemna, a nie tylko
    /// „inny odcień tego samego jasnego koloru".
    #[test]
    fn ambient_nie_wypelnia_cieni() {
        let l = crate::scene::Lighting::default();
        let ambient_luma: f32 =
            l.ambient[0] * 0.2126 + l.ambient[1] * 0.7152 + l.ambient[2] * 0.0722;
        let sun_luma: f32 =
            l.light_color[0] * 0.2126 + l.light_color[1] * 0.7152 + l.light_color[2] * 0.0722;
        let ratio = ambient_luma / (sun_luma * l.intensity);

        assert!(
            ratio < 0.05,
            "ambient:słońce = {:.3} — cienie wypełnione (prawidłowo < 0.05)",
            ratio
        );
        // Cienie nadal muszą być widoczne jako „niebo z wnętrza",
        // a nie czysta czerń — inaczej scena wygląda jak wyłączony
        // silnik, a nie jak niedoświetlone pomieszczenie.
        assert!(
            ambient_luma > 0.01,
            "ambient = {} — zbyt ciemno, cienie staną się dziurami",
            ambient_luma
        );
    }

    /// Kalibracja obrazu musi być aktywna domyślnie.
    ///
    /// Te trzy pola odpowiadają na zgłoszone problemy: mętna soczewka
    /// (ostrość), szare cienie (lift), brak różnicy między materiałami
    /// (chropowatość). Wyłączone zostawiają obraz dokładnie takim,
    /// jak był przed poprawką.
    #[test]
    fn kalibracja_obrazu_jest_wlaczona() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        assert!(
            p.clarity > 0.3,
            "clarity = {} — obraz będzie znowu mętny",
            p.clarity
        );
        // Powyżej 1.5 unsharp daje halo (białą obwódkę) na
        // sylwetkach, a to dokładnie ten defekt, który usuwamy.
        assert!(
            p.clarity <= 1.5,
            "clarity = {} — pojawi się halo na krawędziach",
            p.clarity
        );
        assert!(
            p.shadow_lift > 0.0 && p.shadow_lift <= 0.05,
            "shadow_lift = {} — poza zakresem 0..=0.05",
            p.shadow_lift
        );
        // `roughness_bias` celowo 0: korekta chropowatości to decyzja
        // sceny, a nie silnika. Test pilnuje tylko, że pole istnieje
        // i mieści się w zadeklarowanym zakresie.
        assert!(
            (-1.0..=1.0).contains(&p.roughness_bias),
            "roughness_bias = {} poza -1..1",
            p.roughness_bias
        );
    }

    /// Nasycenie i kontrast muszą być wyżej niż 1.
    ///
    /// AgX celowo odbarwia kolory (żeby światła nie były plamami), więc
    /// domyślne 1.0 dawało wyblakłą scenę. Podbicie po tonemapie
    /// odzyskuje intensywność materiałów.
    #[test]
    fn kolory_sa_nasycone_a_ma_kontrast() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(
            p.saturation > 1.05,
            "saturation = {} — scena wygląda wyblakła",
            p.saturation
        );
        assert!(
            p.contrast > 1.05,
            "contrast = {} — brak głębi tonalnej",
            p.contrast
        );
    }

    fn domyslne_sa_dokladnie_takie_jak_zamowiono() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        assert_eq!(p.ssao_strength, 0.21, "siła SSAO");
        assert_eq!(p.ssao_radius, 0.12, "promień SSAO w metrach");
        assert_eq!(p.outline_strength, 0.02, "kontur");
        assert_eq!(p.ssr_strength, 0.11, "odbicie ekranowe");
        assert_eq!(p.dof_max_blur, 0.10, "maks. blur DoF");
    }

    /// Wszystkie cztery powyższe wartości muszą być **bardzo małe** —
    /// to efekt świadomie wyłączony, nie tylko osłabiony.
    ///
    /// Rozróżnienie jest ważne przy diagnostyce: jeśli AO znika
    /// całkowicie, to oczywista przyczyna to zbyt mały `ssao_strength`
    /// albo `ssao_radius`, a nie błąd w shaderze.
    #[test]
    fn cztery_efekty_sa_zasadniczo_wylaczone() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(p.ssao_strength <= 0.25, "AO nadal aktywne");
        assert!(p.ssao_radius <= 0.15, "promień AO zbyt duży");
        assert!(p.outline_strength <= 0.05, "kontur nadal aktywny");
        assert!(p.ssr_strength <= 0.15, "SSR nadal aktywne");
        assert!(p.dof_max_blur <= 0.5, "DoF nadal aktywny");
    }

    fn domyslne_efekty_sa_lagodne() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        // AO: najbardziej szkodliwy przy dużym promieniu — zaciemnia
        // cały obrys obiektu, a nie sam styk z podłożem.
        assert!(
            p.ssao_strength <= 0.6,
            "ssao_strength = {}",
            p.ssao_strength
        );
        assert!(p.ssao_radius <= 0.4, "ssao_radius = {}", p.ssao_radius);
        assert!(
            p.ssao_intensity <= 1.3,
            "ssao_intensity = {}",
            p.ssao_intensity
        );

        // Kontur powyżej ~0.4 zamienia postać w plakat z grubą ramą.
        assert!(
            p.outline_strength <= 0.4,
            "outline = {}",
            p.outline_strength
        );

        // Anamorficzna poświata powyżej 0.6 daje poziome kreski
        // na każdej jasnej plamie — obraz wygląda „chory".
        assert!(p.anamorphic <= 0.25, "anamorphic = {}", p.anamorphic);
        assert!(p.lens_flare <= 0.20, "lens_flare = {}", p.lens_flare);
        assert!(p.god_rays <= 0.30, "god_rays = {}", p.god_rays);

        // Bloom 0.32+ przy progu 1.15 świecił na każdej jasnej
        // powierzchni zamiast tylko na źródłach.
        assert!(p.bloom <= 0.20, "bloom = {}", p.bloom);

        // Winieta powyżej 0.25 przyciemnia narożnik jak defekt kadru.
        assert!(p.vignette <= 0.20, "vignette = {}", p.vignette);

        // Split tone powyżej 0.4 przesuwa całą paletę w błękit/bursztyn.
        assert!(p.split_tone <= 0.40, "split_tone = {}", p.split_tone);

        // Antyaliasing to NIE efekt — obniżanie dodaje schodki.
        assert!(
            p.antialias >= 0.5,
            "antialias = {} — to nie efekt",
            p.antialias
        );
    }

    /// Odrzucanie próbek AO musi działać domyślnie.
    ///
    /// `0` oznaczałoby całkowite wyłączenie odrzucania, a wtedy
    /// `ssao_reject_fadeoff` cicho nic nie robiłby, a `fs_ssao`
    /// zachowywałby się jak stary test kulowy.
    #[test]
    fn odrzucanie_ao_ma_sensowna_domyslna_wartosc() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(
            (50.0..=200.0).contains(&p.ssao_reject_fadeoff),
            "ssao_reject_fadeoff = {} — poza zakresem użytecznym 50..=200",
            p.ssao_reject_fadeoff
        );
    }

    /// Uniform musi przenosić `reject_fadeoff` do shadera.
    ///
    /// Parametr jedzie w `rays.z` (pole rezerwowe), bo dopisanie
    /// nowego `vec4` przesunęłoby wszystkie późniejsze pola.
    #[test]
    fn uniform_oddaje_parametr_do_pola_rays_z() {
        use crate::postfx::{PostParams, PostSettings};
        let s = PostSettings::default();
        let p = PostParams::from_settings(
            &s,
            /* w */ 1920.0,
            /* h */ 1080.0,
            /* time */ 0.0,
            /* sun */ [0.5; 4],
            /* near */ 0.1,
            /* far */ 1000.0,
            /* focus */ 1.0,
            /* tan_half_fov */ 0.5,
            /* aspect */ 16.0 / 9.0,
        );
        assert_eq!(
            p.rays[2], s.ssao_reject_fadeoff,
            "rays.z musi nieść ssao_reject_fadeoff dla `fs_ssao`"
        );
    }

    /// Mgła musi liczyć **analityczny całkowity** gęstości, a nie
    /// `1 - exp(-dist · ρ)`.
    ///
    /// ## Dlaczego to pilnujemy testem
    ///
    /// Wersja liniowa daje jednolitą, „mleczną" warstwę na każdej
    /// wysokości — dach domu tonął w mgle tak samo jak ulica pod nim.
    /// WickedEngine liczy to analitycznie (`fogHF.hlsli`), więc powtarzamy
    /// u siebie ten sam wzór, żeby zachować zgodny wygląd.
    #[test]
    fn mgla_jest_wysokosciowa_a_nie_jednolita() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("base_distance") && src.contains("exp_distance"),
            "s3d.wgsl nie ma analitycznej całkowitej mgły (brak rozdzielenia \
             na część bazową i eksponencjalną)"
        );
        // Stary, jednolity wzór może istnieć TYLKO jako wypadek przy
        // `fog.x == 0`. Poza tym nie powinien się pojawiać.
        assert!(
            src.contains("fog_amount = 1.0 - exp(-dist * scene.atmos.x)"),
            "s3d.wgsl nie ma nawet przypadku zapasowego dla mgły jednolitej"
        );
        // Anizotropia rozpraszania musi być z parametrizowana — mgła
        // gruntowa i niebo to różne ośrodki o różnej fazie.
        assert!(
            src.contains("henyey_greenstein(dot(-ray_dir, L), FOG_PHASE_G)"),
            "faza mgły musi liczyć kąt z kierunku promienia, nie z `V` \
             (kierunek do oka) — inaczej mgła świeci w złym miejscu"
        );
    }

    /// Rozmiar uniformu sceny musi zgadzać się z WGSL.
    ///
    /// `fog: vec4<f32>` doszło do struktury `Scene`, więc uniform urosnął
    /// o 16 B. Test chroni przed sytuacją, w której Rust i shader
    /// rozjeżdżają się cicho, a wgpu odrzuci bind group dopiero
    /// podczas renderowania.
    #[test]
    fn uniform_sceny_ma_miejsce_na_mgle() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("fog: vec4<f32>"),
            "s3d.wgsl nie deklaruje pola `fog`"
        );
        assert_eq!(
            std::mem::size_of::<crate::mesh::SceneUniform>() % 16,
            0,
            "SceneUniform musi być wielokrotnością 16 B dla WGSL"
        );
    }

    /// Rozmiar uniformu materiału musi zgadzać się z obiektem w WGSL.
    #[test]
    fn material_uniform_is_six_vec4s() {
        assert_eq!(
            std::mem::size_of::<crate::material::MaterialUniform>(),
            96,
            "MaterialUniform musi mieć 6 × vec4, bo WGSL ma 6 pól vec4"
        );
    }
}
