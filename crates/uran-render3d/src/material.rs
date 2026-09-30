//! Materiały PBR na GPU: tekstury, parametry i ich wiązanie z shaderem.
//!
//! ## Po co to istnieje
//!
//! Parser `.mtl` (moduł `import`) umie przeczytać `map_Kd`, `map_Bump`
//! i `map_Pr`/`map_Pm`, ale wczytany plik to dopiero opis. Ten moduł
//! zamienia go w zasoby, które wgpu potrafi związać z potokiem.
//!
//! ## Trzy tekstury, nie jedna
//!
//! Blender eksportuje „wypalony" materiał jako trzy mapy:
//!
//! | Tekstura | Kanały | Format wgpu | Dlaczego nie sRGB |
//! |----------|--------|-------------|-------------------|
//! | Base Color | RGB | `Rgba8UnormSrgb` | kolor — musi być zdekodowany przez GPU |
//! | Normal | RGB | `Rgba8Unorm` | wektor, nie kolor |
//! | ORM | R=AO, G=rough, B=metal | `Rgba8Unorm` | wartości fizyczne |
//!
//! Wpisanie mapy normalnych w formacie sRGB jest **klasycznym błędem**:
//! GPU zdekodowałby ją jak kolor, wygładził nierówności i normalne
//! pojechałyby w złą stronę. Dlatego format dobieramy z roli tekstury,
//! a nie z roli pliku.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::import::Material;

/// Parametry materiału przekazywane do shadera (5 × `vec4` = 80 B).
///
/// Wszystko w `vec4`, bo WGSL wyrównuje `vec3` do 16 B — struktura
/// z samymi `f32` i `vec3` dałaby inny rozmiar po obu stronach i
/// walidacja wgpu odrzuciłaby bufor.
///
/// ## Dlaczego pięć `vec4`
///
/// W stylu Endfield materiał musi obsłużyć jednocześnie PBR i rysunek
/// anime. Rozdzielenie tego na osobne struktury tylko mnożyłoby bind
/// grupy, więc trzymamy tu komplet: warstwa bazowa (`params`, `flags`),
/// cechy powierzchni (`surface`) i barwy dodatkowe (`tint`, `accent`).
///
/// | pole | zawartość | rozmiar |
/// |------|-----------|---------|
/// | `params` | chropowatość, metaliczność, UV, normal mapping | 16 B |
/// | `flags` | użyte mapy, odwrócenie V | 16 B |
/// | `surface` | stylizacja, anizotropia, SSS, rim | 16 B |
/// | `tint` | emisja RGB + mnożnik diffuse | 16 B |
/// | `accent` | grubość SSS + kolor SSS | 16 B |
/// | **suma** | | **80 B** |
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MaterialUniform {
    /// `x` = chropowatość bazowa, `y` = metaliczność bazowa,
    /// `z` = mnożnik UV, `w` = siła normal mappingu.
    pub params: [f32; 4],
    /// `x` = albedo mapa?, `y` = normal mapa?, `z` = ORM mapa?,
    /// `w` = odwrócenie skaliowej osi V (V do góry / do dołu).
    pub flags: [f32; 4],
    /// `x` = stylizacja 0..1 (rampa anime na dyfuzji),
    /// `y` = anizotropia -1..1 (rozciągnięcie połysku),
    /// `z` = siła SSS (podpowierzchniowe rozpraszanie),
    /// `w` = siła rim lightu.
    ///
    /// To jest „warstwa rysunkowa" materiału. Reszta zostaje w pełni
    /// PBR, więc metal na ramieniu nadal odbija otoczenie jak metal —
    /// to właśnie ten hybrydowy model, a nie cel-shading.
    pub surface: [f32; 4],
    /// `rgb` = emissive (światło własne), `w` = mnożnik jasności diffuse.
    ///
    /// `w` poniżej 1 przyciemnia bazę bez ruszania emisji — dzięki
    /// temu neonowa listwa na ścianie może świecić, będąc ciemną
    /// w rozproszonym świetle.
    pub tint: [f32; 4],
    /// `x` = grubość SSS (0 = cienka skóra, 1 = gruba),
    /// `y,z,w` = kolor SSS (zwykle przesunięty w czerwień).
    pub accent: [f32; 4],
    /// `rgb` = kolor podstawowy z `Surface::base`,
    /// `w` = 1 gdy kolor pochodzi z materiału, 0 gdy ma go użyć
    /// kolor wierzchołka.
    ///
    /// Ten ostatni bit jest najważniejszy. Importer `.obj` wkleja `Kd`
    /// do koloru KAŻDEGO wierzchołka, więc dla modeli z pliku bazą jest
    /// już wierzchołek — mnożenie przez `base` przyciemniłoby je
    /// podwójnie (`Kd · Kd`). Dla materiałów pisanych w kodzie
    /// (`add_surface`) wierzchołki bywają białe i bazą musi być
    /// `Surface::base`. Bez tego bitu nie da się obsłużyć obu dróg
    /// jednym polem.
    pub base: [f32; 4],
}

