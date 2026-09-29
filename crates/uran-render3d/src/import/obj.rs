//! Parser plików `.obj` (Wavefront) + `.mtl` do gotowych siatek silnika.
//!
//! ## Co tu robimy
//!
//! 1. Czytamy `v` / `vn` / `vt` / `f` / `usemtl` / `o` / `mtllib`.
//! 2. Rozwijamy wielokąty na trójkąty (wianek) i rozdzielamy indeksy,
//!    bo `f 1/5/9 2/6/10` to trzy różne pary (pozycja, UV, normalna).
//! 3. Zamieniamy osie Z-up (Blender) na Y-up (silnik).
//! 4. Normalizujemy skalę i stawiamy model na `y = 0`.

use std::path::Path;

use uran_math::Vec3;

use super::mtl::{Material, MaterialLib};
use crate::mesh::{Mesh, Vertex};

/// Referencja na wierzchołek: `pozycja/uv/normalna` z 1-based indeksów.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceRef {
    pub pos: usize,
    pub uv: Option<usize>,
    pub nrm: Option<usize>,
}

impl FaceRef {
    /// Parsuje token `1/2/3`, `1//3`, `1/2` lub `1`.
    ///
    /// Ujemne indeksy są względne do KOŃCA listy (`-1` = ostatni), więc
    /// nie wolno ich liczyć od zera. Zwracamy indeks 0-based.
    fn parse(tok: &str, npos: usize, nuv: usize, nnrm: usize) -> Result<Self, String> {
        let mut it = tok.split('/');
        let pos = it.next().unwrap_or("").trim();
        let uv = it.next().map(str::trim).filter(|s| !s.is_empty());
        let nrm = it.next().map(str::trim).filter(|s| !s.is_empty());

        if pos.is_empty() {
            return Err(format!("pusta referencja wierzchołka: {tok}"));
        }
        let pos = parse_index(pos, npos)?;
        let uv = match uv {
            Some(v) => Some(parse_index(v, nuv).map_err(|e| format!("uv: {e}"))?),
            None => None,
        };
        let nrm = match nrm {
            Some(v) => Some(parse_index(v, nnrm).map_err(|e| format!("normalna: {e}"))?),
            None => None,
        };
        Ok(Self { pos, uv, nrm })
    }
}

/// 1-based (lub ujemny od końca) indeks na 0-based.
fn parse_index(tok: &str, len: usize) -> Result<usize, String> {
    let raw: i64 = tok.parse().map_err(|_| format!("zły indeks: {tok}"))?;
    let idx = if raw < 0 { len as i64 + raw } else { raw - 1 };
    if idx < 0 || idx as usize >= len {
        return Err(format!("indeks {tok} poza zakresem 1..={len}"));
    }
    Ok(idx as usize)
}

/// Wynik parsowania tekstu `.obj`, jeszcze bez siatek.
#[derive(Debug, Clone, Default)]
pub struct ObjScene {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Trójkąty po rozwinięciu wielokątów.
    ///
    /// Trzymamy pełne [`FaceRef`], a nie sam indeks pozycji: w `.obj`
    /// te trzy listy są niezależne i ten sam wierzchołek często ma
    /// inne UV niż sąsiad.
    pub triangles: Vec<[FaceRef; 3]>,
    /// Materiał każdego trójkąta.
    pub tri_material: Vec<String>,
    /// Nazwy obiektów z `o`/`g`, równoległe do `tri_object`.
    pub objects: Vec<String>,
    /// Indeks obiektu każdego trójkąta.
    pub tri_object: Vec<usize>,
    /// Nazwa pliku `.mtl` z `mtllib`.
    pub mtllib: Option<String>,
}

