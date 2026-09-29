// Shader 3D — oświetlenie per-werthołek, bez tekstur.
//
// Celowo nie używamy instancjonowania w shaderze: transformację obiektu
// dostajemy z osobnego bufora `models` przez `first_instance`. Dzięki temu
// każdy obiekt ma własną macierz, a my nie mnożymy draw calli.

struct Scene {
    // Uważaj na wyrównanie w WGSL: samo `vec3<f32>` zajmuje 12 B, ale
    // następna zmienna musi zaczynać się od wielokrotności 16 — przez to
    // struktura z samymi `vec3` urosłaby do 160 B, a `SceneUniform` po
    // stronie Rust ma 128 B i walidacja wgpu odrzuciłaby bufor.
    //
    // Dlatego WSZĘDZIE są `vec4`: 64 (macierz) + 4 * 16 = 128 B.
    view_proj: mat4x4<f32>,   // 64 B
    eye: vec4<f32>,           // xyz = pozycja oka, w = nieużywane
    light_dir: vec4<f32>,     // xyz = kierunek światła, w = nieużywane
    light_color: vec4<f32>,   // rgb = kolor światła, a = nieużywane
    // rgb = ambient, a = czas świata
    ambient_time: vec4<f32>,
}

struct Model {
    // mat4x4 + vec4 = 80 B
    model: mat4x4<f32>,
    tint: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<storage, read> models: array<Model>;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // normalna w przestrzeni świata
    @location(0) world_normal: vec3<f32>,
    // pozycja w przestrzeni świata (do specular i mgły)
    @location(1) world_pos: vec3<f32>,
    // kolor wierzchołka przemnożony przez tint obiektu
    @location(2) color: vec3<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // `instance_index` liczy od 1, bo `first_instance` to 1
    @builtin(instance_index) inst: u32,
) -> VsOut {
    let m = models[inst - 1u];
    let world = m.model * vec4<f32>(position, 1.0);

    var out: VsOut;
    // macierz modelu jest czysto rotacyjno-skalowa (bez shear), więc
    // wystarczy przemnożyć normalną przez tę samą macierz i znormalizować
    out.clip_pos = scene.view_proj * world;
    out.world_normal = (m.model * vec4<f32>(normal, 0.0)).xyz;
    out.world_pos = world.xyz;
    out.color = color * m.tint.rgb;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let l = normalize(scene.light_dir.xyz);
    let v = normalize(scene.eye.xyz - in.world_pos);
    let ndotl = max(dot(n, l), 0.0);

    // --- światło
    // Barwy są podawane w sRGB (tak, jak je czyta oko), a mnożenie
    // i odbicie wykonujemy w LINIOWYM. Bez tej konwersji kolor (0.34, 0.38,
    // 0.24) potraktowany jak liniowy wypada po gamma-encodingu blado i
    // wypłukany — właśnie dlatego teren wyglądał jak trawa w rozcieńczalniku.
    let albedo = srgb_to_linear(in.color);
    let sun = srgb_to_linear(scene.light_color.rgb);

    let diffuse = sun * ndotl;

    // Blinn-Phong zamiast Phonga: połówka wektora jest tańsza i daje
    // ostrzejszy, mniej rozmyty refleks
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), 64.0) * 0.25;

    // --- ambient typu hemisfera: niebo z góry, ciemne odbicie ziemi z dołu.
    // Płaskie ambient wygląda, jakby wszystkie ściany dostały to samo
    // światło; hemisfera daje wrażenie otoczenia.
    let sky = srgb_to_linear(scene.ambient_time.rgb);
    let hemi = mix(sky * 0.55, sky * 1.35, n.y * 0.5 + 0.5);

    // --- ekspozycja
    // Oświetlenie liczymy w liniowym, gdzie 1.0 to „biały" i wartości
    // rzadko tam docierają. Bez mnożnika kadr wychodzi ciemny mimo
    // poprawnych kolorów. 1.35 to empirycznie: trawa wygląda jak trawa,
    // a jasne ściany czołgów nie przechodzą w biel.
    let exposure = 1.35;
    var lit = albedo * (hemi + diffuse) * exposure + vec3<f32>(spec);

    // --- światło wsteczne: rozjaśnia krawędzie brył skierowanych do
    // kamery, dzięki czemu kształt pozostaje czytelny mimo cienia
    let rim = pow(1.0 - max(dot(n, v), 0.0), 3.5) * 0.10;
    lit = lit + albedo * rim * exposure;

    // --- mgła: dalekie obiekty zlewają się z niebem, co daje skalę
    let dist = length(scene.eye.xyz - in.world_pos);
    let fog = clamp((dist - 45.0) / 200.0, 0.0, 0.85);
    // kolmgły bierzemy z gradientu nieba (taki sam, co rysuje `clear`)
    let fog_col = vec3<f32>(0.36, 0.50, 0.70);
    lit = mix(lit, fog_col, fog);

    return vec4<f32>(lit, 1.0);
}

/// sRGB -> liniowy, zgodną z krówką aproksymacji.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}