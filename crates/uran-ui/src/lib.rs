//! UI w stylu **Canvas** (jak Unity): drzewo elementów z `RectTransform`
//! (anchor + pivot), obrazami, tekstem i przyciskami.
//!
//! # Jak to działa
//!
//! * [`Canvas`] to korzeń — drzewo [`Element`]ów rysowane w przestrzeni ekranu
//!   (HUD) albo w przestrzeni świata.
//! * [`RectTransform`] pozycjonuje element względem rodzica przez **anchor**
//!   (gdzie w rodzicu) i **pivot** (który punkt elementu), dokładnie jak w Unity.
//! * [`ElementKind`] mówi, co rysować: panel, obraz, tekst albo przycisk.
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! let tex = ctx.load_image("ui/panel.png");
//! let hud = Canvas::screen(
//!     Element::panel("tlo", RectTransform::center(Vec2::new(400.0, 300.0)),
//!                    Color::from_hex(0x1B1F2A).with_alpha(0.9))
//!         .with_child(Element::button("graj", RectTransform::center(Vec2::new(160.0, 48.0)),
//!                                    "Graj", 20.0, Color::from_hex(0x2A3446), Color::from_hex(0x3A4A66))),
//! );
//!
//! // w systemie rysowania:
//! ctx.ui(&hud);
//! ```

use uran_asset::{FontData, Handle, Image};
use uran_ecs::UvRect;
use uran_math::{Color, Rect, Vec2};
use uran_render::{Graphics, TextAlign};

/// Uchwyt tekstury.
pub type Texture = Handle<Image>;
/// Uchwyt czcionki.
pub type Font = Handle<FontData>;

/// Tryb renderowania canvasa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasMode {
    /// Overlay ekranu — niezależny od kamery (HUD, menu).
    Screen,
    /// W przestrzeni świata — podąża za kamerą i sceną.
    World,
}

/// Pozycja i rozmiar elementu — anchor + pivot (jak `RectTransform` w Unity).
///
/// * `anchor` (0..1) — punkt w **rodzicu**, do którego element jest przyczepiony:
///   `(0,0)` to róg `min`, `(1,1)` to róg `max`.
/// * `pivot` (0..1) — punkt w **elemencie**, który ląduje na anchorze.
/// * `size` — rozmiar elementu (piksele w trybie ekranu, jednostki w trybie świata).
/// * `offset` — dodatkowe przesunięcie od punktu anchora.
#[derive(Debug, Clone, Copy)]
pub struct RectTransform {
    pub anchor: Vec2,
    pub pivot: Vec2,
    pub size: Vec2,
    pub offset: Vec2,
}

impl RectTransform {
    /// Pełna kontrola (anchor, pivot, rozmiar, przesunięcie).
    pub const fn new(anchor: Vec2, pivot: Vec2, size: Vec2, offset: Vec2) -> Self {
        Self {
            anchor,
            pivot,
            size,
            offset,
        }
    }

    /// Środek rodzica.
    pub fn center(size: Vec2) -> Self {
        Self::new(Vec2::splat(0.5), Vec2::splat(0.5), size, Vec2::ZERO)
    }

    /// Lewy-górny róg rodzica.
    pub fn top_left(size: Vec2) -> Self {
        Self::new(Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0), size, Vec2::ZERO)
    }

    /// Prawy-górny róg rodzica.
    pub fn top_right(size: Vec2) -> Self {
        Self::new(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), size, Vec2::ZERO)
    }

    /// Lewy-dolny róg rodzica.
    pub fn bottom_left(size: Vec2) -> Self {
        Self::new(Vec2::ZERO, Vec2::ZERO, size, Vec2::ZERO)
    }

    /// Prawy-dolny róg rodzica.
    pub fn bottom_right(size: Vec2) -> Self {
        Self::new(Vec2::new(1.0, 0.0), Vec2::new(1.0, 0.0), size, Vec2::ZERO)
    }

    /// Środek górnej krawędzi.
    pub fn top_center(size: Vec2) -> Self {
        Self::new(Vec2::new(0.5, 1.0), Vec2::new(0.5, 1.0), size, Vec2::ZERO)
    }

    /// Środek dolnej krawędzi.
    pub fn bottom_center(size: Vec2) -> Self {
        Self::new(Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.0), size, Vec2::ZERO)
    }

    /// Z dodatkowym przesunięciem (od punktu anchora).
    pub fn offset(mut self, offset: Vec2) -> Self {
        self.offset = offset;
        self
    }
}

/// Rozwiązuje `RectTransform` do konkretnego prostokąta w obrębie `parent`.
pub fn resolve(transform: &RectTransform, parent: Rect) -> Rect {
    let anchor_point = parent.min + transform.anchor * parent.size();
    let pivot_offset = transform.size * transform.pivot;
    let origin = anchor_point + transform.offset - pivot_offset;
    Rect::from_xywh(origin.x, origin.y, transform.size.x, transform.size.y)
}

