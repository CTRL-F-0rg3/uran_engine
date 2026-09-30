//! Import modeli glTF 2.0 (`.glb`) ze skinningiem i materiałami.
//!
//! ## Po co to, skoro jest `.obj`
//!
//! `.obj` obsługuje statyczne bryły. Postać to co innego: ma szkielet
//! (macierze kości), wagi skinningu na wierzchołkach i klipy animacji.
//! Wavefront tego nie wyraża, a większość postaci z Blendera ma to
//! właśnie w glTF/FBX.
//!
//! ## Zakres — świadomie wąski
//!
//! | Obszar | Stan |
//! |--------|------|
//! | `.glb` (JSON + BIN w jednym pliku) | ✅ |
//! | `.gltf` + osobne pliki | ❌ nieobsługiwane |
//! | skinning (`JOINTS_0`/`WEIGHTS_0`) | ✅ do 4 wpływów |
//! | animacje z pliku | ❌ (klipy czytamy z `.urananm`) |
//! | `KHR_draco_mesh_compression` | ❌ |
//! | morph targete | ❌ |
//!
//! Nieobsługiwane rzeczy warto znać, bo plik może je zawierać, a my
//! po cichu je pomijamy: [`GltfScene::unsupported`] wymienia je po nazwie.
//!
//! ## Oś
//!
//! glTF jest Y-up, tak jak silnik — **nie przenosimy osi**. To różni ten
//! loader od `import::obj`, który przeskakuje z Z-up Blendera na Y-up.
//! Dzięki temu pozycje ze skinningiem, macierze kości i UV pasują do
//! pliku bajt w bajt i nie trzeba ich nigdzie obracać.

use std::collections::BTreeMap;
use std::fmt;

use uran_math::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

use super::json::{self, Json};

/// Błąd importu z kontekstem (który element, który plik).
#[derive(Debug)]
pub struct GltfError {
    pub msg: String,
}

impl fmt::Display for GltfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "glTF: {}", self.msg)
    }
}

impl std::error::Error for GltfError {}

