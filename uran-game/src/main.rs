use uran_engine::prelude::*;
use uran_ecs::Transform;
use uran_render::MeshBuilder;
use uran_math::{Vec3, Color};

fn main() {
    let mut app = App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Engine - Fluent Mesh API")
                .resizable(true)
                .background(0x000000) // Całkowicie czarne tło
        );

    // Trójkąt w rozsądnym rozmiarze (współrzędne NDC mieszczą się w -1..1),
    // wyśrodkowany na ekranie.
    let (mesh, material, visibility) = MeshBuilder::triangle(0.0, 0.0, 0.0, 0.5)
        .color(Color::from_hex(0xFFAA00))
        .visibility(true)
        .is_shaded(false)
        .build();

    // --- ZMIANA: Pozycja na środku ekranu ---
    app.world.spawn((
        Transform {
            translation: Vec3::new(0.0, 0.0, 0.0), // Zmieniono z -0.8 na 0.0 (środek)
            rotation: 0.0,
            scale: Vec3::ONE,
        },
        mesh,
        material,
        visibility,
    ));

    println!("Stworzono trójkąt z Fluent API!");
    app.run();
}