/// Rodzaj elementu UI — co narysować.
#[derive(Debug, Clone)]
pub enum ElementKind {
    /// Kolorowe tło / panel.
    Panel {
        color: Color,
    },
    /// Obraz (tekstura + region UV + tint).
    Image {
        texture: Texture,
        uv: UvRect,
        color: Color,
    },
    /// Tekst.
    Text {
        text: String,
        font_size: f32,
        color: Color,
        align: TextAlign,
    },
    /// Przycisk: tło + etykieta. `hovered` decyduje o kolorze przy rysowaniu.
    Button {
        color: Color,
        hover_color: Color,
        label: String,
        font_size: f32,
        hovered: bool,
    },
}

/// Węzeł canvasa — element z transformem, typem i (opcjonalnie) dziećmi.
#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub transform: RectTransform,
    pub kind: ElementKind,
    pub visible: bool,
    pub children: Vec<Element>,
}

impl Element {
    pub fn new(name: impl Into<String>, transform: RectTransform, kind: ElementKind) -> Self {
        Self {
            name: name.into(),
            transform,
            kind,
            visible: true,
            children: Vec::new(),
        }
    }

    /// Panel (kolorowe tło).
    pub fn panel(name: impl Into<String>, transform: RectTransform, color: Color) -> Self {
        Self::new(name, transform, ElementKind::Panel { color })
    }

    /// Obraz z pełnym regionem UV i białym tintem.
    pub fn image(name: impl Into<String>, transform: RectTransform, texture: Texture) -> Self {
        Self::new(
            name,
            transform,
            ElementKind::Image {
                texture,
                uv: UvRect::FULL,
                color: Color::WHITE,
            },
        )
    }

    /// Obraz z regionem UV i tintem.
    pub fn image_uv(
        name: impl Into<String>,
        transform: RectTransform,
        texture: Texture,
        uv: UvRect,
        color: Color,
    ) -> Self {
        Self::new(name, transform, ElementKind::Image { texture, uv, color })
    }

    /// Tekst (wyśrodkowany).
    pub fn text(
        name: impl Into<String>,
        transform: RectTransform,
        text: impl Into<String>,
        font_size: f32,
        color: Color,
    ) -> Self {
        Self::new(
            name,
            transform,
            ElementKind::Text {
                text: text.into(),
                font_size,
                color,
                align: TextAlign::Center,
            },
        )
    }

    /// Przycisk: tło + etykieta.
    pub fn button(
        name: impl Into<String>,
        transform: RectTransform,
        label: impl Into<String>,
        font_size: f32,
        color: Color,
        hover_color: Color,
    ) -> Self {
        Self::new(
            name,
            transform,
            ElementKind::Button {
                color,
                hover_color,
                label: label.into(),
                font_size,
                hovered: false,
            },
        )
    }

    /// Dodaje dziecko.
    pub fn with_child(mut self, child: Element) -> Self {
        self.children.push(child);
        self
    }

    /// Dodaje wiele dzieci.
    pub fn with_children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children.extend(children);
        self
    }

    /// Ustawia widoczność.
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

/// Korzeń UI — drzewo elementów + tryb renderowania.
#[derive(Debug, Clone)]
pub struct Canvas {
    /// Tryb: ekran (HUD) albo świat.
    pub mode: CanvasMode,
    /// Kolor tła całego canvasa (rysowany pod wszystkim).
    pub background: Option<Color>,
    /// Warstwa sortowania (domyślnie 2000 — nad światem).
    pub layer: i32,
    /// Korzeń drzewa.
    pub root: Element,
}

impl Canvas {
    /// Canvas w przestrzeni ekranu (HUD).
    pub fn screen(root: Element) -> Self {
        Self {
            mode: CanvasMode::Screen,
            background: None,
            layer: 2000,
            root,
        }
    }

    /// Canvas w przestrzeni świata (podąża za kamerą).
    pub fn world(root: Element) -> Self {
        Self {
            mode: CanvasMode::World,
            background: None,
            layer: 2000,
            root,
        }
    }

    /// Ustawia kolor tła.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Ustawia warstwę sortowania.
    pub fn layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    /// Rysuje canvas. `root` to prostokąt, który canvas wypełnia:
    /// * tryb ekranu — cały viewport (wywołaj po `gfx.screen_space()`),
    /// * tryb świata — widoczny obszar kamery.
    pub fn draw(&self, gfx: &mut Graphics<'_>, font: Option<Font>, root: Rect) {
        if let Some(bg) = self.background {
            gfx.layer(self.layer).color(bg).draw_rect(root);
        }
        self.draw_element(&self.root, root, gfx, font);
    }