fn err<T>(msg: impl Into<String>) -> Result<T, GltfError> {
    Err(GltfError { msg: msg.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Buduje minimalny `.glb` z podanym JSON-em i (opcjonalnie) BIN-em.
    ///
    /// Testy na prawdziwym pliku 16 MB byłyby niepraktyczne, a błąd
    /// wczytywania kontenera to dokładnie ten sam kod, co przy realnym
    /// pliku — więc testujemy go na syntetycznych bajtach.
    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        // Chunki GLB muszą mieć długość wielokrotnością 4, ale **offset
        // w JSON-ie liczymy względem początku danych, nie ogonka** —
        // więc `bin` zostawiamy nietknięty, a dopełnienie dopisujemy
        // dopiero w nagłówku. Inaczej `bin[32..36]` w teście wskazywałoby
        // nie na to, co loader zobaczy.
        let js_len = json.len().div_ceil(4) * 4;
        let bin_len = bin.len().div_ceil(4) * 4;
        let total = 12 + 8 + js_len + 8 + bin_len;
        let mut out = Vec::new();
        out.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(js_len as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(json.as_bytes());
        out.resize(12 + 8 + js_len, b' ');
        out.extend_from_slice(&(bin_len as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(bin);
        out.resize(total, 0);
        out
    }

    /// Scene z jedną kością, jednym wierzchołkiem i jednym trójkątem.
    const MINIMAL: &str = r#"{
      "scene":0,
      "scenes":[{"nodes":[0,2]}],
      "nodes":[
        {"name":"Root","children":[1]},
        {"name":"Bone","translation":[0,1,0]},
        {"name":"MeshNode","mesh":0}
      ],
      "meshes":[{
        "name":"M",
        "primitives":[{
          "attributes":{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2},
          "indices":3,
          "material":0
        }]
      }],
      "materials":[{"name":"mat","pbrMetallicRoughness":{
        "baseColorFactor":[1,0,0,1],"roughnessFactor":0.3,"metallicFactor":0.0,
        "baseColorTexture":{"index":0}}}],
      "skins":[{"joints":[1],"inverseBindMatrices":4}],
      "accessors":[
        {"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","byteOffset":0},
        {"bufferView":0,"componentType":5126,"count":4,"type":"VEC4","byteOffset":12},
        {"bufferView":0,"componentType":5126,"count":4,"type":"VEC4","byteOffset":28},
        {"bufferView":0,"componentType":5125,"count":3,"type":"SCALAR","byteOffset":44},
        {"bufferView":0,"componentType":5126,"count":1,"type":"MAT4","byteOffset":64}
      ],
      "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":256}],
      "images":[{"name":"t","bufferView":0,"mimeType":"image/png"}]
    }"#;

    /// Bajty dla [`MINIMAL`], ułożone w kolejności, w której je czyta
    /// `read_f32`.
    ///
    /// ## Dlaczego nie wektor `Vec<f32>`
    ///
    /// Indeksy są `UNSIGNED_INT` (4 B), a pozycje i wagi `FLOAT` (też
    /// 4 B, ale inna interpretacja bajtów). Wypełnienie jednym wektorem
    /// `f32` dajełoby bajty, które dla indeksów są niepoprawnym
    /// `u32` — stąd osobne `copy_from_slice` na każdy blok.
    fn minimal_bin() -> Vec<u8> {
        let mut b = vec![0u8; 256];
        // Jeden helper zamiast dwóch domknięć: oba pożyczałyby `&mut b`
        // naraz, a Rust tego nie pozwala.
        fn put(b: &mut [u8], off: usize, bytes: &[u8]) {
            b[off..off + bytes.len()].copy_from_slice(bytes);
        }
        // POSITION: VEC3 f32 pod offsetem 0
        put(&mut b, 0, &1.0f32.to_le_bytes());
        // JOINTS_0: VEC4 f32 pod offsetem 12
        put(&mut b, 12, &0.0f32.to_le_bytes());
        // WEIGHTS_0: VEC4 f32 pod offsetem 28
        put(&mut b, 28, &1.0f32.to_le_bytes());
        // indices: SCALAR u32 pod offsetem 44
        put(&mut b, 44, &0u32.to_le_bytes());
        put(&mut b, 48, &0u32.to_le_bytes());
        put(&mut b, 52, &0u32.to_le_bytes());
        // inverseBindMatrices: MAT4 f32 pod offsetem 64 (przekątna = 1)
        put(&mut b, 64, &1.0f32.to_le_bytes());
        put(&mut b, 80, &1.0f32.to_le_bytes());
        put(&mut b, 96, &1.0f32.to_le_bytes());
        put(&mut b, 112, &1.0f32.to_le_bytes());
        b
    }

    #[test]
    fn wczytuje_minimalna_scene() {
        let s = load(&glb(MINIMAL, &minimal_bin())).expect("nie wczytano sceny");
        assert_eq!(s.joints.len(), 1, "szkielet musi mieć 1 kość");
        assert_eq!(s.joints[0].name, "Bone");
        assert_eq!(s.meshes.len(), 1);
        assert_eq!(s.meshes[0].vertices.len(), 1);
        assert_eq!(s.meshes[0].indices, vec![0, 0, 0]);
    }

    #[test]
    fn skinning_przechodzi_przez_wierzcholek() {
        let s = load(&glb(MINIMAL, &minimal_bin())).unwrap();
        let v = s.meshes[0].vertices[0];
        // Pozycja musi przejść przez macierz węzła (tu tożsamość).
        assert!((v.position[0] - 1.0).abs() < 1e-6, "x = {}", v.position[0]);
        assert!((v.weights[0] - 1.0).abs() < 1e-6, "waga = {}", v.weights[0]);
    }

    #[test]
    fn odrzuca_plik_bez_sygnatury_glb() {
        let e = load(b"to nie jest glb, tylko tekst").unwrap_err();
        assert!(
            format!("{e}").contains("glTF"),
            "komunikat musi wspominać o formacie: {e}"
        );
    }

    #[test]
    fn odrzuca_plik_bez_skinu() {
        let json = MINIMAL.replace(
            "\"skins\":[{\"joints\":[1],\"inverseBindMatrices\":4}]",
            "\"skins\":[]",
        );
        let e = load(&glb(&json, &minimal_bin())).unwrap_err();
        assert!(format!("{e}").contains("skin"), "komunikat: {e}");
    }

    #[test]
    fn skin_vertex_ma_64_bajty() {
        // Rozmiar wierzchołka to kontrakt z `VertexBufferLayout` w module
        // `skin` — zmiana jednego bez drugiego psuje renderowanie po cichu.
        assert_eq!(std::mem::size_of::<SkinVertex>(), 64);
        assert_eq!(SkinVertex::STRIDE, 64);
    }

    #[test]
    fn offset_buffer_view_nie_liczony_podwojnie() {
        // Regression: `read_f32` dodało `bv.byteOffset` do danych, które
        // `view_bytes` już przesunęły. Przy offset 0 (małe pliki) błąd
        // był niewidoczny, a na 16 MB assetcie wypadał poza dane.
        // Ten test stawia `byteOffset` na 64 i wymaga, żeby wartość
        // dotarła z właściwego miejsca.
        let json = r#"{
          "scene":0,"scenes":[{"nodes":[0]}],
          "nodes":[{"name":"B"},{"name":"N","mesh":0}],
          "meshes":[{"name":"M","primitives":[{
            "attributes":{"POSITION":0},"indices":1,"material":0}]}],
          "materials":[{"name":"m"}],
          "skins":[{"joints":[0]}],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","byteOffset":0},
            {"bufferView":0,"componentType":5125,"count":3,"type":"SCALAR","byteOffset":16}
          ],
          "bufferViews":[{"buffer":0,"byteOffset":32,"byteLength":64}]
        }"#;
        // Bufor 96 B. Wewnątrz widoku (od bajtu 32) pozycja leży pod 0,
        // a indeksy pod 16. Pozycja to VEC3 = 12 B, więc 16 zostawia
        // 4 B odstępu — gdyby indeksy stały pod 12, nachodziłyby na
        // pozycję i test sprawdzałby coś zupełnie innego.
        //
        // Wartości 5/6/7 są celowo różne od zera: gdyby loader czytał
        // niewłaściwy fragment, zamiast trzech zer zobaczylibyśmy w
        // komunikacie błędu konkretne bajty.
        let mut bin = vec![0u8; 96];
        bin[32..36].copy_from_slice(&7.0f32.to_le_bytes());
        for (k, idx) in [5u32, 6, 7].iter().enumerate() {
            let o = 32 + 16 + k * 4;
            bin[o..o + 4].copy_from_slice(&idx.to_le_bytes());
        }
        let s = load(&glb(json, &bin)).expect("nie wczytano");
        assert!(
            (s.meshes[0].vertices[0].position[0] - 7.0).abs() < 1e-6,
            "pozycja x = {}, oczekiwano 7.0 (offset pomieszany?)",
            s.meshes[0].vertices[0].position[0]
        );
        assert_eq!(
            s.meshes[0].indices,
            vec![5, 6, 7],
            "indeksy spójne — `left` pokazuje, czyje bajty przeczytano"
        );
    }

    #[test]
    fn alpha_mask_nie_requiere_blendu() {
        // Wyrzynane włosy rysujemy w passie głównym; tylko BLEND
        // potrzebuje wtórnego bufora i depth-write wyłączonego.
        assert!(!AlphaMode::Mask.needs_blend());
        assert!(!AlphaMode::Opaque.needs_blend());
        assert!(AlphaMode::Blend.needs_blend());
    }

    /// Ścieżka do assetu testowego. Test się pomija, gdy pliku nie ma —
    /// workspace ma dać się zbudować bez ciężkich assetów.
    fn asset() -> Option<Vec<u8>> {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../endfield-3d/assets/Free test/Theresa - Free Test.glb"
        );
        std::fs::read(p).ok()
    }

    /// Test na prawdziwym pliku 16 MB.
    ///
    /// To jedyny test, który widzi komplet danych — 22 tekstury osadzone
    /// w BIN, `byteStride` w `bufferViews`, `u16` w `JOINTS_0`. Na
    /// syntetycznych bajtach z `MINIMAL` te ścieżki kodu nigdy by nie
    /// weszły, a to właśnie w nich kryją się błędy „działa na małym
    /// pliku, psuje się na 16 MB".
    #[test]
    fn wczytuje_prawdziwy_asset_postaci() {
        let Some(bytes) = asset() else {
            eprintln!("pominięto: brak pliku .glb w assets");
            return;
        };
        let s = load(&bytes).expect("nie wczytano assetu postaci");

        assert_eq!(s.joints.len(), 73, "szkielet ma 73 kości");
        assert_eq!(s.meshes.len(), 11, "3 meshe, 11 prymitywów łącznie");
        assert!(s.materials.len() >= 11, "materiałów: {}", s.materials.len());
        assert!(s.textures.len() >= 20, "tekstur: {}", s.textures.len());
        assert_eq!(s.inverse_bind.len(), 73, "macierz wiązania na kość");

        // Nazwy kości muszą się zgadzać z klipami animacji (.urananm),
        // bo eksport z Blendera indeksuje kości dokładnie po nazwie.
        assert!(s.joint_by_name("J_Bip_C_Hips").is_some(), "brak bioder");
        assert!(s.joint_by_name("J_Bip_C_Head").is_some(), "brak głowy");
        assert!(
            s.joint_by_name("J_Bip_L_Foot").is_some(),
            "brak lewej stopy"
        );
        // Hierarchia: stopa musi wisieć na goleni.
        let foot = s.joint_by_name("J_Bip_L_Foot").unwrap();
        let shin = s.joint_by_name("J_Bip_L_LowerLeg").unwrap();
        assert_eq!(s.joints[foot].parent, Some(shin), "stopa pod golenią");
        assert!(s.joints[shin].parent.is_some(), "goleń ma rodzica");

        // Skinning: wagi jednego wierzchołka sumują się do ~1.
        let v = &s.meshes[0].vertices[0];
        let wsum: f32 = v.weights.iter().sum();
        assert!(
            (wsum - 1.0).abs() < 0.01,
            "wagi wierzchołka sumują się do {wsum}, nie do 1"
        );

        // Skala: człowiek ma ~1.5 m — nie 0.015 m i nie 150 m.
        let h = s.bounds_height();
        assert!((1.3..1.7).contains(&h), "wysokość modelu {h} m");

        // Mapa normalnych nie może być oznaczona jako kolor (sRGB) —
        // wtedy GPU zdekodowałaby ją jak kolor i wygładziła relief.
        if let Some(i) = s.materials.iter().find_map(|m| m.normal_texture) {
            assert!(!s.textures[i].is_color, "mapa normalnych nie może być sRGB");
        }
    }
}