// 6 × `vec4` = 96 B. Weryfikacja kompilacyjna chroni kontrakt
// z `Material` w `s3d.wgsl`: zmiana jednej strony bez drugiej kończy
// się błędem walidacji wgpu dopiero w trakcie renderowania.
const _: () = assert!(std::mem::size_of::<MaterialUniform>() == 96);

/// Materiał gotowy do rysowania.
pub struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    // Trzymamy tekstury przy sobie: wgpu nie ma współdzielenia
    // zasobów, więc zwolnienie `wgpu::Texture` gwarantowałoby, że
    // bind group wskaże na nic.
    _albedo: wgpu::Texture,
    _normal: wgpu::Texture,
    _orm: wgpu::Texture,
}

/// Zestaw domyślnych tekstur dla materiałów bez plików.
///
/// Trzymamy je w [`MaterialBank`], bo wgpu nie pozwala współdzielić
/// jednej tekstury między bind grupy — każdy materiał dostaje swój
/// bind group, ale fizycznie dzieli te same obrazy.
pub struct MaterialBank {
    // `TextureView` NIE implementuje `Clone` w wgpu 0.19, a ten sam
    // widok musi trafić do bind group każdego materiału. Dlatego
    // trzymamy go za `Arc` zamiast kopiować.
    pub white: std::sync::Arc<wgpu::TextureView>,
    pub flat_normal: std::sync::Arc<wgpu::TextureView>,
    pub orm_default: std::sync::Arc<wgpu::TextureView>,
    pub sampler: wgpu::Sampler,
    pub layout: std::sync::Arc<wgpu::BindGroupLayout>,
    /// Materiały w kolejności ich identyfikatorów.
    pub materials: Vec<GpuMaterial>,
    /// Identyfikatory materiałów, których nie wolno usunąć.
    pub fallback: Option<MaterialId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialId(pub u32);

/// Opis powierzchni — komplet ustawień potrzebnych do stylu Endfield.
///
/// ## Po co to osobno od `import::Material`
///
/// `import::Material` opisuje to, co **wypisano w pliku `.mtl`**: kolor
/// dyfuzyjny, mapy, klasyczną chropowatość. Nie ma tam miejsca na
/// „rampa anime", SSS czy anizotropię, bo Wavefront tego nie przewiduje.
///
/// A sceny budowane w kodzie (cała reszta silnika) nie mają plików `.mtl`
/// w ogóle. `Surface` jest więc drogą bez plików: opisujemy materiał
/// kodem, a [`MaterialBank::add_surface`] buduje z niego uniform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    /// Kolor podstawowy podany w sRGB, zakres 0..1.
    ///
    /// Shader dekoduje go przed oświetleniem, tak jak teksturę albedo —
    /// dzięki temu kolory z kodu i z pliku wyglądają identycznie.
    pub base: [f32; 3],
    /// Chropowatość 0..1. Niska = lustro, wysoka = mat.
    pub roughness: f32,
    /// Metaliczność 0..1. 1 oznacza, że albedo jest kolorem odbicia.
    pub metallic: f32,
    /// Siła rampy anime 0..1. 0 = czyste PBR, 1 = mocne stopniowanie.
    pub stylize: f32,
    /// Anizotropia -1..1: rozciąga połysk wzdłuż kierunku szczotkowania.
    ///
    /// Włosy i szczotkowane metale dostają tu ~0.8, dzięki czemu połysk
    /// jest pasmem przesuniętym względem środka włókna, a nie okręgiem.
    pub anisotropy: f32,
    /// Siła podpowierzchniowego rozpraszania 0..1 (skóra, wosk).
    pub sss: f32,
    /// Grubość 0..1. Wyższa = światło wnika głębiej (grubsza skóra).
    pub sss_thickness: f32,
    /// Kolor rozpraszania podpowierzchniowego, zwykle przesunięty
    /// w czerwień — stąd uszy i nos „ciepleją" pod światło.
    pub sss_tint: [f32; 3],
    /// Siła światła krawędziowego 0..1.
    ///
    /// To ta charakterystyczna poświata wokół postaci. Uwaga: nie jest
    /// to contour — światło rzeczywiste ma granicę tam, gdzie kończy się
    /// normalna, a nie tam, gdzie kończy się obiekt.
    pub rim: f32,
    /// Światło własne (neon, ekran, laser), zakres 0..n.
    ///
    /// Podajemy HDR: wartość 4.0 dla lampy odda poświatę w bloomie.
    pub emissive: [f32; 3],
    /// Mnożnik jasności diffuse 0..n. Poniżej 1 przyciemnia bazę,
    /// nie ruszając emisji.
    pub diffuse_gain: f32,
    /// Powtarzalność UV (przeskalowanie współrzędnych tekstury).
    pub uv_scale: f32,
    /// Siła normal mappingu 0..1.
    pub normal_strength: f32,
    /// Czy bazą koloru jest wierzchołek (`true`), czy `base` (`false`).
    ///
    /// To rozróżnienie jest konieczne, bo istnieją DWA niezależne
    /// źródła koloru:
    ///
    /// | droga | źródło koloru | pole |
    /// |---|---|---|
    /// | import `.obj`/`.mtl` | importer wkleja `Kd` w każdy wierzchołek | `vertex_color` |
    /// | `add_surface` | `Surface::base` | `!vertex_color` |
    ///
    /// Gdyby obie mnożyły się nawzajem, model z pliku przyciemniłby się
    /// o `Kd²`, a programowy materiał byłby zignorowany. Dlatego
    /// wybieramy jedno źródło flagą, zamiast mnożyć oba.
    ///
    /// Ustaw `true` tylko wtedy, gdy świadomie chcesz, żeby kolor
    /// wierzchołków sterował materiałem (np. podłoga z szwami).
    pub vertex_color: bool,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            base: [0.8, 0.8, 0.8],
            // 0.6 = neutralne „malowane metaliczne", ani lustro, ani guma.
            // To samo, co zakłada `import::Material::fallback`, więc
            // materiał z kodu i z pliku mają spójny punkt wyjścia.
            roughness: 0.6,
            metallic: 0.0,
            // 0.25 to domyślnie: lekki rysunkowy charakter, ale wciąż
            // czytelne formy. Pełne 1.0 wygląda płasko na otoczeniu.
            stylize: 0.25,
            anisotropy: 0.0,
            sss: 0.0,
            sss_thickness: 0.3,
            sss_tint: [0.85, 0.25, 0.18],
            rim: 0.0,
            emissive: [0.0; 3],
            diffuse_gain: 1.0,
            uv_scale: 1.0,
            normal_strength: 1.0,
            // Domyślnie baza koloru to `base`, nie wierzchołek. To
            // właściwy wybór dla materiałów pisanych w kodzie: caller
            // nie musi pamiętać, żeby pomalować geometrię na biel.
            vertex_color: false,
        }
    }
}

