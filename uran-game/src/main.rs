use uran_engine::prelude::*;
use uran_ecs::{Transform, Sprite};
use uran_math::Vec3;

fn main() {
    let mut app = App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Engine - ECS Test")
                .resizable(true)
                .background(0x1A1A2E) // Ciemny granat, żeby było widać ruch
        );

    // 1. Tworzymy encję (np. przyszły obrazek)
    // Na razie ma tylko Transform i marker Sprite
    let my_entity = app.world.spawn((
        Transform {
            translation: Vec3::new(-5.0, 0.0, 0.0), // Zaczyna po lewej stronie
            rotation: 0.0,
            scale: Vec3::new(1.0, 1.0, 1.0),
        },
        Sprite, // Marker, że to jest obiekt graficzny
    ));

    println!("Stworzono encję o ID: {:?}", my_entity);

    // 2. Uruchamiamy silnik
    app.run();
}