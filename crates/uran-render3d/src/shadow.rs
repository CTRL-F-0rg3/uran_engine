//! Mapowanie cieni kierunkowych: macierz światła i mapa głębokości.
//!
//! ## Jak to działa
//!
//! Cienie są **kierunkowe** (jedno słońce), bo taki model pasuje do
//! [`crate::scene::Lighting`], który trzyma jeden `light_dir`. Światło
//! kierunkowe zamienia się w punktowe tylko wtedy, gdy umieścimy je
//! bardzo daleko — a robimy właśnie to.
//!
//! 1. **Dobieramy kadr.** AABB całej sceny dostaje kwadratowy obszar
//!    ortograficzny liczony wokół jego środka. Kwadrat, bo mapa ma
//!    kwadratowy format — prostokąt dałby niejednakową rozdzielczość
//!    na piksel w obu osiach, czyli rozjeżdżające cienie.
//! 2. **Rysujemy głębokość z pozycji słońca** do tekstury `Depth32Float`.
//!    Pass zapamiętuje WYŁĄCZNIE głębokość, bez color attachmentu.
//! 3. **Porównujemy** głębokość piksela z mapą, z 9 próbek (PCF 3×3)
//!    wyliczamy uśredniony wynik jasny/uciemniony.
//!
//! ## Dlaczego `Depth32Float`, a nie `Depth24Plus`
//!
//! Tekstura głębokości jest jednocześnie buforem depth i źródłem do
//! próbkowania. `Depth32Float` daje pełną precyzję float, więc
//! porównanie z bias-em daje stabilne krawędzie cienia. `Depth24Plus`
//! po zapisie i odczycie potrafi dawać „schodki" na płaskich
//! powierzchniach równoległych do kierunku światła.
//!
//! ## Bias — najczęstsza przyczyna zepsutych cieni
//!
//! Bez biasu powierzchnia „oświetla sama siebie" (*shadow acne*): piksel
//! porównuje głębokość z samą sobą, wynosi 0, a cała płaszczyzna
//! dostaje czarne linie. Rozwiązujemy to na dwa sposoby naraz:
//!
//! * **offset w przestrzeni światła** — przesuwamy próbkę wzdłuż
//!   kierunku do słońca o `depth_bias`,
//! * **normal-offset** — odsuwamy próbkę o `normal * normal_offset`,
//!   co pomaga przy powierzchniach odchylonych od osi światła.
//!
//! ## Koszt
//!
//! Jeden dodatkowy pass rysujący tę samą geometrię — dla 300 obiektów
//! to kilka tysięcy trójkątów ponownie, na GPU praktycznie za darmo.

// `Vec4` uzywa wylacznie modul testow (wektor jednorodny punktu),
// wiec importujemy go tylko przy `cargo test`.
#[cfg(test)]
use uran_math::Vec4;
use uran_math::{Mat4, Vec3};

/// Rozdzielczość mapy cieni w pikselach na krawędź.
///
/// 2048² to standardowy kompromis: 512 daje widoczne schodki na
/// krawędziach, 4096 kosztuje cztery razy więcej pamięci i czasu
/// bez zauważalnej różnicy przy kadrze typowym dla gry.
pub const SHADOW_RES: u32 = 2048;

/// Odległość od obiektu, na której stawiamy oko cienia.
///
/// Światło jest **kierunkowe**, więc jego pozycja nie ma znaczenia —
/// liczy się tylko kierunek. Ten dystans daje tyle, że scena z
/// odległości setek metrów nadal mieści się między płaszczyznami nożyc.
const EYE_BACKOFF: f32 = 400.0;

/// Jak mocno cienie przyciemniają światło bezpośrednie.
#[derive(Debug, Clone, Copy)]
pub struct ShadowSettings {
    /// Przesunięcie próbki w głębokości (0 = wyłączony).
    pub depth_bias: f32,
    /// Przesunięcie próbki wzdłuż normalnej, w jednostkach świata.
    pub normal_offset: f32,
    /// 0.0 = brak cienia, 1.0 = pełny cień.
    pub strength: f32,
    /// Promień PCF w texelach.
    pub radius: f32,
    /// Czy w ogóle rysujemy cienie.
    pub enabled: bool,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            // 0.0016 w jednostkach NDC z głębokości. Przy `Depth32Float`
            // zakres nożyc to 0..1, więc bias musi być wyrażony w tych
            // samych jednostkach — za mały daje acne, za duży „odkleja"
            // cień od obiektu i robi ciemną obwódkę.
            depth_bias: 0.0016,
            // 0.035 m to grubość „skórki" cienia dla obiektów tej skali
            // (farma, budynki, postać). Dla 2-metrowego domu to 1,7%
            // wysokości — niewidoczne, ale wystarczające.
            normal_offset: 0.035,
            // Pełny cień, ale shader miesza go ze światłem rozproszonym,
            // więc w cieniu widać niebieskawe otoczenie, nie czerń.
            strength: 0.78,
            radius: 1.5,
            enabled: true,
        }
    }
}