impl Surface {
    /// Materiał metaliczny: pełne PBR, bez rampy anime.
    ///
    /// Podstawa pod otoczenie przemysłowe — metal ma być metalem
    /// w pełni fizycznym, inaczej wygląda jak płaska naklejka.
    pub fn metal(base: [f32; 3], roughness: f32) -> Self {
        Self {
            base,
            roughness,
            metallic: 1.0,
            stylize: 0.0,
            ..Self::default()
        }
    }

    /// Materiał tkaniny/skafandra: rysunkowy, z lekkim SSS na krawędziach.
    pub fn cloth(base: [f32; 3]) -> Self {
        Self {
            base,
            roughness: 0.85,
            stylize: 0.7,
            sss: 0.25,
            sss_thickness: 0.5,
            ..Self::default()
        }
    }

    /// Skóra: mocna rampa + podpowierzchniowe rozpraszanie.
    pub fn skin(base: [f32; 3]) -> Self {
        Self {
            base,
            roughness: 0.52,
            stylize: 0.85,
            sss: 1.0,
            sss_thickness: 0.3,
            rim: 0.35,
            ..Self::default()
        }
    }

    /// Włosy: anizotropia daje pasmowy, jedwabisty połysk.
    pub fn hair(base: [f32; 3]) -> Self {
        Self {
            base,
            roughness: 0.28,
            stylize: 0.8,
            anisotropy: 0.85,
            rim: 0.5,
            sss: 0.3,
            sss_thickness: 0.15,
            sss_tint: [0.95, 0.55, 0.35],
            ..Self::default()
        }
    }