/// Błąd parsowania z numerem linii — bez niego „OBJ nie działa" jest
/// bezużyteczne przy trzymilionowym pliku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "linia {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parsuje tekst `.obj` do [`ObjScene`].
pub fn parse_obj(text: &str) -> Result<ObjScene, Box<dyn std::error::Error>> {
    let mut scene = ObjScene::default();
    let mut cur_material = String::new();
    let mut cur_object = 0usize;
    scene.objects.push(String::new());

    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let rest: Vec<&str> = it.collect();
        let err = |m: String| ParseError { line: line_no, message: m };

        match key {
            "v" => {
                let p = parse_vec3(&rest).ok_or_else(|| err("zły wiersz v".into()))?;
                scene.positions.push(p);
            }
            "vn" => {
                let p = parse_vec3(&rest).ok_or_else(|| err("zły wiersz vn".into()))?;
                scene.normals.push(p);
            }
            "vt" => {
                // `vt u v` albo `vt u v w` (w ignorujemy — mamy tylko 2D)
                let u = rest.first().and_then(|s| s.parse::<f32>().ok());
                let v = rest.get(1).and_then(|s| s.parse::<f32>().ok());
                match (u, v) {
                    (Some(u), Some(v)) => scene.uvs.push([u, v]),
                    (Some(u), None) => scene.uvs.push([u, 0.0]),
                    _ => return Err(Box::new(err("zły wiersz vt".into()))),
                }
            }
            "f" => {
                if rest.len() < 3 {
                    return Err(Box::new(err(format!(
                        "wielokąt ma {} wierzchołków", rest.len()
                    ))));
                }
                let mut face = Vec::with_capacity(rest.len());
                for tok in &rest {
                    // `?` na `String` zamieniłby błąd w gołą treść i
                    // zgubił numer linii — dlatego pakujemy go tu w `ParseError`
                    let r = FaceRef::parse(
                        tok,
                        scene.positions.len(),
                        scene.uvs.len(),
                        scene.normals.len(),
                    );
                    face.push(r.map_err(|e| err(format!("wiersz twarzy: {e}")))?);
                }
                push_face(&mut scene, &face, &cur_material, cur_object);
            }
            "usemtl" => cur_material = rest.join(" "),
            "o" | "g" => {
                // Przy pustej nazwie używamy materiału — dzięki temu
                // lista części nigdy nie zawiera pustego obiektu.
                let name = rest.join(" ");
                let name = if name.is_empty() { cur_material.clone() } else { name };
                cur_object = scene.objects.len();
                scene.objects.push(name);
            }
            // W pliku z Blendera bywa „junak m10 .mtl" ze spacją w nazwie
            "mtllib" => scene.mtllib = Some(rest.join(" ")),
            // `s`, `vp`, `l` — pomijamy: normalne dostarcza sam plik
            _ => {}
        }
    }
    Ok(scene)
}

/// Rozwijamy wielokąt na trójkąty (wianek) i pilnujemy orientacji.
///
/// Wianek `(0,i,i+1)` wystarcza dla kwadratów i większości ngonów
/// Blendera. Tam, gdzie plik DOSTARCZA normalne, dodatkowo sprawdzamy
/// zgodność iloczynu wektorowego z zapisaną normalną i ewentualnie
/// zamieniamy dwa wierzchołki. Bez tego przy wklęsłych wielokątach
/// cześć ścian renderowałaby się do środka i znikała.
fn push_face(scene: &mut ObjScene, face: &[FaceRef], material: &str, object: usize) {
    if face.len() < 3 {
        return;
    }
    for k in 1..face.len() - 1 {
        let a = face[0];
        let b = face[k];
        let c = face[k + 1];
        let flip = match (a.nrm, b.nrm, c.nrm) {
            (Some(na), Some(_), Some(_)) => {
                let geo = face_normal(
                    scene.positions[a.pos],
                    scene.positions[b.pos],
                    scene.positions[c.pos],
                );
                let s = scene.normals[na];
                geo.dot(Vec3::new(s[0], s[1], s[2])) < 0.0
            }
            // bez normalnych nie mamy czym zweryfikować — ufamy plikowi
            _ => false,
        };
        let (b, c) = if flip { (c, b) } else { (b, c) };
        scene.triangles.push([a, b, c]);
        scene.tri_material.push(material.to_string());
        scene.tri_object.push(object);
    }
}

fn parse_vec3(rest: &[&str]) -> Option<[f32; 3]> {
    if rest.len() < 3 {
        return None;
    }
    let x: f32 = rest[0].parse().ok()?;
    let y: f32 = rest[1].parse().ok()?;
    let z: f32 = rest[2].parse().ok()?;
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return None;
    }
    Some([x, y, z])
}

/// Iloczyn wektorowy × wektorowy (normalna ściany, przed normalizacją).
fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Vec3 {
    let u = Vec3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let v = Vec3::new(c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    u.cross(v)
}

/// Zamienia Z-up (Blender) na Y-up (silnik).
///
/// Blender: X = wzdłuż modelu, Y = szerokość, Z = wysokość.
/// Silnik: X = wzdłuż, Y = wysokość, Z = szerokość.
///
/// Mapowanie `(x, y, z) -> (x, z, y)` zamienia oś Y na Z, co ODWRACA
/// układ (det = -1) i odwróciłoby wszystkie twarze. Dlatego jedną
/// składową zostawiamy z minusem: `(x, z, -y)` to obrót o -90° wokół X,
/// czyli właściwa rotacja — twarze pozostają widoczne od zewnątrz.
fn blender_to_engine(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[2], -p[1])
}

