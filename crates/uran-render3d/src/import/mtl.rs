//! Parser plików `.mtl` (Wavefront Material).
//!
//! Osobny plik od `obj.rs`, bo materiały mają inne reguły: są nazwane
//! blokami (`newmtl`) zamiast pozycyjnymi, a ich pliki graficzne trzeba
//! rozwiązywać względem katalogu, w którym leży `.mtl` — a nie `.obj`.
//!
//! ## Dlaczego to w ogóle robimy
//!
//! Eksport z Blendera przez `File ▸ Export ▸ Wavefront` daje parę
//! `.obj` + `.mtl` + pliki `.png`. Bez `.mtl` model jest kolorowy tylko
//! w podglądzie Blendera, a w silniku wczytuje się go jako szarą bryłę.
//!
//! ## Kwestia kolorów
//!
//! Specyfikacja Wavefront podaje `Kd` w **sRGB** (tak, jak go widzi
//! oko). Shader przelicza je na liniowy przed oświetleniem. Nie robimy
//! tu konwersji — inaczej kolor byłby konwertowany dwa razy.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Jeden materiał po sparsowaniu `.mtl`.
#[derive(Debug, Clone)]
pub struct Material {
    /// Nazwa z `newmtl` — klucz, pod którym szuka go `usemtl` w `.obj`.
    pub name: String,
    /// Kolor dyfuzyjny (`Kd`) podany w sRGB, zakres 0..1.
    pub albedo: [f32; 3],
    /// Tekstura dyfuzyjna (`map_Kd`) — ścieżka względna do katalogu `.mtl`.
    pub albedo_map: Option<PathBuf>,
    /// Mapa normalnych (`map_Bump`, `bump` lub `norm`).
    ///
    /// Blender eksportuje ją pod `map_Bump`, ale ręcznie pisane pliki
    /// często używają dwóch pozostałych nazw, więc przyjmujemy wszystkie.
    pub normal_map: Option<PathBuf>,
    /// `Ns` (wykładnik spekularny) zamieniony na chropowatość 0..1.
    ///
    /// `Ns` w `.mtl` to stara konwencja Phonga, gdzie 0 = matowe, a
    /// duże wartości = lustro. Odwrócenie skali daje `roughness`, której
    /// oczekuje model PBR.
    pub roughness: f32,
    /// Metaliczność 0..1.
    pub metallic: f32,
    /// Przezroczystość z `d` (1 = nieprzezroczyste) albo `Tr`.
    pub alpha: f32,
    /// `illum` z pliku — zostawiamy, bo niektóre eksporty traktują
    /// materiał inaczej dla `illum 2` (podświetlenie) niż dla `illum 1`.
    pub illum: i32,
}

impl Material {
    /// Materiał zastępczy dla `name`, gdy plik `.mtl` nie istnieje.
    ///
    /// To NIE jest zgadywanie wyglądu — świadomie zwracamy neutralną
    /// szarość, żeby brakujące tekstury były widoczne jako jednolita
    /// bryła, a nie jako losowe kolory.
    pub fn fallback(name: &str) -> Self {
        Self {
            name: name.to_string(),
            albedo: [0.62, 0.62, 0.64],
            albedo_map: None,
            normal_map: None,
            // 0.6 = „zwykły plastik/powłoka", ani lustro, ani guma
            roughness: 0.6,
            metallic: 0.0,
            alpha: 1.0,
            illum: 1,
        }
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::fallback("default")
    }
}

/// Wszystkie materiały z jednego pliku `.mtl`.
#[derive(Debug, Clone, Default)]
pub struct MaterialLib {
    materials: HashMap<String, Material>,
}