/// Typy z rozszerzeń, które świadomie pomijamy.
///
/// Nie jest to lista „braków do naprawy", tylko uczciwe sprawozdanie
/// z tego, czego loader nie czyta. Bez tego plik z Draco wyglądałby
/// na zaimportowany poprawnie, a byłby pusty.
pub const UNSUPPORTED: &[&str] = &[
    "Draco (KHR_draco_mesh_compression)",
    "morph targets (prymitywy > 1 kształtu)",
    "tryby inne niż TRIANGLES",
];

/// Kontener GLB: nagłówek + chunki (JSON i BIN).
struct Container<'a> {
    json: Json,
    bin: &'a [u8],
}

fn read_container(bytes: &[u8]) -> Result<Container<'_>, GltfError> {
    if bytes.len() < 12 {
        return err("plik za krótki na nagłówek GLB");
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != 0x4654_6C67 {
        return err("brak sygnatury `glTF` — to nie jest plik .glb");
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version != 2 {
        return err(format!("wersja glTF {version} (obsługujemy 2)"));
    }
    let total = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if total > bytes.len() {
        return err(format!(
            "nagłówek obiecuje {total} B, a plik ma {}",
            bytes.len()
        ));
    }

    // Chunki: (długość, typ) + dane. Kolejność bywa JSON, BIN, ale
    // szukamy ich po typie, nie po pozycji.
    let mut json_bytes: Option<&[u8]> = None;
    let mut bin: &[u8] = &[];
    let mut off = 12usize;
    while off + 8 <= total {
        let len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        let kind = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        let start = off + 8;
        if start + len > total {
            return err("chunk sięga za koniec pliku");
        }
        match kind {
            0x4E4F_534A => json_bytes = Some(&bytes[start..start + len]),
            0x004E_4942 => bin = &bytes[start..start + len],
            _ => {} // nieznany chunk pomijamy zgodnie ze specyfikacją
        }
        off = start + len + ((4 - len % 4) % 4);
    }

    let Some(jb) = json_bytes else {
        return err("brak chunku JSON");
    };
    // Chunk JSON bywa dopełniony spacjami do wielokrotności 4.
    let text = std::str::from_utf8(jb).map_err(|e| GltfError {
        msg: format!("JSON nie jest UTF-8: {e}"),
    })?;
    let json = json::parse(text.trim_end()).map_err(|e| GltfError {
        msg: format!("JSON: {e}"),
    })?;
    Ok(Container { json, bin })
}

/// Wierzchołek ze skinningiem: pozycja, normalna, UV, 4 kości, 4 wagi.
///
/// ## Dlaczego osobny typ, a nie rozszerzenie [`crate::mesh::Vertex`]
///
/// `Vertex` ma 44 B i jest częścią kontraktu z potokiem w `scene.rs` —
/// test `vertex_is_44_bytes_and_uniform_is_aligned` pilnuje tego rozmiaru,
/// a jego zmiana cicho psuje cały renderer 3D dla wszystkich demek.
/// Dokładanie 24 B (4×`u16` + 4×`f32`) rozszerzyłoby każdą bryłę świata
/// o 55% rozmiaru wierzchołka, żeby skinning miały tylko postacie.
///
/// Dlatego postacie mają własny format i własny potok, a reszta silnika
/// nawet o nim nie wie.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Indeksy kości — 4 na wierzchołek, bo tyle obsługuje sprzęt.
    ///
    /// W pliku są `u16`; trzymamy je jako `u32`, bo WGSL nie ma sensownego
    /// `array<u16, 4>` w uniformie. Nieużywane sloty mają indeks 0 i wagę 0.
    pub joints: [u32; 4],
    /// Wagi — suma nie musi wynosić 1, bo plik może mieć mniej niż 4
    /// wpływy na wierzchołek; shader normalizuje.
    pub weights: [f32; 4],
}