    fn draw_element(&self, el: &Element, parent: Rect, gfx: &mut Graphics<'_>, font: Option<Font>) {
        if !el.visible {
            return;
        }
        let rect = resolve(&el.transform, parent);
        match &el.kind {
            ElementKind::Panel { color } => {
                gfx.layer(self.layer).color(*color).draw_rect(rect);
            }
            ElementKind::Image { texture, uv, color } => {
                gfx.layer(self.layer)
                    .color(*color)
                    .draw_texture(*texture, rect, *uv);
            }
            ElementKind::Text {
                text,
                font_size,
                color,
                align,
            } => {
                if let Some(font) = font {
                    let pos = match align {
                        TextAlign::Left => Vec2::new(rect.left(), rect.center().y),
                        TextAlign::Center => rect.center(),
                        TextAlign::Right => Vec2::new(rect.right(), rect.center().y),
                    };
                    gfx.layer(self.layer)
                        .color(*color)
                        .draw_text(font, text, pos, *font_size, *align);
                }
            }
            ElementKind::Button {
                color,
                hover_color,
                label,
                font_size,
                hovered,
            } => {
                let bg = if *hovered { *hover_color } else { *color };
                gfx.layer(self.layer).color(bg).draw_rect(rect);
                if let Some(font) = font {
                    gfx.layer(self.layer)
                        .color(Color::WHITE)
                        .draw_text(font, label, rect.center(), *font_size, TextAlign::Center);
                }
            }
        }
        for child in &el.children {
            self.draw_element(child, rect, gfx, font);
        }
    }

    /// Rozwiązuje drzewo do płaskiej listy `(nazwa, prostokąt)`.
    pub fn layout(&self, root: Rect) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        self.collect(&self.root, root, &mut out);
        out
    }

    fn collect(&self, el: &Element, parent: Rect, out: &mut Vec<(String, Rect)>) {
        if !el.visible {
            return;
        }
        let rect = resolve(&el.transform, parent);
        out.push((el.name.clone(), rect));
        for child in &el.children {
            self.collect(child, rect, out);
        }
    }

    /// Zwraca nazwę **najwyższego** elementu, który zawiera `point`.
    pub fn hit(&self, root: Rect, point: Vec2) -> Option<String> {
        let mut flat = self.layout(root);
        flat.reverse();
        flat.into_iter()
            .find(|(_, r)| r.contains(point))
            .map(|(name, _)| name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Element wyśrodkowany ląduje na środku rodzica.
    #[test]
    fn center_resolves_to_middle() {
        let t = RectTransform::center(Vec2::new(40.0, 20.0));
        let r = resolve(&t, Rect::from_xywh(0.0, 0.0, 100.0, 100.0));
        assert_eq!(r, Rect::from_xywh(30.0, 40.0, 40.0, 20.0));
    }

    /// top_left przykleja element do lewego-górnego rogu rodzica.
    #[test]
    fn top_left_resolves_to_top_left_corner() {
        let t = RectTransform::top_left(Vec2::new(40.0, 20.0));
        let r = resolve(&t, Rect::from_xywh(0.0, 0.0, 100.0, 100.0));
        assert_eq!(r.min, Vec2::new(0.0, 80.0));
        assert_eq!(r.max, Vec2::new(40.0, 100.0));
    }

    /// offset przesuwa element względem punktu anchora.
    #[test]
    fn offset_shifts_from_anchor() {
        let t = RectTransform::center(Vec2::new(10.0, 10.0)).offset(Vec2::new(5.0, -5.0));
        let r = resolve(&t, Rect::from_xywh(0.0, 0.0, 100.0, 100.0));
        assert_eq!(r.center(), Vec2::new(55.0, 45.0));
    }

    /// layout zbiera nazwy i prostokąty, a hit wskazuje najwyższy element.
    #[test]
    fn layout_and_hit_work() {
        let canvas = Canvas::screen(
            Element::panel(
                "panel",
                RectTransform::center(Vec2::new(100.0, 100.0)),
                Color::WHITE,
            )
            .with_child(Element::button(
                "btn",
                RectTransform::center(Vec2::new(40.0, 20.0)),
                "ok",
                16.0,
                Color::WHITE,
                Color::WHITE,
            )),
        );
        let root = Rect::from_xywh(0.0, 0.0, 200.0, 200.0);
        assert_eq!(canvas.layout(root).len(), 2);
        assert_eq!(canvas.hit(root, Vec2::new(100.0, 100.0)), Some("btn".to_string()));
        assert_eq!(canvas.hit(root, Vec2::new(60.0, 60.0)), Some("panel".to_string()));
    }
}



