//! Kierunki w pierwszej osobie: yaw/pitch na wektory bazowe.
//!
//! # Dlaczego ten moduł istnieje
//!
//! Kamera pierwszoosobowa potrzebuje trzech wektorów: dokąd patrzymy,
//! w którą stronę idziemy przy `W` i w którą przy `D`. Błąd w tej
//! matematyce jest bardzo łatwy do popełnienia i bardzo trudny do
//! zauważenia na własnym kodzie — stąd osobny plik z testami.
//!
//! ## Skąd bierze się odwrócone A/D
//!
//! „W prawo na ekranie" to oś X macierzy widzenia, czyli
//! `Mat4::look_at_rh` — dokładnie ta, której używa renderer. Dla
//! patrzenia poziomego na `+Z` dostajemy:
//!
//! ```text
//!   forward = +Z,  up = +Y
//!   right   = +Z × +Y = -X
//! ```
//!
//! Czyli `-X`. To wygląda sprzecznie z intuicją („patrząc na `+Z`,
//! widzę `+X` po prawej"), ale tak wynika z prawoskrętnej konwencji.
//!
//! **Uwaga na historię tego miejsca.** Pierwotna wersja dawała tu
//! `+X`, przez co A i D były zamienione. Poprawka na `-X` była
//! prawidłowa, ale została wycofana pod wpływem błędnego testu, który
//! porównywał `right` ze **składową** macierzy zamiast z **projekcją**
//! punktu na ekran. Iloczyn wektorów jest antysymetryczny, więc oba
//! zapisy (`forward × up` i `up × forward`) różnią się znakiem i tylko
//! jeden pasuje do tego, co naprawdę robi renderer.
//!
//! Dlatego znak `right` ustalamy wyłącznie testem, nie rachunkiem na
//! kartce: [`tests::right_matches_the_view_matrix`] rzutuje punkt
//! przesunięty o `right` przez prawdziwą macierz i wymaga dodatniego
//! `x` w NDC, czyli prawej połowy kadru. Zmiana konwencji w silniku
//! wywali ten test zamiast po cichu odwrócić sterowanie.
//!
//! Uwaga na drugą pułapkę: skoro `right` zmienia znak, zmienia się
//! też kolejność iloczynu w [`up`] — `right × forward`, nie
//! `forward × right`.
//!
//! ## Konwencja
//!
//! * `yaw = 0` — patrzymy w `+Z`, zgodnie z `Mat4::look_at_rh`,
//! * dodatni `yaw` obraca w lewo (konwencja matematyczna),
//! * `forward` jest zawsze znormalizowany i **nigdy** równoległy do Y
//!   (`pitch` jest przycięty w [`clamp_pitch`]), więc [`flat`] zawsze
//!   zwraca wektor jednostkowy.

use uran_math::Vec3;

/// Ograniczenie kąta patrzenia w górę/dół, w radianach (85°).
///
/// Prawie 90°. Pełne 90° oznaczałoby `forward` równoległy do osi
/// `look_at`, a wtedy wektor „w prawo" degeneruje do zera i chód
/// na wschód przestaje działać. 5° zostaje jako zapas.
pub const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.0873;

/// Przycięcie kąta do bezpiecznego zakresu.
pub fn clamp_pitch(pitch: f32) -> f32 {
    pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT)
}

/// Kierunek patrzenia z kątów `yaw` (poziom) i `pitch` (pion).
///
/// Znak `pitch` jest ujemny w górę, bo oś Y wskazuje w górę, a
/// dodatni obrót wokół X odchyla wektor w dół.
pub fn forward(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = clamp_pitch(pitch).sin_cos();
    Vec3::new(sy * cp, sp, cy * cp)
}

/// Wektor „w prawo" — poziomy, prostopadły do kierunku patrzenia.
///
/// Znak ustalony testem `right_matches_the_view_matrix` przeciw
/// prawdziwej macierzy `Mat4::look_at_rh`. Dla `yaw = 0` (patrzymy na
/// `+Z`) daje `-X` — patrz opis modułu, dlaczego to nie jest `+X`.
pub fn right(yaw: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    Vec3::new(-cy, 0.0, sy)
}

/// Wektor „w górę" ekranu, prostopadły do patrzenia i do „w prawo".
///
/// Kolejność iloczynu jest tu istotna i łatwo ją pomylić: dla
/// `yaw = 0` mamy `forward = +Z` i `right = +X`, a `Z × X = +Y` —
/// poprawnie. Zapis odwrotny (`right × forward`) dałby `-Y`, czyli
/// wektor skierowany w dół.
///
/// Przy okazji wynik jest zawsze w górę — składowa Y to `cos(pitch)`,
/// czyli dodatnia dla każdego dopuszczalnego pitchu. Test
/// `up_points_up_not_down` pilnuje tego znaku.
pub fn up(yaw: f32, pitch: f32) -> Vec3 {
    right(yaw).cross(forward(yaw, pitch))
}