    /// Emisja (listwy, ekrany, lasery) podana w HDR.
    pub fn emissive(color: [f32; 3], strength: f32) -> Self {
        Self {
            base: color,
            roughness: 0.4,
            emissive: [
                color[0] * strength,
                color[1] * strength,
                color[2] * strength,
            ],
            stylize: 0.0,
            ..Self::default()
        }
    }

    /// Materiał na `MaterialUniform` — wspólna ścieżka dla obu wejść
    /// (`add_surface` i `add` z plikiem `.mtl`).
    ///
    /// Które źródło koloru jest bazą, mówi samo `Surface::vertex_color`
    /// (patrz opis pola), więc podpis nie przekazuje tej decyzji
    /// osobno — inaczej wywołujący mógłby podać sprzeczne informacje.
    pub(crate) fn to_uniform(
        &self,
        albedo_map: bool,
        normal_map: bool,
        orm_map: bool,
    ) -> MaterialUniform {
        MaterialUniform {
            params: [
                self.roughness.clamp(0.02, 1.0),
                self.metallic.clamp(0.0, 1.0),
                self.uv_scale,
                if normal_map {
                    self.normal_strength
                } else {
                    0.0
                },
            ],
            flags: [
                if albedo_map { 1.0 } else { 0.0 },
                if normal_map { 1.0 } else { 0.0 },
                if orm_map { 1.0 } else { 0.0 },
                // V do góry: OpenGL/Blender ma V rosnące w górę, a
                // wgpu zapisuje tekstury od góry. Bez odwrócenia obraz
                // byłby do góry nogami.
                -1.0,
            ],
            surface: [
                self.stylize.clamp(0.0, 1.0),
                self.anisotropy.clamp(-1.0, 1.0),
                self.sss.clamp(0.0, 1.0),
                self.rim.clamp(0.0, 1.0),
            ],
            tint: [
                self.emissive[0],
                self.emissive[1],
                self.emissive[2],
                self.diffuse_gain.max(0.0),
            ],
            accent: [
                self.sss_thickness.clamp(0.0, 1.0),
                self.sss_tint[0],
                self.sss_tint[1],
                self.sss_tint[2],
            ],
            base: [
                self.base[0],
                self.base[1],
                self.base[2],
                // `w = 1` → shader bierze bazę z `base.rgb`;
                // `w = 0` → z koloru wierzchołka.
                if self.vertex_color { 0.0 } else { 1.0 },
            ],
        }
    }
}