impl SkinVertex {
    /// Rozmiar: 3+3+2+4+4 = 16 liczb po 4 B = 64 B.
    ///
    /// Test `skin_vertex_is_64_bytes` pilnuje zgodności z
    /// `VertexBufferLayout` w module `skin`.
    pub const STRIDE: u64 = 64;
}

/// Siatka postaci: wierzchołki ze skinningiem + indeksy + materiał.
#[derive(Debug, Clone, Default)]
pub struct SkinMesh {
    pub name: String,
    pub vertices: Vec<SkinVertex>,
    pub indices: Vec<u32>,
    /// Indeks materiału w [`GltfScene::materials`].
    pub material: usize,
}

/// Opis materiału po stronie CPU — do zbudowania uniforma w `MaterialBank`.
///
/// glTF niesie więcej niż PBR, ale `MaterialBank` przyjmuje płaską
/// strukturę, więc mapujemy pola na `Surface`. Czego nie umiemy przenieść
/// wprost (przezroczystość), trzymamy w [`Self::alpha_mode`].
#[derive(Debug, Clone)]
pub struct GltfMaterial {
    pub name: String,
    /// Kolor bazowy pomnożony z teksturą.
    pub base_color: [f32; 4],
    pub base_texture: Option<usize>,
    pub normal_texture: Option<usize>,
    /// ORM trzymamy osobno, choć ten asset go nie używa — inne modele mają.
    pub orm_texture: Option<usize>,
    pub roughness: f32,
    pub metallic: f32,
    /// `OPAQUE` / `MASK` / `BLEND` — decyzja o rysowaniu z wtórnym
    /// buforem i o depth-write.
    pub alpha_mode: AlphaMode,
    /// Próg wycinania dla `MASK` (glTF `alphaCutoff`, domyślnie 0.5).
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    /// Emisja HDR — oczy i podświetlenia świecą w bloomie.
    pub emissive: [f32; 3],
    pub emissive_texture: Option<usize>,
}

/// Tryb przezroczystości z glTF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

impl Default for AlphaMode {
    fn default() -> Self {
        Self::Opaque
    }
}

impl AlphaMode {
    /// Czy materiał potrzebuje rysowania w kolejce alfa.
    ///
    /// `Mask` (wyrzynanie włosów) da się narysować w passie głównym,
    /// bo jest binarny — potrzebuje odrzucania, nie mieszania.
    pub fn needs_blend(&self) -> bool {
        matches!(self, AlphaMode::Blend)
    }
}

/// Tekstura z pliku — gotowe bajty do wrzucenia na GPU.
#[derive(Debug, Clone)]
pub struct GltfTexture {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Czy to mapa kolorów (sRGB) czy dane (normalna, ORM).
    ///
    /// Wpisanie normalnej w sRGB to klasyczny błąd: GPU zdekodowałby ją
    /// jak kolor i wygładził nierówności, przez co oświetlenie pojechałoby
    /// w złą stronę.
    pub is_color: bool,
}

/// Węzeł szkieletu w pozy spoczynkowej.
///
/// glTF trzyma tylko TRS węzła i osobno macierze `inverseBindMatrices`
/// dla skinu. Nie warto tego składać na raz — do animacji potrzebujemy
/// macierzy lokalnych, a do skinningu odwrotnych, więc czytamy obie
/// i trzymamy je osobno.
#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    /// Węzeł nadrzędny, `None` dla korzenia szkieletu.
    pub parent: Option<usize>,
    /// Transformacja lokalna w pozy spoczynkowej (bind pose).
    pub local: Mat4,
    /// Indeks w liscie `skins[].joints` — to definiuje kolejność
    /// macierzy, której oczekuje `JOINTS_0` na wierzchołkach.
    pub skin_index: u32,
}

/// Model postaci: siatki, materiały, tekstury, szkielet.
///
/// Wszystko jest w jednej strukturze, bo zwykle ładujemy plik w całości
/// i trzymamy w pamięci przez całą grę. Podział na osobne uchwyt
/// (`MeshHandle`, `SkelHandle`) byłby pozorną oszczędnością — plik ma
/// 19 tys. wierzchołków i mieści się w kilku MB.
#[derive(Debug, Default)]
pub struct GltfScene {
    pub name: String,
    pub meshes: Vec<SkinMesh>,
    pub materials: Vec<GltfMaterial>,
    pub textures: Vec<GltfTexture>,
    pub joints: Vec<Joint>,
    /// `inverseBindMatrices` w kolejności `skins[].joints`.
    pub inverse_bind: Vec<Mat4>,
    /// Czego w pliku nie umieliśmy przeczytać.
    pub unsupported: Vec<&'static str>,
    /// Skala dopasowania modelu: po zaimportowaniu człowiek ma 1.7 m.
    ///
    /// glTF traktuje jednostki jako metry, a ten asset jest tak
    /// przygotowany, ale inne eksporty bywają w centymetrach. Pole
    /// daje to wyraźnie do korekty bez zgadywania.
    pub unit_scale: f32,
}

