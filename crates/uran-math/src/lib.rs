//! Warstwa matematyczna silnika: wektory (glam), kolory i prostokąty.

pub mod color;
pub mod rect;

pub use color::Color;
pub use rect::Rect;

pub use glam::{
    EulerRot, IVec2, Mat2, Mat3, Mat4, Quat, UVec2, Vec2, Vec3, Vec4,
};