/// Tekstura 1×1 w zadanym kolorze (uzywana jako "brak tekstury").
fn solid(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], srgb: bool) -> wgpu::Texture {
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Uran 3D tekstura domyslna"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Wczytuje PNG i tworzy z niego teksturę.
///
/// Zwraca `None`, gdy pliku nie ma lub jest uszkodzony — brakująca
/// tekstura nie może wywrócić importu modelu.
fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &std::path::Path,
    srgb: bool,
) -> Option<wgpu::Texture> {
    let img = uran_asset::Image::load_png(path).ok()?;
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return None;
    }
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let size = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Uran 3D tekstura materiału"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &img.data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        size,
    );
    Some(tex)
}

impl MaterialBank {
    /// Tworzy bank z zestawem tekstur domyślnych i materiałem zapasowym.
    ///
    /// Materiał zapasowy MUSI istnieć od razu: `DrawCmd` wskazuje
    /// materiał, a `Option<MaterialId>` w komendzie zamieniałby każde
    /// rysowanie w gałąź. Lepiej jeden wspólny materiał „biały bez map".
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran 3D Layout Materiału"),
            entries: &[
                // 0: albedo (kolor → sRGB, GPU zdekoduje do liniowego)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 1: normalna (wektor → liniowy!)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 2: ORM (AO / roughness / metaliczność → liniowy)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran 3D Sampler"),
            // Powtarzamy poza zakresem 0..1 — modele z Blendera często
            // mają UV wysuwające poza kwadrat, a bez tego krawędź
            // tekstury ciemnieje w miejscu, gdzie zaczyna się tiling.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Zastępcze: biały albedo, płaska normalna (0,5,0,5,1) i ORM
        // oznaczające „brak AO, chropowatość 1 (matowe), metal 0".
        let white = solid(device, queue, [255, 255, 255, 255], true);
        let flat_normal = solid(device, queue, [128, 128, 255, 255], false);
        let orm_default = solid(device, queue, [255, 255, 0, 255], false);

        let mut bank = Self {
            white: std::sync::Arc::new(white.create_view(&Default::default())),
            flat_normal: std::sync::Arc::new(flat_normal.create_view(&Default::default())),
            orm_default: std::sync::Arc::new(orm_default.create_view(&Default::default())),
            sampler,
            layout: std::sync::Arc::new(layout),
            materials: Vec::new(),
            fallback: None,
        };
        // Materiał 0 zawsze istnieje i zawsze jest biały bez map
        let id = bank.add(device, queue, &Material::fallback("default"));
        bank.fallback = Some(id);
        bank
    }