impl GltfScene {
    /// Indeks kości o podanej nazwie — dla animacji po nazwie.
    pub fn joint_by_name(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Wysokość modelu w jednostkach pliku (najwyższy wierzchołek).
    pub fn bounds_height(&self) -> f32 {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for m in &self.meshes {
            for v in &m.vertices {
                lo = lo.min(v.position[1]);
                hi = hi.max(v.position[1]);
            }
        }
        if lo > hi {
            0.0
        } else {
            hi - lo
        }
    }
}

/// Kody typów komponentów z glTF.
const F32: u32 = 5126;
const U16: u32 = 5123;
const U8: u32 = 5121;

/// `UNSIGNED_INT` — typ komponentu, którego glTF używa dla niemal
/// wszystkich indeksów wierzchołków. Bez wariantu poniżej `match`
/// zwracałby `0.0`, czyli **każdy trójkąt degenerowałby się do punktu**
/// i postać zniknęłaby z ekranu.
const U32: u32 = 5125;

/// `componentType` + `type` jako (rozmiar elementu, liczba składowych).
fn comp_info(component: u32, ty: &str) -> Option<(usize, usize)> {
    let size = match component {
        5120 | U8 => 1,
        U16 => 2,
        5122 => 2,
        5125 => 4,
        F32 => 4,
        _ => return None,
    };
    let n = match ty {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT4" => 16,
        _ => return None,
    };
    Some((size, n))
}

/// Czyta `bufferView` i zwraca bajty z zakresu `offset..offset+len`.
fn view_bytes<'a>(c: &Container<'a>, idx: usize) -> Result<&'a [u8], GltfError> {
    let bv = c
        .json
        .get("bufferViews")
        .and_then(|v| v.idx(idx))
        .ok_or_else(|| GltfError {
            msg: format!("brak bufferView {idx}"),
        })?;
    let off = bv.get("byteOffset").and_then(Json::as_u32).unwrap_or(0) as usize;
    let len = bv
        .get("byteLength")
        .and_then(Json::as_u32)
        .ok_or_else(|| GltfError {
            msg: format!("bufferView {idx} bez byteLength"),
        })? as usize;
    let end = off.checked_add(len).ok_or_else(|| GltfError {
        msg: "przepełnienie offsetu bufferView".into(),
    })?;
    c.bin.get(off..end).ok_or_else(|| GltfError {
        msg: format!(
            "bufferView {idx} wychodzi poza BIN chunk ({} B)",
            c.bin.len()
        ),
    })
}

/// Czytuje accessor jako płaskie `Vec<f32>`, konwertując typy całkowite.
///
/// Wagi skinningu bywają `u8`/`u16` znormalizowane, a `f32` — więc
/// jedna ścieżka odczytu zamiast osobnej dla każdego typu.
fn read_f32(c: &Container<'_>, idx: usize) -> Result<Vec<f32>, GltfError> {
    let acc = c
        .json
        .get("accessors")
        .and_then(|a| a.idx(idx))
        .ok_or_else(|| GltfError {
            msg: format!("brak accessor {idx}"),
        })?;
    let component = acc
        .get("componentType")
        .and_then(Json::as_u32)
        .ok_or_else(|| GltfError {
            msg: format!("accessor {idx} bez componentType"),
        })?;
    let ty = acc.get("type").and_then(Json::as_str).unwrap_or("SCALAR");
    let count = acc
        .get("count")
        .and_then(Json::as_u32)
        .ok_or_else(|| GltfError {
            msg: format!("accessor {idx} bez count"),
        })? as usize;
    let normalized = acc
        .get("normalized")
        .and_then(Json::as_bool)
        .unwrap_or(false);

    let Some(bvi) = acc.get("bufferView").and_then(Json::as_u32) else {
        // Accessor bez `bufferView` to same zera — dozwolone przez specyfikację.
        return Ok(vec![
            0.0;
            count * comp_info(component, ty).map_or(1, |x| x.1)
        ]);
    };
    let bv = c
        .json
        .get("bufferViews")
        .and_then(|v| v.idx(bvi as usize))
        .ok_or_else(|| GltfError {
            msg: format!("brak bufferView {bvi}"),
        })?;
    // `view_bytes` zwraca już dane **przesunięte** o `byteOffset`
    // bufferView, więc tutaj wolno dodać wyłącznie przesunięcie samego
    // accessora. Dodanie tu jeszcze raz `bv.byteOffset` to klasyczny
    // podwójny offset — na małych plikach (offset 0) przechodzi, a na
    // realnym assecte 16 MB wywala się, bo `u32` się przepełnia zakres.
    let base = acc.get("byteOffset").and_then(Json::as_u32).unwrap_or(0) as usize;
    let (size, ncomp) = comp_info(component, ty).ok_or_else(|| GltfError {
        msg: format!("accessor {idx}: typ {ty} nieobsługiwany"),
    })?;
    let data = view_bytes(c, bvi as usize)?;

    // `byteStride` bywa większy niż rozmiar elementu (interleaving).
    let stride = bv.get("byteStride").and_then(Json::as_u32).unwrap_or(0) as usize;
    let stride = if stride == 0 { size * ncomp } else { stride };

    let mut out = Vec::with_capacity(count * ncomp);
    for i in 0..count {
        let start = base + i * stride;
        for k in 0..ncomp {
            let o = start + k * size;
            let b = data.get(o..o + size).ok_or_else(|| GltfError {
                msg: format!("accessor {idx}: brak bajtów dla elementu {i}"),
            })?;
            let v = match component {
                F32 => f32::from_le_bytes(b.try_into().unwrap()),
                U16 => {
                    let v = u16::from_le_bytes(b.try_into().unwrap()) as f32;
                    // `normalized` znaczy dzielenie przez 65535, nie surowa wartość
                    if normalized {
                        v / 65535.0
                    } else {
                        v
                    }
                }
                5122 => i16::from_le_bytes(b.try_into().unwrap()) as f32,
                U32 => u32::from_le_bytes(b.try_into().unwrap()) as f32,
                U8 => {
                    let v = b[0] as f32;
                    if normalized {
                        v / 255.0
                    } else {
                        v
                    }
                }
                5120 => b[0] as i8 as f32,
                _ => 0.0,
            };
            out.push(v);
        }
    }
    Ok(out)
}

