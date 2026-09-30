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

    /// Rozmiar uniformu materiału musi zgadzać się z obiektem w WGSL.
    ///
    /// `MaterialUniform` ma asercję rozmiaru w `material.rs`, ale ta
    /// nie wie nic o WGSL. `vec4` w obu miejscach musi dać 96 B —
    /// inaczej wgpu odrzuci bind group dopiero przy renderowaniu.
    #[test]
    fn material_uniform_is_six_vec4s() {
        assert_eq!(
            std::mem::size_of::<crate::material::MaterialUniform>(),
            96,
            "MaterialUniform musi mieć 6 × vec4, bo WGSL ma 6 pól vec4"
        );
    }
}