    /// Dodaje materiał z pliku `.mtl` i zwraca jego identyfikator.
    ///
    /// Materiały plikowe dostają tylko wartości, które Wavefront umie
    /// zapisać. Pozostałe pola [`MaterialUniform`] zostają zerowe, czyli
    /// `stylize = 0`, `sss = 0`, `rim = 0` — importowany model wygląda
    /// jak czyste PBR, dokładnie jak w edytorze.
    pub fn add(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, m: &Material) -> MaterialId {
        // Wszystkie ścieżki rozpatrujemy w stosunku do katalogu `.mtl`
        // (import już je złączył), a brak pliku nie jest błędem.
        let albedo_tex = m
            .albedo_map
            .as_deref()
            .and_then(|p| load_texture(device, queue, p, true));
        let normal_tex = m
            .normal_map
            .as_deref()
            .and_then(|p| load_texture(device, queue, p, false));
        // ORM: Blender w `.mtl` zapisuje je osobno (`map_Pr`, `map_Pm`),
        // a w glTF jako jedną paczkę. Bierzemy `albedo_map` tylko gdy
        // wskazuje na plik ORM — inaczej zostawiamy wartości stałe.
        let orm_tex: Option<wgpu::Texture> = None;

        // Kolor z pliku przechodzi przez tę samą ścieżkę co `Surface`,
        // więc importowany model i model z kodu mają identyczną bazę.
        let surface = Surface {
            base: m.albedo,
            roughness: m.roughness,
            metallic: m.metallic,
            // Materiał plikowy = czyste PBR. Stylizację ustawia gra przez
            // `add_surface`, bo `.mtl` o niej nie wie.
            stylize: 0.0,
            // `Kd` importer wkleił już do koloru każdego wierzchołka,
            // więc baza musi pochodzić stamtąd. Gdyby wzięła się z
            // `base`, kolor z pliku zostałby zignorowany.
            vertex_color: true,
            ..Surface::default()
        };
        let uniform = surface.to_uniform(
            albedo_tex.is_some(),
            normal_tex.is_some(),
            orm_tex.is_some(),
        );
        self.push_material(device, queue, uniform, albedo_tex, normal_tex, orm_tex)
    }

    /// Dodaje materiał opisany kodem (bez plików) i zwraca jego identyfikator.
    ///
    /// To droga dla scen generowanych w kodzie. Nie ma tu żadnych
    /// tekstur — materiał dostaje tekstury zastępcze 1×1, więc wizualnie
    /// bazuje na `Surface::base` i parametrach, a nie na plikach.
    pub fn add_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &Surface,
    ) -> MaterialId {
        // Wszystkie flagi map `false`: shader użyje koloru wierzchołka
        // i wartości stałych z uniformu.
        self.push_material(
            device,
            queue,
            surface.to_uniform(false, false, false),
            None,
            None,
            None,
        )
    }

    /// Wspólny koniec obu ścieżek: tworzy uniform, bind grupę i wpisuje
    /// materiał do banku.
    ///
    /// Wyciągamy to osobno, bo różni je wyłącznie komplet tekstur —
    /// reszta (layout, próbnik, trzymanie zasobów przy sobie) identyczna
    /// i rozjazdowała się przy każdej zmianie rozmiaru uniformu.
    #[allow(clippy::too_many_arguments)]
    fn push_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniform: MaterialUniform,
        albedo_tex: Option<wgpu::Texture>,
        normal_tex: Option<wgpu::Texture>,
        orm_tex: Option<wgpu::Texture>,
    ) -> MaterialId {
        let albedo_view = albedo_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.white));
        let normal_view = normal_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.flat_normal));
        let orm_view = orm_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.orm_default));

        let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uran 3D Uniform Materiału"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran 3D Bind Materiału"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&orm_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });

        let id = MaterialId(self.materials.len() as u32);
        self.materials.push(GpuMaterial {
            bind_group,
            uniform: ubuf,
            // Tekstury trzymamy przy sobie: wgpu nie współdzieli
            // zasobów, więc ich zwolnienie zostawiłoby bind group
            // wskazujący na nic.
            _albedo: albedo_tex.unwrap_or_else(|| self.white_placeholder(device, queue)),
            _normal: normal_tex.unwrap_or_else(|| self.normal_placeholder(device, queue)),
            _orm: orm_tex.unwrap_or_else(|| self.orm_placeholder(device, queue)),
        });
        id
    }

    fn white_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [255, 255, 255, 255], true)
    }
    fn normal_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [128, 128, 255, 255], false)
    }
    fn orm_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [255, 255, 0, 255], false)
    }

    /// Bind grupy materiału, albo `None` gdy identyfikator jest zły.
    pub fn bind_group(&self, id: MaterialId) -> Option<&wgpu::BindGroup> {
        self.materials.get(id.0 as usize).map(|m| &m.bind_group)
    }
}