impl MaterialLib {
    /// Parsuje zawartość `.mtl`.
    ///
    /// `base_dir` to katalog pliku `.mtl` — wszystkie ścieżki w treści
    /// (`map_Kd ...`) rozwiązujemy względem niego. Bez tego import
    /// „działałby" tylko, gdyby katalogi szczęśliwie się zgadzały.
    pub fn parse(text: &str, base_dir: impl AsRef<Path>) -> Self {
        let base_dir = base_dir.as_ref();
        // `Path::new(".").join("a.png")` daje `./a.png`, a w testach i przy
        // ścieżkach względnych oczekujemy po prostu `a.png`.
        let base_dir = if base_dir.as_os_str().is_empty() || base_dir == Path::new(".") {
            Path::new("")
        } else {
            base_dir
        };
        let mut lib = Self::default();
        let mut cur: Option<Material> = None;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            let Some(key) = it.next() else { continue };
            let rest: Vec<&str> = it.collect();

            match key {
                "newmtl" => {
                    // nowe `newmtl` domyka poprzedni materiał
                    if let Some(m) = cur.take() {
                        lib.materials.insert(m.name.clone(), m);
                    }
                    let name = rest.join(" ");
                    if name.is_empty() {
                        continue;
                    }
                    cur = Some(Material::fallback(&name));
                }
                "Kd" => {
                    if let (Some(m), Some(c)) = (cur.as_mut(), parse_rgb(&rest)) {
                        m.albedo = c;
                    }
                }
                "map_Kd" | "map_kd" => {
                    if let (Some(m), Some(p)) = (cur.as_mut(), texture_path(&rest)) {
                        m.albedo_map = Some(base_dir.join(p));
                    }
                }
                "map_Bump" | "bump" | "norm" | "map_Kn" => {
                    if let (Some(m), Some(p)) = (cur.as_mut(), texture_path(&rest)) {
                        m.normal_map = Some(base_dir.join(p));
                    }
                }
                // Wykładnik Phonga -> chropowatość. Logika w `ns_to_roughness`.
                "Ns" => {
                    if let (Some(m), Some(v)) = (cur.as_mut(), first_f32(&rest)) {
                        m.roughness = ns_to_roughness(v);
                    }
                }
                "Pm" => {
                    if let (Some(m), Some(v)) = (cur.as_mut(), first_f32(&rest)) {
                        m.metallic = v.clamp(0.0, 1.0);
                    }
                }
                "d" => {
                    if let (Some(m), Some(v)) = (cur.as_mut(), first_f32(&rest)) {
                        m.alpha = v.clamp(0.0, 1.0);
                    }
                }
                // `Tr` to odwrotność `d` — w starszych plikach to jedyne,
                // co opisuje przezroczystość.
                "Tr" => {
                    if let (Some(m), Some(v)) = (cur.as_mut(), first_f32(&rest)) {
                        m.alpha = (1.0 - v).clamp(0.0, 1.0);
                    }
                }
                "illum" => {
                    if let (Some(m), Some(v)) = (cur.as_mut(), first_f32(&rest)) {
                        m.illum = v as i32;
                    }
                }
                _ => {}
            }
        }