/// AABB świata, z którego liczymy kadr cienia.
#[derive(Debug, Clone, Copy)]
pub struct SceneBounds {
    pub lo: Vec3,
    pub hi: Vec3,
}

impl SceneBounds {
    /// Pusty zakres gotowy do [`Self::expand`].
    ///
    /// `lo`/`hi` z przeciwnymi nieskończonościami: pierwszy punkt
    /// rozszerzy zakres do samego siebie, a nie zostawi zerowy.
    pub fn empty() -> Self {
        Self {
            lo: Vec3::splat(f32::INFINITY),
            hi: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    /// Wstawia punkt do AABB.
    pub fn expand(&mut self, p: Vec3) {
        self.lo = self.lo.min(p);
        self.hi = self.hi.max(p);
    }

    /// Czy zakres ma sens (co najmniej jeden punkt).
    pub fn is_valid(&self) -> bool {
        self.lo.x.is_finite() && self.lo.x <= self.hi.x
    }

    /// Środek AABB.
    pub fn center(&self) -> Vec3 {
        (self.lo + self.hi) * 0.5
    }

    /// Największa z trzech długości.
    pub fn max_extent(&self) -> f32 {
        (self.hi - self.lo).max_element().max(0.001)
    }

    /// Promień kuli opisanej na AABB: połowa długości przekątnej.
    ///
    /// Kadr cienia to KWADRAT obrócony dookoła kierunku światła, więc
    /// musi objąć całą kulę opisaną na AABB — a nie połowę
    /// najdłuższego boku. Różnica jest bardzo widoczna: dla AABB
    /// 20×12×18 m `max_extent/2` to 10 m, a promień kuli ~14,7 m.
    /// Przy zbyt małym kadrze narożniki bryły wypadają poza mapę,
    /// ich `in_range` daje fałsz i fragmenty są bez cienia.
    pub fn radius(&self) -> f32 {
        ((self.hi - self.lo).length() * 0.5).max(0.001)
    }
}

/// Tekstura + próbnik + macierz światła.
pub struct ShadowMap {
    /// Widok mapy głębokości. Trzymamy go w strukturze, bo `bind_group`
    /// wskazuje na tę teksturę — wgpu nie współdzieli zasobów.
    view: wgpu::TextureView,
    /// Prównywarka (`LessEqual`), nie zwykły sampler.
    sampler: wgpu::Sampler,
    pub settings: ShadowSettings,
    /// Obliczona w klatce, dla debugu i testów.
    light_view_proj: Mat4,
}

impl ShadowMap {
    /// Tworzy teksturę i próbnik porównujący.
    pub fn new(device: &wgpu::Device, settings: ShadowSettings) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uran 3D Shadow Map"),
            size: wgpu::Extent3d {
                width: SHADOW_RES,
                height: SHADOW_RES,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            // 1, nie `samples` z renderera głównego: pass cienia nie ma
            // MSAA, bo rysujemy wyłącznie głębokość. MSAA na samej
            // głębokości nic by nie dało poza kosztem.
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            // `TEXTURE_BINDING` jest konieczne: tę samą teksturę czyta
            // shader sceny. Bez tej flagi walidacja wgpu odrzuci bind group.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran 3D Shadow Sampler"),
            // `compare` czyni z samplera PRÓBOWNIK PORÓWNAWCZY.
            // `LessEqual` znaczy: „oświetlony, jeżeli moja głębokość
            // jest bliższa niż zapisana" — standardowa konwencja shadow
            // map. Bez `compare` tekstury depth nie da się próbkować w WGSL.
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // `ClampToEdge` zamiast `Repeat`: poza kadrzem ma być
            // oświetlenie (kadr dobieramy do całej sceny), a nie wzór.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            view: texture.create_view(&Default::default()),
            sampler,
            settings,
            light_view_proj: Mat4::IDENTITY,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Macierz świata -> przestrzeń cienia, liczona w [`Self::fit`].
    pub fn light_view_proj(&self) -> Mat4 {
        self.light_view_proj
    }

    /// Rozmiar jednego texela w skali UV — do skalowania promienia PCF.
    pub fn texel_uv(&self) -> f32 {
        1.0 / SHADOW_RES as f32
    }

    /// Liczy macierz `light_view_proj` dla podanych granic sceny.
    ///
    /// Światło jest kierunkowe, więc stawiamy oko daleko wzdłuż
    /// `light_dir` (który wskazuje OD źródła) i patrzymy na środek kadru.
    pub fn fit(&mut self, bounds: SceneBounds, light_dir: Vec3) {
        if !bounds.is_valid() {
            // Pusta scena: trzymamy poprzednią macierz, żeby nie mrugać.
            return;
        }
        self.light_view_proj = light_matrix(bounds, light_dir);
    }
}

/// Buduje macierz projekcji cienia dla kierunku `light_dir`.
///
/// `light_dir` wskazuje OD źródła (tak jak [`crate::scene::Lighting`]),
/// więc oko stawiamy w `center + light_dir * EYE_BACKOFF` i patrzymy
/// na środek kadru.
///
/// ## Dlaczego kwadratowy kadr
///
/// Używamy jednego `half` dla obu osi. Gdyby każda oś dostała swój
/// promień, obraz postaciującej (`half / (far - near)`) różniłby się
/// w osi X i Y — ten sam obiekt dawałby inne zniekształcenie zależnie
/// od tego, w którą stronę na niego patrzymy. Kwadrat eliminuje ten
/// problem za cenę trochę zmarnowanych texeli.
pub fn light_matrix(bounds: SceneBounds, light_dir: Vec3) -> Mat4 {
    let center = bounds.center();
    // Kadr to KWADRAT obrócony dookoła osi światła, więc bierzemy
    // promień kuli opisanej na AABB, a nie połowę najdłuższego boku.
    // +1 m marginesu na błąd precyzji głębokości przy samej krawędzi.
    let half = bounds.radius() + 1.0;

    // Światło kierunkowe: kierunek musi być jednostkowy, bo `look_at`
    // normalizuje różnicę oczu i celu. Zerowy wektor daje NaN, a ten
    // NaN trafia do macierzy i psuje CAŁĄ scenę, nie tylko cień.
    let dir = light_dir.normalize_or_zero();
    let dir = if dir == Vec3::ZERO {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        dir
    };

    // Wektor „w górę" musi być NIErównoległy do kierunku patrzenia.
    // Przy słońcu prosto w dół (`dir` = +Y) wektor `up` = +Y jest
    // równoległy do osi oka i `look_at` zwraca macierz pełną NaN —
    // wtedy znika CAŁA scena, nie tylko cień. Dlatego wybieramy
    // dowolną oś nieprostopadłą: +Z, gdy świeci z góry.
    let up = if dir.dot(Vec3::Y).abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    let eye = center + dir * EYE_BACKOFF;
    let view = Mat4::look_at_rh(eye, center, up);

    // Nożyce: od oko do `far`. `near` dodatnie i mniejsze niż
    // odległość, żeby scena nie wyszła za płaszczyznę.
    let near = 0.1;
    let far = EYE_BACKOFF + half * 2.0;
    let proj = Mat4::orthographic_rh(-half, half, -half, half, near, far);
    proj * view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_at(center: Vec3, size: Vec3) -> SceneBounds {
        let mut b = SceneBounds::empty();
        b.expand(center - size * 0.5);
        b.expand(center + size * 0.5);
        b
    }

    /// Wszystkie 8 narożników AABB jako wektory jednorodne.
    fn corners(b: SceneBounds) -> Vec<Vec4> {
        (0..8)
            .map(|i| {
                let p = Vec3::new(
                    if i & 1 == 0 { b.lo.x } else { b.hi.x },
                    if i & 2 == 0 { b.lo.y } else { b.hi.y },
                    if i & 4 == 0 { b.lo.z } else { b.hi.z },
                );
                Vec4::new(p.x, p.y, p.z, 1.0)
            })
            .collect()
    }

    #[test]
    fn expanding_once_makes_bounds_valid() {
        let mut b = SceneBounds::empty();
        assert!(!b.is_valid());
        b.expand(Vec3::new(3.0, 4.0, 5.0));
        assert!(b.is_valid());
        assert_eq!(b.lo, Vec3::new(3.0, 4.0, 5.0));
        assert_eq!(b.hi, Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn centre_and_extent_of_a_box() {
        let b = scene_at(Vec3::ZERO, Vec3::new(10.0, 4.0, 6.0));
        assert!(b.center().abs_diff_eq(Vec3::ZERO, 1e-5));
        // kadr musi objąć NAJDŁUŻSZĄ oś
        assert!((b.max_extent() - 10.0).abs() < 1e-5);
    }

    /// Najważniejszy test: CAŁA scena musi zmieścić się w kadrze.
    ///
    /// Obiekt wychodzący poza mapę cienia albo znika (brak shadow map),
    /// albo rzuca cień, którego nie ma — w obu przypadkach wygląda to
    /// jak dziura w ścianach budynku.
    #[test]
    fn whole_scene_fits_into_shadow_frustum() {
        let bounds = scene_at(Vec3::new(5.0, 0.0, -3.0), Vec3::new(20.0, 12.0, 18.0));
        let m = light_matrix(bounds, Vec3::new(0.4, 0.6, 0.7));

        for (i, p) in corners(bounds).into_iter().enumerate() {
            let clip = m * p;
            // `w` dodatnie — za nim kryją się nożyce, a ujemne
            // oznaczałoby punkt za kamerą cienia.
            assert!(clip.w > 0.0, "narożnik {i} jest za kamerą cienia: {clip:?}");
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0,
                "narożnik {i} wypadł z kadru: {ndc:?}"
            );
            assert!(
                (-1.0..=1.0).contains(&ndc.z),
                "narożnik {i} poza nożycami: {ndc:?}"
            );
        }
    }

    #[test]
    fn a_taller_scene_still_fits() {
        // wysoka wieża wychodząca w górę — najtrudniejszy przypadek,
        // bo to ona definiuje skraj Z przy niskim słońcu.
        let bounds = scene_at(Vec3::ZERO, Vec3::new(8.0, 60.0, 8.0));
        let m = light_matrix(bounds, Vec3::new(0.0, 0.25, 1.0));
        for (i, p) in corners(bounds).into_iter().enumerate() {
            let ndc = (m * p).truncate() / (m * p).w;
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0,
                "narożnik {i} wypadł z kadru: {ndc:?}"
            );
        }
    }

    /// Zerowy kierunek światła to nie wyjątek do zgłoszenia,
    /// tylko stan, który daje deterministyczny wynik.
    #[test]
    fn zero_light_direction_does_not_produce_nan() {
        let bounds = scene_at(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0));
        let m = light_matrix(bounds, Vec3::ZERO);
        let clip = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            clip.x.is_finite() && clip.y.is_finite() && clip.z.is_finite(),
            "zerowy light_dir dał NaN w macierzy: {clip:?}"
        );
    }

    /// Światło prosto w dół: cień musi wciąż trafiać w mapę.
    #[test]
    fn overhead_sun_projects_onto_the_ground() {
        let bounds = scene_at(Vec3::ZERO, Vec3::new(10.0, 2.0, 10.0));
        let m = light_matrix(bounds, Vec3::new(0.0, 1.0, 0.0));
        let clip = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0, "{ndc:?}");
    }

