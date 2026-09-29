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
    // Lambert: ujemny diff oznacza ścianę odwróconą od światła
    let ndotl = max(dot(n, l), 0.0);
    let diffuse = scene.light_color.rgb * ndotl;

    // Blinn-Phong zamiast Phonga: połówka wektora jest tańsza i daje
    // ostrzejski, mniej rozmyty refleks
    let h = normalize(l + v);
    let ndoth = max(dot(n, h), 0.0);
    // wykładnik 48 daje wąski, metaliczny połysk
    let spec = pow(ndoth, 48.0) * 0.35;

    // ambient podnosi czarne ściany do poziomu otoczenia, inaczej bryły
    // wyglądają jak wycięte z papieru
    var lit = in.color * (scene.ambient_time.rgb + diffuse);

    // mgła: dalekie obiekty zlewają się z tłem, co daje poczucie skali
    let dist = length(scene.eye.xyz - in.world_pos);
    let fog = 1.0 - clamp((dist - 60.0) / 260.0, 0.0, 0.75);
    let sky = vec3<f32>(0.45, 0.55, 0.68);
    lit = mix(sky * 0.5, lit, fog);

    return vec4<f32>(lit + spec, 1.0);
}