        if let Some(m) = cur.take() {
            lib.materials.insert(m.name.clone(), m);
        }
        lib
    }

    /// Materiał o podanej nazwie albo [`Material::fallback`].
    ///
    /// Brak wpisu to normalna sytuacja, nie błąd: model bywa eksportowany
    /// bez `.mtl`, a `usemtl` i tak odwołuje się do nazw, których nikt
    /// nie zdefiniował. Dzięki temu import się nie wywala, tylko dostaje
    /// neutralny materiał.
    pub fn get(&self, name: &str) -> Material {
        self.materials
            .get(name)
            .cloned()
            .unwrap_or_else(|| Material::fallback(name))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.materials.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

/// `Ns` (Phong) -> chropowatość 0..1.
///
/// Wykładnik 1000 oznacza lustro (roughness ~0), a `Ns 5` — mat powłoki.
/// Mapujemy logarytmicznie, bo skala Phonga jest wykładnicza: różnica
/// między 10 a 100 jest o wiele bardziej zauważalna niż 100 a 200.
fn ns_to_roughness(ns: f32) -> f32 {
    let ns = ns.max(1.0);
    // 1000 -> ~0.0, 1 -> ~1.0
    let r = 1.0 - (ns.ln() / 1000f32.ln()).clamp(0.0, 1.0);
    r.clamp(0.05, 1.0)
}

/// Trzy liczby po `Kd` (albo jedna liczba = szarość).
fn parse_rgb(rest: &[&str]) -> Option<[f32; 3]> {
    if rest.is_empty() {
        return None;
    }
    let v: Vec<f32> = rest.iter().filter_map(|s| s.parse::<f32>().ok()).collect();
    match v.len() {
        1 => Some([v[0], v[0], v[0]]),
        3.. => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

fn first_f32(rest: &[&str]) -> Option<f32> {
    rest.first()?.parse::<f32>().ok()
}

/// Ścieżka tekstury z dyrektywy `map_*`.
///
/// Linia bywa `map_Kd kolor.png -s 1 1 1 1` (skala UV) albo
/// `map_Kd -o 0 0 0 kolor.png` (przesunięcie), a opcje stoją w obu
/// kierunkach. Nazwa pliku jest jedynym tokenem, który NIE zaczyna się
/// od `-` i NIE jest liczbą — bierzemy właśnie ten.
///
/// Sam „ostatni token" bywa zły: dla `base.png -s 1 1 1 1` dawałby `1`.
fn texture_path(rest: &[&str]) -> Option<PathBuf> {
    rest.iter()
        .rev()
        .find(|tok| !tok.starts_with('-') && tok.parse::<f32>().is_err())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_material() {
        let lib = MaterialLib::parse(
            "newmtl paint\nKd 0.8 0.1 0.1\nNs 250\nd 1.0\nillum 2\n",
            ".",
        );
        assert_eq!(lib.len(), 1);
        let m = lib.get("paint");
        assert!((m.albedo[0] - 0.8).abs() < 1e-6);
        assert!((m.albedo[2] - 0.1).abs() < 1e-6);
        assert_eq!(m.illum, 2);
    }

    #[test]
    fn ns_maps_to_sensible_roughness() {
        // lustro -> niska chropowatość, mat -> wysoka
        assert!(ns_to_roughness(1000.0) < 0.1);
        // Ns 4 to bardzo matowa powłoka: skala logarytmiczna daje ~0.80
        assert!(ns_to_roughness(4.0) > 0.75);
        // monotoniczność: większe Ns = gładsza powierzchnia
        assert!(ns_to_roughness(900.0) < ns_to_roughness(100.0));
    }

    #[test]
    fn texture_path_survives_uv_options() {
        // opcje przed nazwą i po nazwie — bierzemy ostatni token
        let a = texture_path(&["-o", "0", "0", "0", "base.png"]).unwrap();
        assert_eq!(a, PathBuf::from("base.png"));
        let b = texture_path(&["base.png", "-s", "1", "1", "1", "1"]).unwrap();
        assert_eq!(b, PathBuf::from("base.png"));
    }

    #[test]
    fn normal_map_accepts_blender_spelling() {
        for key in ["map_Bump", "bump", "norm"] {
            let lib = MaterialLib::parse(&format!("newmtl m\n{key} n.png\n"), ".");
            assert_eq!(
                lib.get("m").normal_map,
                Some(PathBuf::from("n.png")),
                "{key}"
            );
        }
    }

    #[test]
    fn transparency_reads_both_d_and_tr() {
        let a = MaterialLib::parse("newmtl a\nd 0.25\n", ".");
        assert!((a.get("a").alpha - 0.25).abs() < 1e-6);
        // Tr jest odwrotnością d
        let b = MaterialLib::parse("newmtl b\nTr 0.75\n", ".");
        assert!((b.get("b").alpha - 0.25).abs() < 1e-6);
    }

    #[test]
    fn unknown_material_gets_neutral_fallback() {
        // `usemtl` może wskazywać nazwę, której nikt nie zdefiniował
        let lib = MaterialLib::parse("newmtl known\nKd 1 0 0\n", ".");
        let m = lib.get("unknown");
        assert!(!lib.contains("unknown"));
        assert!((m.albedo[0] - m.albedo[1]).abs() < 1e-6, "zapasowy to szarość");
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let lib = MaterialLib::parse("# komentarz\n\nnewmtl m\n  # w środku\nKd 0.5\n", ".");
        assert_eq!(lib.len(), 1);
    }

    #[test]
    fn texture_paths_resolve_against_mtl_dir() {
        let lib = MaterialLib::parse("newmtl m\nmap_Kd tex/wood.png\n", "/models/bike");
        assert_eq!(
            lib.get("m").albedo_map,
            Some(PathBuf::from("/models/bike/tex/wood.png"))
        );
    }
}