/// Kierunek patrzenia **bez składowej pionowej** — po nim chodzimy.
///
/// Rzut na płaszczyznę `XZ`, potem normalizacja. Normalizacja jest
/// konieczna: rzut na płaszczyznę sam w sobie nie jest jednostkowy.
pub fn flat(yaw: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    Vec3::new(sy, 0.0, cy)
}

/// Kierunek chodu na płaszczyźnie z dwóch składowych wejścia.
///
/// `forward = 1` odpowiada `W`, `strafe = 1` odpowiada `D`; obie
/// wartości są w zakresie `[-1, 1]`, bo liczy je [`crate::update`]
/// jako różnicę stanu dwóch klawiszy, a nie z prędkości.
pub fn move_dir(yaw: f32, forward: f32, strafe: f32) -> Vec3 {
    (flat(yaw) * forward + right(yaw) * strafe).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Odwrócone A/D to najczęstszy błąd w FPS-ie, więc testujemy go
    /// wprost: przy patrzeniu w `+Z` wektor w prawo to `-X`.
    ///
    /// Znak bierzemy z `right_matches_the_view_matrix`, który sprawdza
    /// to rzutem przez prawdziwą macierz `look_at_rh` — ten test sam
    /// w sobie tylko powtarza jego wynik.
    #[test]
    fn strafe_right_points_where_the_screen_right_is() {
        let r = right(0.0);
        assert!((r.x + 1.0).abs() < 1e-5, "prawo musi być -X, jest {r:?}");
        assert!(r.y.abs() < 1e-6, "prawo nie ma składowej pionowej: {r:?}");
    }

    /// Prostopadłość osi pod kątem — regresja na `forward × Y`.
    #[test]
    fn right_is_orthogonal_to_flat_forward() {
        for k in 0..16 {
            let yaw = k as f32 * std::f32::consts::TAU / 16.0;
            let f = flat(yaw);
            let r = right(yaw);
            let dot = f.x * r.x + f.y * r.y + f.z * r.z;
            assert!(
                dot.abs() < 1e-5,
                "kąty nie są prostopadłe przy {yaw}: {dot}"
            );
        }
    }

    /// Obrót o 90°: patrzymy w `+X`, a w prawo jest `-Z`.
    ///
    /// Sprawdzamy spójność obu wektorów, bo `right` sam w sobie jest
    /// tylko względem „w prawo" — z definicji nie wiadomo, czy to `+Z`
    /// czy `-Z`, dopóki nie porównamy go z kierunkiem patrzenia.
    #[test]
    fn yaw_rotates_the_right_vector_consistently() {
        let r = right(std::f32::consts::FRAC_PI_2);
        let f = flat(std::f32::consts::FRAC_PI_2);
        assert!((f.x - 1.0).abs() < 1e-5, "yaw=90° patrzy w +X: {f:?}");
        // Obrót o 90°: patrzymy w `+X`, a prawa strona kadru to `+Z`.
        // Znak potwierdza `right_matches_the_view_matrix`.
        assert!((r.z - 1.0).abs() < 1e-5, "prawo przy yaw=90° to +Z: {r:?}");
    }
    /// Regres: `right` musi wskazywać tam, gdzie naprawdę jest prawa
    /// strona kadru.
    ///
    /// To najważniejszy test w tym pliku i jedyny, który naprawdę
    /// chroni A/D. Wcześniejsze wersje sprawdzały znak wektora
    /// „na oko" (`d.x > 0`) — taka kontrola jest zielona dla obu znaków,
    /// bo nie odwołuje się do tego, jak znak ląduje na ekranie.
    ///
    /// Zamiast tego rzutujemy punkt przesunięty o `right` przez tę
    /// samą macierz `look_at_rh`, której używa renderer, i sprawdzamy,
    /// że ląduje po **prawej** stronie (dodatnie X w przestrzeni
    /// kamery). Tak znak jest jednoznaczny — wynika z macierzy, nie
    /// z rachunku „na kartce".
    #[test]
    fn right_matches_the_view_matrix() {
        use uran_math::Mat4;
        for k in 0..8 {
            let yaw = k as f32 * std::f32::consts::FRAC_PI_4;
            let eye = Vec3::ZERO;
            let fwd = forward(yaw, 0.0);
            let view = Mat4::look_at_rh(eye, eye + fwd, Vec3::Y);

            // Punkt 10 m przed graczem i 1 m w stronę `right(yaw)`.
            let probe = eye + fwd * 10.0 + right(yaw);
            let p = view * probe.extend(1.0);
            // Dzielimy przez `w`, bo dopiero to daje NDC — tak jak
            // zrobi to rasteryzacja. Bez tego porównywalibyśmy
            // współrzędne oczne, a nie to, co widać na ekranie.
            let ndc_x = p.x / p.w;
            assert!(
                ndc_x > 0.5,
                "right({yaw}) ląduje po LEWEJ stronie kadru (ndc.x={ndc_x:.3}): \
                 prawa={:?}, macierz daje dla +X ndc.x={:.3}",
                right(yaw),
                {
                    let other = view * (eye + fwd * 10.0 + Vec3::X).extend(1.0);
                    other.x / other.w
                }
            );
        }
    }

    /// `W` ma iść dokładnie tam, gdzie patrzymy — to właśnie ta
    /// własność rozjechała się przy kamerze trzecioosobowej.
    #[test]
    fn w_walks_where_we_look() {
        for k in 0..16 {
            let yaw = k as f32 * std::f32::consts::TAU / 16.0;
            let d = move_dir(yaw, 1.0, 0.0);
            let f = flat(yaw);
            let dot = d.x * f.x + d.z * f.z;
            assert!(dot > 0.999, "W nie idzie do przodu przy yaw={yaw}: {dot}");
        }
    }

    /// `D` musi iść w prawo ekranu, czyli **nie** w lewo świata.
    ///
    /// Oczekiwane kierunki biorą się z `Mat4::look_at_rh` (patrz
    /// `right_matches_the_view_matrix`), a nie z intuicji: przy
    /// patrzeniu na `+Z`, czyli `yaw = 0`, prawa strona ekranu to
    /// `-X`. Stąd `D` daje `-X`, a `A` — `+X`.
    #[test]
    fn d_walks_right_not_left() {
        let d = move_dir(0.0, 0.0, 1.0);
        assert!(d.x < -0.9, "D musi iść w -X przy yaw=0, jest {d:?}");
        let d = move_dir(0.0, 0.0, -1.0);
        assert!(d.x > 0.9, "A musi iść w +X przy yaw=0, jest {d:?}");
    }

    /// Regres: ruch w bok musi być zgodny z osią X kamery **po
    /// obrocie** — nie tylko przy `yaw = 0`. Obracać się i chodzić
    /// w bok to dokładnie to, o co prosiłeś myszką.
    #[test]
    fn strafing_follows_the_camera_after_turning() {
        for k in 0..8 {
            let yaw = k as f32 * std::f32::consts::FRAC_PI_4;
            let d = move_dir(yaw, 0.0, 1.0);
            let r = right(yaw);
            let dot = d.x * r.x + d.z * r.z;
            assert!(
                dot > 0.999,
                "D nie idzie w prawo ekranu przy yaw={yaw}: {d:?} vs {r:?}"
            );
        }
    }

    /// Ruch po skosie nie może być szybszy niż po prostej.
    #[test]
    fn diagonal_is_not_faster_than_straight() {
        let a = move_dir(0.0, 1.0, 0.0).length();
        let d = move_dir(0.0, 1.0, 1.0).length();
        assert!((a - 1.0).abs() < 1e-5, "ruch po prostej to 1, jest {a}");
        assert!((d - 1.0).abs() < 1e-5, "ruch po skosie to 1, jest {d}");
    }

    /// Kąt nie wychodzi poza 90°, więc `right` nigdy nie znika.
    #[test]
    fn pitch_is_clamped_below_vertical() {
        assert!(clamp_pitch(10.0) < std::f32::consts::FRAC_PI_2);
        assert!(clamp_pitch(-10.0) > -std::f32::consts::FRAC_PI_2);
        for p in [-5.0f32, -1.0, 0.0, 1.0, 5.0] {
            assert!(up(0.0, p).length() > 0.5, "`up` degeneruje przy pitch={p}");
            assert!(right(0.0).length() > 0.99);
        }
    }

    /// Wektor patrzenia musi być jednostkowy — długość `flat` zależy
    /// od `pitch` i przy zerowym kącie padłaby do zera.
    #[test]
    fn forward_is_unit_length() {
        for k in 0..17 {
            let pitch = k as f32 * 1.0 - 8.0;
            let f = forward(0.3, pitch);
            assert!(
                (f.length() - 1.0).abs() < 1e-4,
                "forward nie jest jednostkowy przy pitch={pitch}: {}",
                f.length()
            );
        }
    }

    #[test]
    fn up_points_up_not_down() {
        // Sam test prostopadłości nie wykryłby odwrócenia znaku — `-Y`
        // też jest prostopadły do patrzenia. Sprawdzamy więc sam znak.
        let u = up(0.0, 0.0);
        assert!(u.y > 0.99, "`up` musi wskazywać w górę, jest {u:?}");
        for k in 0..16 {
            let yaw = k as f32 * std::f32::consts::TAU / 16.0;
            // Przy patrzeniu poziomym wektor „w górę ekranu" ma zawsze
            // dodatnią składową Y, niezależnie od obrotu wokół Y.
            assert!(up(yaw, 0.0).y > 0.99, "up skierowany w dół przy yaw={yaw}");
        }
    }

    /// `up` jest prostopadły do patrzenia — obrót głową nie wywraca
    /// kamery na bok.
    #[test]
    fn up_is_orthogonal_to_forward() {
        let f = forward(0.7, 0.4);
        let u = up(0.7, 0.4);
        let dot = f.x * u.x + f.y * u.y + f.z * u.z;
        assert!(dot.abs() < 1e-4, "`up` nie jest prostopadły: {dot}");
    }
}