/// Wierzchołek w układzie pliku przed zastosowaniem macierzy węzła.
///
/// glTF pozwala, żeby pozycje były już w przestrzeni świata, a wtedy
/// transformacja węzła to tylko orientacja. Dla skinningu to nie ma
/// znaczenia — i tak liczymy macierz `bone * IBM` i mnożymy wierzchołek —
/// ale normalne muszą dostać tę samą bazę, inaczej oświetlenie jest
/// przekrzywione o pozycję modelu.
struct RawPrimitives {
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    joints: Vec<f32>,
    weights: Vec<f32>,
    indices: Vec<u32>,
    material: usize,
}

/// Wczytuje jeden prymityw siatki ze skinningiem.
fn read_primitive(
    c: &Container<'_>,
    prim: &Json,
    fallback_mat: usize,
) -> Result<RawPrimitives, GltfError> {
    let mode = prim.get("mode").and_then(Json::as_u32).unwrap_or(4);
    if mode != 4 {
        return err(format!("prymityw w trybie {mode} (chcemy tylko trójkąty)"));
    }
    let attrs = prim.get("attributes").ok_or_else(|| GltfError {
        msg: "prymityw bez `attributes`".into(),
    })?;
    let attr = |name: &str| -> Result<u32, GltfError> {
        attrs
            .get(name)
            .and_then(Json::as_u32)
            .ok_or_else(|| GltfError {
                msg: format!("prymityw bez atrybutu {name}"),
            })
    };

    let positions = read_f32(c, attr("POSITION")? as usize)?;
    let nverts = positions.len() / 3;

    // Normalna i UV są opcjonalne w specyfikacji — bez nich dostajemy
    // wartości neutralne, a nie błąd, bo plik nadal da się wyświetlić.
    let normals = match attrs.get("NORMAL").and_then(Json::as_u32) {
        Some(i) => read_f32(c, i as usize)?,
        None => vec![0.0; nverts * 3],
    };
    let uvs = match attrs.get("TEXCOORD_0").and_then(Json::as_u32) {
        Some(i) => read_f32(c, i as usize)?,
        None => vec![0.0; nverts * 2],
    };

    // Skinning: brak `JOINTS_0` znaczy statyczną siatkę, wtedy
    // przypisujemy kość 0 z wagą 1 — geometria zostaje nieruchoma.
    let (joints, weights) = match attrs.get("JOINTS_0").and_then(Json::as_u32) {
        Some(ji) => {
            let w = attrs
                .get("WEIGHTS_0")
                .and_then(Json::as_u32)
                .ok_or_else(|| GltfError {
                    msg: "JOINTS_0 bez WEIGHTS_0".into(),
                })?;
            (read_f32(c, ji as usize)?, read_f32(c, w as usize)?)
        }
        None => {
            let mut j = vec![0.0; nverts * 4];
            let mut w = vec![0.0; nverts * 4];
            for v in 0..nverts {
                w[v * 4] = 1.0;
            }
            (j, w)
        }
    };

    let indices = match prim.get("indices").and_then(Json::as_u32) {
        Some(i) => read_f32(c, i as usize)?
            .into_iter()
            .map(|v| v as u32)
            .collect(),
        // Bez indeksów to kolejność wierzchołków 0..n.
        None => (0..nverts as u32).collect(),
    };

    let material = prim
        .get("material")
        .and_then(Json::as_u32)
        .unwrap_or(fallback_mat as u32) as usize;

    Ok(RawPrimitives {
        positions,
        normals,
        uvs,
        joints,
        weights,
        indices,
        material,
    })
}

/// Wyciąga indeks tekstury z referencji `{"index": N}` na obiekcie.
fn tex_ref(v: &Json) -> Option<usize> {
    v.get("index").and_then(Json::as_u32).map(|i| i as usize)
}

fn alpha_mode(s: Option<&str>) -> AlphaMode {
    match s {
        Some("MASK") => AlphaMode::Mask,
        Some("BLEND") => AlphaMode::Blend,
        _ => AlphaMode::Opaque,
    }
}

fn read_materials(c: &Container<'_>, default_tex: Option<usize>) -> Vec<GltfMaterial> {
    let Some(list) = c.json.get("materials").and_then(Json::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|m| {
            let pbr = m.get("pbrMetallicRoughness");
            let base_color = pbr
                .and_then(|p| p.get("baseColorFactor"))
                .and_then(Json::as_f64_vec)
                .map(|v| {
                    let mut c = [1.0f32; 4];
                    for (i, x) in v.iter().take(4).enumerate() {
                        c[i] = *x as f32;
                    }
                    c
                })
                .unwrap_or([1.0; 4]);
            GltfMaterial {
                name: m
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                base_color,
                base_texture: pbr
                    .and_then(|p| p.get("baseColorTexture"))
                    .and_then(tex_ref)
                    .or(default_tex),
                normal_texture: m.get("normalTexture").and_then(tex_ref),
                orm_texture: pbr
                    .and_then(|p| p.get("metallicRoughnessTexture"))
                    .and_then(tex_ref),
                // glTF trzyma roughness/metallic w jednym mnożniku
                // (G rozkład, B metaliczność), a silnik ma je osobno —
                // dlatego czytamy obie składowe niezależnie.
                roughness: pbr
                    .and_then(|p| p.get("roughnessFactor"))
                    .and_then(Json::as_f64)
                    .unwrap_or(1.0) as f32,
                metallic: pbr
                    .and_then(|p| p.get("metallicFactor"))
                    .and_then(Json::as_f64)
                    .unwrap_or(1.0) as f32,
                alpha_mode: alpha_mode(m.get("alphaMode").and_then(Json::as_str)),
                alpha_cutoff: m.get("alphaCutoff").and_then(Json::as_f64).unwrap_or(0.5) as f32,
                double_sided: m
                    .get("doubleSided")
                    .and_then(Json::as_bool)
                    .unwrap_or(false),
                emissive: m
                    .get("emissiveFactor")
                    .and_then(Json::as_f64_vec)
                    .map(|v| {
                        let mut e = [0.0f32; 3];
                        for (i, x) in v.iter().take(3).enumerate() {
                            e[i] = *x as f32;
                        }
                        e
                    })
                    .unwrap_or([0.0; 3]),
                emissive_texture: m.get("emissiveTexture").and_then(tex_ref),
            }
        })
        .collect()
}

