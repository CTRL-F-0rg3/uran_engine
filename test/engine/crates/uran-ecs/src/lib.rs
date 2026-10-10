//! Warstwa ECS oparta na `hecs`, z komponentami pod render 2D.

pub mod component;

pub use component::{BlendMode, Material, Mesh, Sprite, Transform, UvRect, Vertex, Visibility};

// `hecs` jest reeksportowane, żeby gry nie musiały dodawać zależności.
pub use hecs::{Entity, QueryBorrow, Ref, RefMut, World};
