//! Rdzeń silnika: okno, czas i wejście z urządzeń.
//!
//! Warstwa `uran-core` nie zna nic o renderze ani ECS — to cel, żeby dało się
//! ją testować w izolacji.

pub mod input;
pub mod time;
pub mod window;

pub use input::{Input, Key, Modifiers, Mouse};
pub use time::Time;
pub use window::{windowed, WindowDescriptor};