/// Jedna część modelu: geometria + nazwa + materiał.
#[derive(Debug, Clone)]
pub struct ModelPart {
    /// Nazwa obiektu z `o` (np. `wheel_f`), albo materiał, gdy `o` nie było.
    pub name: String,
    pub mesh: Mesh,
    pub material: Material,
}

/// Cały wczytany model.
#[derive(Debug, Clone, Default)]
pub struct ImportedModel {
    pub parts: Vec<ModelPart>,
    /// Środek bryły przed normalizacją (do diagnostyki i skalowania).
    pub raw_center: Vec3,
    /// Wymiary modelu w jednostkach pliku, PRZED skalowaniem.
    pub raw_size: Vec3,
}

/// Buduje siatki z [`ObjScene`] i przypisuje materiały.
///
/// Nie optymalizujemy wspólnych wierzchołków między ścianami: pliki
/// z Blendera i tak rzadko je dzielą, a scalanie po pozycji psułoby
/// mapowanie UV, które akurat w tych modelach jest istotne.
fn build(scene: ObjScene, lib: &MaterialLib) -> ImportedModel {
    // Tylko obiekty, które faktycznie mają trójkąty.
    let mut order: Vec<usize> = (0..scene.objects.len()).collect();
    order.retain(|&o| scene.tri_object.contains(&o));

    let mut parts = Vec::with_capacity(order.len());
    for obj in order {
        let mut mesh = Mesh::new();
        let mut part_material = String::new();

        for (ti, tri) in scene.triangles.iter().enumerate() {
            if scene.tri_object[ti] != obj {
                continue;
            }
            if part_material.is_empty() {
                part_material = scene.tri_material[ti].clone();
            }
            let material = lib.get(&scene.tri_material[ti]);
            // Normalna z geometrii — awaryjnie, gdy plik nie ma `vn`.
            let gn = face_normal(
                scene.positions[tri[0].pos],
                scene.positions[tri[1].pos],
                scene.positions[tri[2].pos],
            )
            .normalize_or_zero();

            for corner in tri {
                let p = blender_to_engine(scene.positions[corner.pos]);
                let n = corner
                    .nrm
                    .map(|i| {
                        let r = scene.normals[i];
                        Vec3::new(r[0], r[1], r[2])
                    })
                    .unwrap_or(gn);
                let uv = corner.uv.map(|i| scene.uvs[i]).unwrap_or([0.0, 0.0]);
                // Kd jako kolor wierzchołka: gdy jest tekstura, shader
                // zdominuje ją i tak; gdy jej nie ma, dostajemy kolor
                // materiału zamiast bieli.
                mesh.push(Vertex::new_uv(p, n, uv, material.albedo));
            }
            let i = mesh.vertices.len() as u32 - 3;
            mesh.indices.extend_from_slice(&[i, i + 1, i + 2]);
        }

        if mesh.vertices.is_empty() {
            continue;
        }
        let material = lib.get(&part_material);
        let obj_name = scene.objects[obj].clone();
        parts.push(ModelPart {
            name: if obj_name.is_empty() {
                material.name.clone()
            } else {
                obj_name
            },
            mesh,
            material,
        });
    }

    // Środek i rozmiar surowej geometrii
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in &scene.positions {
        let p = Vec3::new(p[0], p[1], p[2]);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if lo.x == f32::INFINITY {
        lo = Vec3::ZERO;
        hi = Vec3::ZERO;
    }

    ImportedModel {
        parts,
        raw_center: (lo + hi) * 0.5,
        raw_size: hi - lo,
    }
}

impl ImportedModel {
    /// Aktualny rozmiar modelu (po normalizacji).
    pub fn bounds_size(&self) -> Vec3 {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for part in &self.parts {
            for v in &part.mesh.vertices {
                lo = lo.min(v.pos());
                hi = hi.max(v.pos());
            }
        }
        if lo.x == f32::INFINITY {
            return Vec3::ZERO;
        }
        hi - lo
    }

    /// Przesuwa model tak, by stał na `y = 0` i był wyśrodkowany w XZ.
    ///
    /// Dzięki temu siatka wstawiona do sceny ma sensowne początki bez
    /// wiedzy o tym, gdzie w pliku leżał oryginał.
    pub fn normalize_to_ground(&mut self) {
        let size = self.bounds_size();
        if size.x <= 0.0 && size.y <= 0.0 {
            return;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for part in &self.parts {
            for v in &part.mesh.vertices {
                lo = lo.min(v.pos());
                hi = hi.max(v.pos());
            }
        }
        if lo.x == f32::INFINITY {
            return;
        }
        let offset = Vec3::new(-(lo.x + hi.x) * 0.5, -lo.y, -(lo.z + hi.z) * 0.5);
        for part in &mut self.parts {
            for v in &mut part.mesh.vertices {
                v.position = (v.pos() + offset).to_array();
            }
        }
    }

    /// Skaluje model do zadanej wysokości (w jednostkach świata).
    ///
    /// Modele z Blendera bywają w centymetrach albo w dowolnej skali
    /// eksportu, a scena ma metry. Wysokość jest tu wybrana, bo motocykl
    /// rozpoznajemy po wysokości — po długości różnią się motocykle
    /// i samochody.
    pub fn scale_to_height(&mut self, height: f32) {
        let size = self.bounds_size();
        if size.y <= 1e-6 || !size.y.is_finite() || height <= 0.0 {
            return;
        }
        let k = height / size.y;
        for part in &mut self.parts {
            for v in &mut part.mesh.vertices {
                // Skala jednorodna nie zmienia kierunku, więc normalnych
                // nie ruszamy — wystarczy je znormalizować w shaderze.
                v.position = (v.pos() * k).to_array();
            }
        }
    }

    /// Łączy części o tym samym materiale (mniej draw calli).
    pub fn merge_by_material(&mut self) {
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<String, Mesh> = BTreeMap::new();
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        for part in &self.parts {
            let e = groups.entry(part.material.name.clone()).or_default();
            let base = e.vertices.len() as u32;
            e.vertices.extend_from_slice(&part.mesh.vertices);
            for i in &part.mesh.indices {
                e.indices.push(base + i);
            }
            names
                .entry(part.material.name.clone())
                .or_insert_with(|| part.name.clone());
        }
        let mut out = Vec::new();
        for (mat, mesh) in groups {
            if mesh.vertices.is_empty() {
                continue;
            }
            out.push(ModelPart {
                name: names.get(&mat).cloned().unwrap_or_else(|| mat.clone()),
                material: Material::fallback(&mat),
                mesh,
            });
        }
        self.parts = out;
    }

    /// Łączy WSZYSTKIE części w jedną siatkę (jeden materiał).
    pub fn merge_all(&mut self) {
        let mut merged = Mesh::new();
        for part in &self.parts {
            let base = merged.vertices.len() as u32;
            merged.vertices.extend_from_slice(&part.mesh.vertices);
            for i in &part.mesh.indices {
                merged.indices.push(base + i);
            }
        }
        let material = self.parts.first().map(|p| p.material.clone()).unwrap_or_default();
        self.parts = vec![ModelPart { name: "merged".into(), mesh: merged, material }];
    }
}

/// Wczytuje `.obj` wraz z `.mtl`.
///
/// `path` wskazuje na plik `.obj`. Katalog `.mtl` bierzemy z dyrektywy
/// `mtllib`, a jeśli go nie ma — z katalogu samego `.obj`. Brakujący
/// `.mtl` to nie błąd: model dostanie materiały zastępcze i wciąż
/// się załaduje.
pub fn load_from_file(path: impl AsRef<Path>) -> Result<ImportedModel, Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let scene = parse_obj(&text)?;

    let dir = path.parent().unwrap_or(Path::new("."));
    let mtl_path = scene
        .mtllib
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|n| dir.join(n))
        .unwrap_or_else(|| path.with_extension("mtl"));

    let lib = match std::fs::read_to_string(&mtl_path) {
        Ok(t) => MaterialLib::parse(&t, mtl_path.parent().unwrap_or(dir)),
        Err(_) => MaterialLib::default(),
    };

    let mut model = build(scene, &lib);
    model.normalize_to_ground();
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE: &str = "\
# test
mtllib test.mtl
o body
v -1 -1 0
v  1 -1 0
v  1  1 0
v -1  1 0
vn 0 0 -1
vt 0 0
vt 1 0
vt 1 1
vt 0 1
usemtl skin
f 1/1/1 2/2/1 3/3/1 4/4/1
";

    #[test]
    fn parses_cube_with_quads_and_uvs() {
        let s = parse_obj(CUBE).unwrap();
        assert_eq!(s.positions.len(), 4);
        assert_eq!(s.uvs.len(), 4);
        assert_eq!(s.normals.len(), 1);
        // kwadrat -> 2 trójkąty
        assert_eq!(s.triangles.len(), 2);
        assert_eq!(s.mtllib.as_deref(), Some("test.mtl"));
        assert_eq!(s.tri_material[0], "skin");
    }

    #[test]
    fn handles_negative_indices() {
        // -1 = ostatni wierzchołek, -2 = przedostatni
        let s = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n").unwrap();
        assert_eq!(s.triangles[0][0].pos, 0);
        assert_eq!(s.triangles[0][1].pos, 1);
        assert_eq!(s.triangles[0][2].pos, 2);
    }

    #[test]
    fn splits_objects_on_o_directive() {
        let s = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\no a\nf 1 2 3\no b\nf 1 2 3\n").unwrap();
        assert_eq!(s.objects.len(), 3, "pusty początek + a + b");
        assert_ne!(s.tri_object[0], s.tri_object[1]);
    }

    #[test]
    fn blender_zup_becomes_engine_yup() {
        // (1, 2, 3) w Blenderze: x=1, y=2 (szerokość), z=3 (wysokość)
        let p = blender_to_engine([1.0, 2.0, 3.0]);
        assert!((p.x - 1.0).abs() < 1e-6, "oś X bez zmian");
        assert!((p.y - 3.0).abs() < 1e-6, "Z Blendera staje się Y silnika");
        assert!((p.z - (-2.0)).abs() < 1e-6, "Y Blendera staje się -Z");
    }

    #[test]
    fn axis_conversion_preserves_winding() {
        // Konwersja osi MUSI być właściwą rotacją (det = +1), inaczej
        // cały model renderowałby się do środka — odwrócone culling
        // zamienia widoczne ściany w niewidoczne.
        //
        // Nie porównujemy „normalnej przed" z „normalną po": to jest
        // OBRÓT, więc wektor normalnej też się zmienia i musiałby
        // przejść tę samą transformację. Sprawdzamy więc niezmiennik
        // samej transformacji: ortonormalność i znak wyznacznika.
        let bx = Vec3::X;
        let by = Vec3::Y;
        let bz = Vec3::Z;
        let ex = blender_to_engine(bx.to_array());
        let ey = blender_to_engine(by.to_array());
        let ez = blender_to_engine(bz.to_array());

        // obraz musi być ortonormalny (to obrót, nie deformacja)
        for (a, b) in [(ex, ey), (ex, ez), (ey, ez)] {
            assert!((a.dot(b)).abs() < 1e-6, "wektory nie są prostopadłe");
            assert!((a.length() - 1.0).abs() < 1e-6, "wektor nie jest jednostkowy");
        }
        // wyznacznik = ex · (ey × ez) — dodatni dla właściwej rotacji
        let det = ex.dot(ey.cross(ez));
        assert!((det - 1.0).abs() < 1e-6, "wyznacznik {det} — układ odwrócony");
    }

    #[test]
    fn normalize_puts_model_on_the_ground() {
        let mut m = build(parse_obj(CUBE).unwrap(), &MaterialLib::default());
        m.normalize_to_ground();
        let min_y = m
            .parts
            .iter()
            .flat_map(|p| p.mesh.vertices.iter())
            .map(|v| v.pos().y)
            .fold(f32::INFINITY, f32::min);
        assert!(min_y.abs() < 1e-5, "model stoi na podłodze, a nie w {min_y}");
    }

    #[test]
    fn scale_to_height_gives_requested_size() {
        // Prawdziwy prostopadłościan, a nie płaska kwadratowa ściana:
        // `scale_to_height` skaluje po wysokości, więc bryła bez
        // zróżnicowania w osi Y jest tu bezużyteczna. Musi też mieć
        // ŚCIANY — `build` kopiuje tylko wierzchołki użyte przez trójkąty.
        let src = "\
v -1 -1 0
v  1 -1 0
v  1  1 0
v -1  1 0
v -1 -1 2
v  1 -1 2
v  1  1 2
v -1  1 2
f 1 2 3
f 1 3 4
f 5 6 7
f 5 7 8
f 1 2 6
f 1 6 5
f 2 3 7
f 2 7 6
f 3 4 8
f 3 8 7
f 4 1 5
f 4 5 8
";
        let mut m = build(parse_obj(src).unwrap(), &MaterialLib::default());
        assert!((m.bounds_size().y - 2.0).abs() < 1e-5, "w pliku wysokość 2");
        m.scale_to_height(1.2);
        let size = m.bounds_size();
        // po konwersji osi wysokość Blendera (Z) staje się Y silnika
        assert!((size.y - 1.2).abs() < 1e-4, "wysoka {size:?}");
    }

    #[test]
    fn merge_by_material_reduces_part_count() {
        let src = "\
v 0 0 0
v 1 0 0
v 0 1 0
o a
usemtl red
f 1 2 3
o b
usemtl red
f 1 2 3
o c
usemtl blue
f 1 2 3
";
        let mut m = build(parse_obj(src).unwrap(), &MaterialLib::default());
        assert_eq!(m.parts.len(), 3);
        m.merge_by_material();
        assert_eq!(m.parts.len(), 2, "dwa materiały zamiast trzech obiektów");
        let total: usize = m.parts.iter().map(|p| p.mesh.indices.len()).sum();
        assert_eq!(total, 9, "9 indeksów = 3 trójkąty");
    }

    #[test]
    fn merge_all_keeps_all_triangles() {
        let mut m = build(parse_obj(CUBE).unwrap(), &MaterialLib::default());
        m.merge_all();
        assert_eq!(m.parts.len(), 1);
        assert_eq!(m.parts[0].mesh.indices.len(), 6);
    }

    #[test]
    fn reports_line_number_on_bad_index() {
        let err = parse_obj("v 0 0 0\nf 1 2 99\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("linia 2"), "brak numeru linii w: {msg}");
    }

    #[test]
    fn rejects_degenerate_face() {
        // mniej niż 3 wierzchołki to nie trójkąt
        assert!(parse_obj("v 0 0 0\nv 1 0 0\nf 1 2\n").is_err());
    }

    #[test]
    fn keeps_material_names_with_spaces() {
        // nazwy materiałów z polskimi znakami i spacjami bywają w plikach
        // z Blendera; `join(" ")` musi je odtworzyć w całości
        let s = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl osłona\nf 1 2 3\n").unwrap();
        assert_eq!(s.tri_material[0], "osłona");
    }

    /// Test integracyjny na modelu junaka prosto z Blendera.
    ///
    /// Ładujemy prawdziwy plik z repo, bo parser syntaktycznie poprawny
    /// wciąż potrafi zgubić UV albo odwrócić model — a to widać dopiero
    /// na geometrii, nie w testach na stringach.
    ///
    /// Test pomijamy, gdy pliku nie ma (np. kopia bez zasobów).
    #[test]
    fn loads_real_blender_motorcycle() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../junak-rider/assets/junak m10.obj");
        let Ok(mut model) = load_from_file(&path) else {
            eprintln!("pomijam: brak {}", path.display());
            return;
        };
        model.scale_to_height(1.2);

        let verts: usize = model.parts.iter().map(|p| p.mesh.vertices.len()).sum();
        let idx: usize = model.parts.iter().map(|p| p.mesh.indices.len()).sum();
        let tris = idx / 3;
        eprintln!(
            "junak: {} części, {verts} wierzchołków, {tris} trójkątów, rozmiar {:?}",
            model.parts.len(),
            model.bounds_size()
        );

        assert!(model.parts.len() > 5, "motocykl ma wiele obiektów");
        assert!(tris > 10_000, "zachowaliśmy geometrię: {tris} trójkątów");
        // Import NIE scala wierzchołków (patrz `build`), więc na trójkąt
        // przypada dokładnie 3 wierzchołki. To świadomy koszt: 193 tys.
        // wierzchołków × 44 B ≈ 8,5 MB, ale za to UV są dokładne.
        assert_eq!(verts, tris * 3, "wierzchołki i trójkąty się nie zgadzają");

        // UV muszą dojechać do wierzchołków — bez tego model wczytuje
        // się, ale jest nieoteksturowany mimo pliku z Blendera
        let with_uv = model
            .parts
            .iter()
            .flat_map(|p| p.mesh.vertices.iter())
            .filter(|v| v.has_uv())
            .count();
        assert!(with_uv > 1000, "UV nie dotarły: tylko {with_uv} wierzchołków");

        // po skalowaniu motocykl ma sensowne wymiary w metrach
        let size = model.bounds_size();
        assert!(size.y > 0.5 && size.y < 3.0, "wysokość {size:?} to nie motocykl");
        assert!(size.x > 1.0 && size.x < 4.0, "długość {size:?} to nie motocykl");
    }
}