/// Które tekstury są kolorowe, a które danymi.
///
/// Robimy to po tym, co z nich faktycznie czyta dany materiał, a nie po
/// nazwie pliku: eksporter nazywa je dowolnie.
fn mark_color_textures(mats: &[GltfMaterial], texs: &mut [GltfTexture]) {
    for m in mats {
        for t in [m.base_texture, m.emissive_texture].into_iter().flatten() {
            if let Some(x) = texs.get_mut(t) {
                x.is_color = true;
            }
        }
    }
}

fn read_textures(c: &Container<'_>, mats: &[GltfMaterial]) -> Vec<GltfTexture> {
    let mut out = Vec::new();
    let Some(images) = c.json.get("images").and_then(Json::as_array) else {
        return out;
    };
    let Some(samplers) = c.json.get("samplers").and_then(Json::as_array) else {
        return out;
    };
    for im in images {
        let Some(bvi) = im.get("bufferView").and_then(Json::as_u32) else {
            // Obraz z zewnętrznym `uri` wymagałby czytania z dysku —
            // przy `.glb` nie występuje, więc to cicha pominięcie.
            continue;
        };
        let Ok(bytes) = view_bytes(c, bvi as usize) else {
            continue;
        };
        let mime = im
            .get("mimeType")
            .and_then(Json::as_str)
            .unwrap_or("image/png");
        // `samplers[i].source` wskazuje obraz; ścieżka idzie przez teksturę.
        let _ = (mime, samplers);
        out.push(GltfTexture {
            name: im
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("tex")
                .to_string(),
            bytes: bytes.to_vec(),
            is_color: false,
        });
    }
    mark_color_textures(mats, &mut out);
    out
}

/// Buduje listę kości: rodzic, macierz lokalna, indeks w skinie.
fn read_skeleton(
    c: &Container<'_>,
    out: &mut Vec<Joint>,
    inverse: &mut Vec<Mat4>,
) -> Result<(), GltfError> {
    let skins = c
        .json
        .get("skins")
        .and_then(Json::as_array)
        .ok_or_else(|| GltfError {
            msg: "plik nie ma `skins` — to nie postać".into(),
        })?;
    let Some(skin) = skins.first() else {
        return err("pusta lista `skins`");
    };
    let joint_ids: Vec<usize> = skin
        .get("joints")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Json::as_u32)
                .map(|i| i as usize)
                .collect()
        })
        .unwrap_or_default();
    if joint_ids.is_empty() {
        return err("skin bez listy `joints`");
    }

    let nodes = c
        .json
        .get("nodes")
        .and_then(Json::as_array)
        .ok_or_else(|| GltfError {
            msg: "brak `nodes`".into(),
        })?;

    // Rodzic po indeksie węzła — potrzebny do złożenia hierarchii.
    let mut parent_of = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        if let Some(kids) = n.get("children").and_then(Json::as_array) {
            for k in kids.iter().filter_map(Json::as_u32) {
                parent_of[k as usize] = Some(i);
            }
        }
    }

    let mut index_of = BTreeMap::new();
    for (si, id) in joint_ids.iter().enumerate() {
        index_of.insert(*id, si);
    }

    for (si, id) in joint_ids.iter().enumerate() {
        let n = nodes.get(*id).ok_or_else(|| GltfError {
            msg: format!("węzeł {id} poza zakresem"),
        })?;
        out.push(Joint {
            name: n
                .get("name")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string(),
            parent: parent_of[*id].and_then(|p| index_of.get(&p).copied()),
            local: node_matrix(n),
            skin_index: si as u32,
        });
    }

    // `inverseBindMatrices` to macierze 4×4 kolumnowo, w kolejności
    // `skins[].joints`. Brak pola = same macierze tożsamości.
    if let Some(acc) = skin.get("inverseBindMatrices").and_then(Json::as_u32) {
        let flat = read_f32(c, acc as usize)?;
        for m in flat.chunks_exact(16) {
            inverse.push(mat_from_cols(m));
        }
    }
    while inverse.len() < out.len() {
        inverse.push(Mat4::IDENTITY);
    }
    Ok(())
}

