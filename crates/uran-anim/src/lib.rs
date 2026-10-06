//! Animacja 2D dla silnika Uran.
//!
//! Moduł składa się z pięciu warstw — każda z osobna ma sens, bo gry
//! używają ich niezależnie:
//!
//! | typ | po co jest |
//! |-----|-----------|
//! | [`SpriteSheet`] | siatka klatek w teksturze (klatka może być prostokątna) |
//! | [`AnimationClip`] / [`Animator`] | *która* klatka jest aktualna i jak przesuwa się w czasie |
//! | [`Facing`] | cztery kierunki odpowiadające czterem wierszom arkusza |
//! | [`Character`] | pozycja + ruch + **automatyczny** dobór animacji do ruchu |
//! | [`SpriteEffect`] | animacja **jednorazowa**: zaklęcie, pocisk, eksplozja |
//!
//! ## Dlaczego `Character`, a nie tylko `Animator`
//!
//! Najczęstszy błąd w animacji postaci to rozjazd: postać idzie, a sprite
//! pokazuje bieg albo stoi. Tu dzieje się to automatycznie — `Character`
//! bierze **wektor wejścia** i sam ustala stan (`Idle`/`Walk`/`Run`)
//! oraz kierunek, czyli wiersz arkusza. Gra nie musi pamiętać, że
//! „w lewo to wiersz 1".
//!
//! ## Przykład
//!
//! ```ignore
//! use uran_anim::prelude::*;
//!
//! // raz przy starcie:
//! let idle = ctx.load_image("player/64X128_Idle_Free.png").unwrap();
//! let walk = ctx.load_image("player/64X128_Walking_Free.png").unwrap();
//! let run  = ctx.load_image("player/64X128_Runing_Free.png").unwrap();
//! let sheets = CharacterSheets {
//!     idle: SpriteSheet::new(idle, Vec2::new(64.0, 128.0), size_of(idle)),
//!     walk: SpriteSheet::new(walk, Vec2::new(64.0, 128.0), size_of(walk)),
//!     run:  SpriteSheet::new(run,  Vec2::new(64.0, 128.0), size_of(run)),
//! };
//!
//! // w systemie gry:
//! let input = Vec2::new(
//!     ctx.input.axis(Key::KeyA, Key::KeyD),
//!     ctx.input.axis(Key::KeyS, Key::KeyW),
//! );
//! player.update(ctx.dt(), input, ctx.input.shift(), bounds, config);
//!
//! // w systemie rysowania:
//! player.draw(&mut ctx.gfx, &sheets, config);
//! ```

pub mod character;
pub mod clip;
pub mod effect;
pub mod facing;
pub mod sheet;

pub use character::{Character, CharacterConfig, CharacterSheets, MotionState};
pub use clip::{AnimationClip, Animator, LoopMode, Playback};
pub use effect::{EffectConfig, SpriteEffect};
pub use facing::{Facing, DEFAULT_DEADZONE};
pub use sheet::SpriteSheet;

/// Wszystko, czego typowo potrzebuje gra korzystająca z animacji 2D.
pub mod prelude {
    pub use crate::character::{Character, CharacterConfig, CharacterSheets, MotionState};
    pub use crate::clip::{AnimationClip, Animator, LoopMode, Playback};
    pub use crate::effect::{EffectConfig, SpriteEffect};
    pub use crate::facing::{Facing, DEFAULT_DEADZONE};
    pub use crate::sheet::SpriteSheet;
}

/// Wersja modułu (do logów i diagnostyki).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