    /// Światło poziome daje długie cienie — i nadal mieszczą się w kadrze.
    #[test]
    fn low_sun_keeps_scene_in_frame() {
        let bounds = scene_at(Vec3::ZERO, Vec3::new(30.0, 3.0, 30.0));
        let m = light_matrix(bounds, Vec3::new(1.0, 0.08, 0.2));
        for (i, p) in corners(bounds).into_iter().enumerate() {
            let clip = m * p;
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && clip.w > 0.0,
                "narożnik {i} wypadł przy niskim słońcu: {ndc:?}"
            );
        }
    }

    #[test]
    fn shadow_map_defaults_are_sane() {
        let s = ShadowSettings::default();
        assert!(s.enabled);
        // bias musi być DODATNI (odpycha próbkę od powierzchni) i mały
        assert!(s.depth_bias > 0.0 && s.depth_bias < 0.01);
        assert!(s.normal_offset > 0.0);
        assert!((0.0..=1.0).contains(&s.strength));
        assert!(s.radius > 0.0 && s.radius < SHADOW_RES as f32);
    }

    #[test]
    fn texel_size_is_inverse_of_resolution() {
        // 1/2048 = 0,000488281
        let t = 1.0 / SHADOW_RES as f32;
        assert!((t - 0.000488281).abs() < 1e-6);
    }
}