/// Macierz lokalna węzła z TRS (domyślnie tożsamość).
fn node_matrix(n: &Json) -> Mat4 {
    let t = n
        .get("translation")
        .and_then(Json::as_f64_vec)
        .map(|v| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
        .unwrap_or(Vec3::ZERO);
    let r = n
        .get("rotation")
        .and_then(Json::as_f64_vec)
        .map(|v| Quat::from_xyzw(v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32))
        .unwrap_or(Quat::IDENTITY);
    let s = n
        .get("scale")
        .and_then(Json::as_f64_vec)
        .map(|v| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
        .unwrap_or(Vec3::ONE);
    // Złożenie T*R*S z osobnych konstruktorów — tak robi to `mesh.rs`
    // w `model_matrix`, więc zgodność z resztą silnika jest widoczna
    // na pierwszy rzut oka.
    Mat4::from_translation(t) * Mat4::from_mat3(Mat3::from_quat(r)) * Mat4::from_scale(s)
}

/// Buduje macierz z 16 liczb w kolejności kolumnowej (jak glTF).
fn mat_from_cols(m: &[f32]) -> Mat4 {
    // `Mat4::from_cols` składa kolumny, a glTF zapisuje macierz
    // kolumnowo — te same cztery kolumny, tylko w innym zapisie.
    Mat4::from_cols(
        Vec4::new(m[0], m[1], m[2], m[3]),
        Vec4::new(m[4], m[5], m[6], m[7]),
        Vec4::new(m[8], m[9], m[10], m[11]),
        Vec4::new(m[12], m[13], m[14], m[15]),
    )
}

/// Wczytuje plik `.glb` ze ścieżki.
pub fn load_from_file(path: &str) -> Result<GltfScene, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    load(&bytes)
}

/// Wczytuje model z bajtów pliku `.glb`.
///
/// Każdy prymityw staje się osobną [`SkinMesh`], bo każdy ma własny
/// materiał — a `DrawCmd` w silniku niesie jeden `MaterialId`.
pub fn load(bytes: &[u8]) -> Result<GltfScene, Box<dyn std::error::Error>> {
    let c = read_container(bytes)?;

    let mut scene = GltfScene {
        unit_scale: 1.0,
        ..Default::default()
    };

    read_skeleton(&c, &mut scene.joints, &mut scene.inverse_bind)?;
    scene.materials = read_materials(&c, None);
    scene.textures = read_textures(&c, &scene.materials);

    let nodes = c.json.get("nodes").and_then(Json::as_array);
    let meshes = c.json.get("meshes").and_then(Json::as_array);

    if let Some(nodes) = nodes {
        for n in nodes {
            let Some(mi) = n.get("mesh").and_then(Json::as_u32) else {
                continue;
            };
            let Some(m) = meshes.and_then(|m| m.get(mi as usize)) else {
                return Err(Box::new(GltfError {
                    msg: format!("węzeł wskazuje na nieistniejący mesh {mi}"),
                }) as Box<dyn std::error::Error>);
            };
            let prims = m.get("primitives").and_then(Json::as_array).unwrap_or(&[]);
            let mesh_name = m.get("name").and_then(Json::as_str).unwrap_or("mesh");
            let node_name = n.get("name").and_then(Json::as_str).unwrap_or("");
            let world = node_matrix(n);

            for (pi, prim) in prims.iter().enumerate() {
                let raw = read_primitive(&c, prim, 0)?;
                // Nazwa musi rozróżniać prymitywy jednego mesha —
                // inaczej w logu zobaczysz trzy „Body".
                let name = if node_name.is_empty() {
                    format!("{mesh_name}#{pi}")
                } else {
                    format!("{node_name}/{mesh_name}#{pi}")
                };
                scene.meshes.push(build_skin_mesh(name, raw, world));
            }
        }
    }

    if scene.meshes.is_empty() {
        return Err(Box::new(GltfError {
            msg: "plik nie zawiera żadnej geometrii".into(),
        }) as Box<dyn std::error::Error>);
    }
    if !c
        .json
        .get("animations")
        .and_then(Json::as_array)
        .map_or(false, |a| a.is_empty())
    {
        scene
            .unsupported
            .push("animacje osadzone w glTF (czytamy z .urananm)");
    }
    Ok(scene)
}

/// Składa [`SkinVertex`] z surowego prymitywu.
fn build_skin_mesh(name: String, raw: RawPrimitives, world: Mat4) -> SkinMesh {
    let n = raw.positions.len() / 3;
    let mut verts = Vec::with_capacity(n);
    // Normalna potrzebuje macierzy **odwrotnej transponowanej**, nie
    // zwykłej: przy skalowaniu modelu zwykła macierz przekrzywia
    // kierunek światła. W glam `inverse()` zwraca same wartości, więc
    // nie ma czego obsługiwać — osobliwe macierze dają NaN, a tych
    // w poprawnym pliku glTF nie ma.
    let inv = world.inverse().transpose();
    let normal_mat = Mat3::from_cols(
        inv.x_axis.truncate(),
        inv.y_axis.truncate(),
        inv.z_axis.truncate(),
    );
    for v in 0..n {
        let p = Vec3::new(
            raw.positions[v * 3],
            raw.positions[v * 3 + 1],
            raw.positions[v * 3 + 2],
        );
        // `Mat4 * Vec3` nie istnieje w glam — punkt mnożymy przez kolumnę
        // z jedynką na końcu, inaczej dostalibyśmy dzielenie przez `w`.
        let p = (world * p.extend(1.0)).truncate();
        let nv = if raw.normals.len() >= (v + 1) * 3 {
            let x = Vec3::new(
                raw.normals[v * 3],
                raw.normals[v * 3 + 1],
                raw.normals[v * 3 + 2],
            );
            (normal_mat * x).normalize_or_zero()
        } else {
            Vec3::Y
        };
        let uv = if raw.uvs.len() >= (v + 1) * 2 {
            Vec2::new(raw.uvs[v * 2], raw.uvs[v * 2 + 1])
        } else {
            Vec2::ZERO
        };
        let mut joints = [0u32; 4];
        let mut weights = [0.0f32; 4];
        for k in 0..4 {
            let ji = (v * 4 + k) as usize;
            let wi = (v * 4 + k) as usize;
            if raw.joints.get(ji).is_some() {
                joints[k] = raw.joints[ji] as u32;
            }
            if raw.weights.get(wi).is_some() {
                weights[k] = raw.weights[wi];
            }
        }
        verts.push(SkinVertex {
            position: p.to_array(),
            normal: nv.to_array(),
            uv: uv.to_array(),
            joints,
            weights,
        });
    }
    SkinMesh {
        name,
        vertices: verts,
        indices: raw.indices,
        material: raw.material,
    }
}
