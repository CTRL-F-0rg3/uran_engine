pub use uran_core as core;
pub use uran_math as math;
pub use uran_render as render;

pub mod app;

pub mod prelude {
    pub use crate::app::App;
    pub use crate::core::{windowed, WindowDescriptor};
    pub use crate::math::Color;
}