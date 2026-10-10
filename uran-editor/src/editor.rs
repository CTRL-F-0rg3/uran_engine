//! Stan i logika edytora map kafli.
//!
//! Edytor operuje na [`TileMap`] silnika: lewy panel to paleta kafli
//! (klik = wybór kafla), widok to mapa z siatką, prawy panel to warstwy.
//! Rysowanie: LPM stawia kafel, PPM usuwa, ŚPM / Spacja + LPM przesuwa
//! widok, kółko myszy robi zoom.

use uran_engine::prelude::*;
use uran_engine::tilemap::TILE_EMPTY;

/// Rozmiar kafla w palecie (piksele ekranu).
const PALETTE_CELL: f32 = 28.0;
/// Lewy górny róg palety (przestrzeń ekranu).
const PALETTE_ORIGIN: Vec2 = Vec2::new(12.0, 44.0);

/// Czcionki sprawdzane w kolejności, gdy brak własnej.
const FONT_FALLBACKS: &[&str] = &[
    "assets/fonts/UranSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];

/// Konfiguracja startowa edytora (z linii poleceń).
pub struct EditorConfig {
    /// Ścieżka zapisu/odczytu mapy (Ctrl+S / Ctrl+O).
    pub map_path: String,
    /// Szerokość nowej mapy (w kaflach).
    pub width: u32,
    /// Wysokość nowej mapy (w kaflach).
    pub height: u32,
    /// Rozmiar kafla w jednostkach świata.
    pub tile_size: f32,
    /// Mapa do wczytania przy starcie (None = nowa mapa).
    pub load_map: Option<String>,
    /// Arkusz kafli do dodania przy starcie: (nazwa, ścieżka PNG, rozmiar kafla).
    pub tileset: Option<(String, String, u32)>,
}

/// Stan edytora.
pub struct Editor {
    pub map: TileMap,
    pub map_path: String,

    font: Option<Font>,
    font_tried: bool,
    inited: bool,

    // paleta kafli
    tileset_idx: usize,
    selected_tile: u32,

    // warstwy
    active_layer: usize,

    // kamera
    cam_pos: Vec2,
    cam_zoom: f32,

    // interakcja
    panning: bool,
    last_mouse: Vec2,
    show_grid: bool,

    // jednorazowe akcje startowe
    load_map: Option<String>,
    pending_tileset: Option<(String, String, u32)>,

    pub status: String,
}

impl Editor {
    pub fn new(cfg: EditorConfig) -> Self {
        Self {
            map: TileMap::new("untitled", cfg.tile_size, cfg.width, cfg.height),
            map_path: cfg.map_path,
            font: None,
            font_tried: false,
            inited: false,
            tileset_idx: 0,
            selected_tile: 0,
            active_layer: 0,
            cam_pos: Vec2::ZERO,
            cam_zoom: 1.0,
            panning: false,
            last_mouse: Vec2::ZERO,
            show_grid: true,
            load_map: cfg.load_map,
            pending_tileset: cfg.tileset,
            status: "gotowy".to_string(),
        }
    }

    fn ensure_init(&mut self, ctx: &mut Ctx) {
        if self.inited {
            return;
        }
        self.inited = true;

        if let Some(path) = self.load_map.take() {
            if let Some(m) = ctx.load_tilemap(&path) {
                self.map = m;
                self.map_path = path.clone();
                self.status = format!("wczytano: {path}");
            } else {
                self.status = format!("nie udało się wczytać mapy `{path}`");
            }
        }

        if let Some((name, src, tile_px)) = self.pending_tileset.take() {
            match self.map.add_tileset_from_image(&name, ctx.assets, &src, tile_px, 0, 0) {
                Ok(()) => self.status = format!("dodano arkusz `{name}` ({src})"),
                Err(e) => self.status = format!("błąd arkusza `{src}`: {e}"),
            }
        }

        if self.map.layers.is_empty() {
            self.add_layer();
        }
        self.center_camera(ctx);
    }

    fn center_camera(&mut self, ctx: &mut Ctx) {
        let world = self.map.world_rect();
        if !world.is_empty() {
            ctx.camera.fit_world(world, ctx.window.size);
            self.cam_pos = ctx.camera.position;
            self.cam_zoom = ctx.camera.zoom;
        }
    }

    fn font(&mut self, ctx: &mut Ctx) -> Option<Font> {
        if !self.font_tried {
            self.font_tried = true;
            self.font = ctx.load_font("assets/fonts/UranSans.ttf");
            if self.font.is_none() {
                for path in FONT_FALLBACKS {
                    if let Ok(handle) = ctx.assets.load_font(path) {
                        self.font = Some(handle);
                        break;
                    }
                }
            }
        }
        self.font
    }

    pub fn update(&mut self, ctx: &mut Ctx) {
        self.ensure_init(ctx);
        self.font(ctx);

        ctx.camera.position = self.cam_pos;
        ctx.camera.zoom = self.cam_zoom;
        ctx.clear_color = Color::from_hex(0x131722);

        let mouse = ctx.input.mouse_position();
        let wheel = ctx.input.scroll_delta().y;

        // zoom kółkiem
        if wheel != 0.0 {
            let factor = 1.1f32.powf(wheel);
            self.cam_zoom = (self.cam_zoom * factor).clamp(0.05, 32.0);
            ctx.camera.zoom = self.cam_zoom;
        }

        // pan: środkowy przycisk albo Spacja + LPM
        let pan_button = ctx.input.mouse_pressed(MouseButton::Middle);
        let space_pan = ctx.input.pressed(Key::Space) && ctx.input.mouse_pressed(MouseButton::Left);
        if pan_button || space_pan {
            if !self.panning {
                self.panning = true;
                self.last_mouse = mouse;
            } else {
                let d = mouse - self.last_mouse;
                self.last_mouse = mouse;
                self.cam_pos -= d / self.cam_zoom;
            }
        } else {
            self.panning = false;
        }

        if !self.panning {
            self.handle_shortcuts(ctx);
            self.handle_paint(ctx);
        }

        ctx.camera.position = self.cam_pos;
        ctx.camera.zoom = self.cam_zoom;
    }

    fn handle_shortcuts(&mut self, ctx: &mut Ctx) {
        // Czytamy całe wejście najpierw, żeby nie trzymać pożyczki `ctx.input`
        // w trakcie wywołań mutujących `ctx` (`load`, `toggle_solid`).
        let esc = ctx.input.just_pressed(Key::Escape);
        let ctrl = ctx.input.control();
        let save = ctrl && ctx.input.just_pressed(Key::KeyS);
        let load = ctrl && ctx.input.just_pressed(Key::KeyO);
        let grid = ctx.input.just_pressed(Key::KeyG);
        let solid = ctx.input.just_pressed(Key::KeyF);
        let new_layer = ctx.input.just_pressed(Key::KeyN);
        let hide = ctx.input.just_pressed(Key::KeyH);
        let prev_layer = ctx.input.just_pressed(Key::BracketLeft);
        let next_layer = ctx.input.just_pressed(Key::BracketRight);
        let prev_tile = ctx.input.just_pressed(Key::KeyQ);
        let next_tile = ctx.input.just_pressed(Key::KeyE);
        let cycle_ts = ctx.input.just_pressed(Key::Tab);

        if esc {
            std::process::exit(0);
        }
        if save {
            self.save();
        }
        if load {
            self.load(ctx);
        }
        if grid {
            self.show_grid = !self.show_grid;
            self.status = if self.show_grid { "siatka: włączona" } else { "siatka: wyłączona" }.to_string();
        }
        if solid {
            self.toggle_solid(ctx);
        }
        if new_layer {
            self.add_layer();
        }
        if hide {
            if let Some(l) = self.map.layers.get_mut(self.active_layer) {
                l.hidden = !l.hidden;
                self.status = format!("warstwa `{}`: {}", l.tiles.name, if l.hidden { "ukryta" } else { "widoczna" });
            }
        }
        if prev_layer {
            self.select_layer(self.active_layer.saturating_sub(1));
        }
        if next_layer {
            self.select_layer((self.active_layer + 1).min(self.map.layers.len().saturating_sub(1)));
        }
        if prev_tile {
            self.selected_tile = self.selected_tile.saturating_sub(1);
        }
        if next_tile {
            self.selected_tile = self.selected_tile.saturating_add(1);
        }
        if cycle_ts {
            self.cycle_tileset();
        }
    }

    fn handle_paint(&mut self, ctx: &mut Ctx) {
        let mouse = ctx.input.mouse_position();

        // klik w palecie wybiera kafel, a nie maluje na mapie
        if self.over_palette(ctx) {
            if ctx.input.mouse_just_pressed(MouseButton::Left) {
                self.select_tile_at_screen(mouse);
            }
            return;
        }

        let world = ctx.mouse_world();
        let (cx, cy) = self.map.world_to_cell(world);
        if !self.map.in_bounds(cx, cy) {
            return;
        }
        if ctx.input.mouse_just_pressed(MouseButton::Left) {
            self.paint(cx, cy, false);
        }
        if ctx.input.mouse_just_pressed(MouseButton::Right) {
            self.paint(cx, cy, true);
        }
    }

    fn paint(&mut self, x: i32, y: i32, erase: bool) {
        let Some(layer) = self.map.layers.get_mut(self.active_layer) else {
            return;
        };
        if !layer.tiles.in_bounds(x, y) {
            return;
        }
        let value = if erase { TILE_EMPTY } else { self.selected_tile };
        layer.tiles.set(x, y, value);
    }

    fn toggle_solid(&mut self, ctx: &mut Ctx) {
        let world = ctx.mouse_world();
        let (cx, cy) = self.map.world_to_cell(world);
        if !self.map.in_bounds(cx, cy) {
            return;
        }
        let props = self.map.props_at(self.active_layer, cx, cy);
        let next = if props.is_solid() { TileProps::EMPTY } else { TileProps::SOLID };
        self.map.set_props(self.active_layer, cx, cy, next);
        self.status = format!("kafel ({cx},{cy}) solid: {}", if next.is_solid() { "tak" } else { "nie" });
    }

    fn select_layer(&mut self, idx: usize) {
        if idx < self.map.layers.len() {
            self.active_layer = idx;
            let name = self.map.layers[idx].tiles.name.clone();
            self.status = format!("warstwa: {name}");
        }
    }

    fn add_layer(&mut self) {
        let tileset = self.active_tileset_name().unwrap_or_else(|| "tiles".to_string());
        let name = format!("layer{}", self.map.layers.len() + 1);
        let layer = Layer::new(&name, &tileset, self.map.width(), self.map.height());
        let idx = self.map.add_layer(layer);
        self.active_layer = idx;
        self.status = format!("dodano warstwę `{name}`");
    }

    fn tileset_names(&self) -> Vec<String> {
        self.map.tilesets.keys().cloned().collect()
    }

    fn active_tileset_name(&self) -> Option<String> {
        self.tileset_names().get(self.tileset_idx).cloned()
    }

    fn active_tileset(&self) -> Option<&Tileset> {
        self.active_tileset_name().and_then(|n| self.map.tilesets.get(&n))
    }

    fn cycle_tileset(&mut self) {
        let n = self.map.tilesets.len();
        if n == 0 {
            return;
        }
        self.tileset_idx = (self.tileset_idx + 1) % n;
        self.selected_tile = 0;
        self.status = format!("arkusz: {}", self.active_tileset_name().unwrap_or_default());
    }

    fn palette_rect(&self) -> Option<Rect> {
        let ts = self.active_tileset()?;
        Some(Rect::from_xywh(
            PALETTE_ORIGIN.x - 4.0,
            PALETTE_ORIGIN.y - 4.0,
            ts.cols() as f32 * PALETTE_CELL + 8.0,
            ts.rows() as f32 * PALETTE_CELL + 8.0,
        ))
    }

    fn over_palette(&self, ctx: &Ctx) -> bool {
        self.palette_rect()
            .map_or(false, |r| r.contains(ctx.input.mouse_position()))
    }

    fn select_tile_at_screen(&mut self, m: Vec2) {
        let Some(ts) = self.active_tileset() else {
            return;
        };
        let col = ((m.x - PALETTE_ORIGIN.x) / PALETTE_CELL).floor() as i32;
        let row = ((m.y - PALETTE_ORIGIN.y) / PALETTE_CELL).floor() as i32;
        if col < 0 || row < 0 || col as u16 >= ts.cols() || row as u16 >= ts.rows() {
            return;
        }
        self.selected_tile = row as u32 * ts.cols() as u32 + col as u32;
        self.status = format!("kafel: {}", self.selected_tile);
    }

    fn save(&mut self) {
        if let Some(parent) = std::path::Path::new(&self.map_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match self.map.save_xml(&self.map_path) {
            Ok(()) => self.status = format!("zapisano: {}", self.map_path),
            Err(e) => self.status = format!("błąd zapisu: {e}"),
        }
    }

    fn load(&mut self, ctx: &mut Ctx) {
        match ctx.load_tilemap(&self.map_path) {
            Some(m) => {
                self.map = m;
                self.active_layer = 0;
                self.selected_tile = 0;
                self.tileset_idx = 0;
                self.status = format!("wczytano: {}", self.map_path);
            }
            None => self.status = format!("nie udało się wczytać: {}", self.map_path),
        }
    }

    pub fn draw(&mut self, ctx: &mut Ctx) {
        let font = self.font(ctx);
        self.draw_world(ctx);
        self.draw_ui(ctx, font);
    }

    fn draw_world(&mut self, ctx: &mut Ctx) {
        let view = ctx.camera.visible_rect(ctx.window.size);

        ctx.gfx.layer(0);
        self.map.draw(&mut ctx.gfx, view);

        if self.show_grid {
            self.draw_grid(ctx, view);
        }

        // podświetlenie kafla pod kursorem
        let world = ctx.mouse_world();
        let (cx, cy) = self.map.world_to_cell(world);
        if self.map.in_bounds(cx, cy) && !self.over_palette(ctx) {
            let r = self.map.cell_rect(cx, cy);
            ctx.gfx
                .layer(100)
                .color(Color::from_hex(0x39C6FF).with_alpha(0.85))
                .draw_rect_outline(r, 2.0);
        }
    }

    fn draw_grid(&mut self, ctx: &mut Ctx, view: Rect) {
        let ts = self.map.tile_size;
        if ts <= 0.0 {
            return;
        }
        let x0 = (view.left() / ts).floor() as i64;
        let x1 = (view.right() / ts).ceil() as i64;
        let y0 = (view.bottom() / ts).floor() as i64;
        let y1 = (view.top() / ts).ceil() as i64;

        ctx.gfx.layer(50).color(Color::from_hex(0x232A38));
        for xi in x0..=x1 {
            let x = xi as f32 * ts;
            ctx.gfx.draw_line(Vec2::new(x, view.bottom()), Vec2::new(x, view.top()), 1.0);
        }
        for yi in y0..=y1 {
            let y = yi as f32 * ts;
            ctx.gfx.draw_line(Vec2::new(view.left(), y), Vec2::new(view.right(), y), 1.0);
        }
    }

    fn draw_ui(&mut self, ctx: &mut Ctx, font: Option<Font>) {
        ctx.gfx.screen_space();
        self.draw_palette(ctx);
        self.draw_panels(ctx, font);
        ctx.gfx.world_space();
    }

    fn draw_palette(&mut self, ctx: &mut Ctx) {
        let Some(ts) = self.active_tileset() else {
            return;
        };
        let cols = ts.cols() as u32;
        let rows = ts.rows() as u32;
        let texture = ts.texture();
        let count = ts.tile_count();

        let panel_w = cols as f32 * PALETTE_CELL + 8.0;
        let panel_h = rows as f32 * PALETTE_CELL + 8.0;
        ctx.gfx
            .layer(2000)
            .color(Color::from_hex(0x1B1F2A).with_alpha(0.92))
            .draw_rect(Rect::from_xywh(
                PALETTE_ORIGIN.x - 4.0,
                PALETTE_ORIGIN.y - 4.0,
                panel_w,
                panel_h,
            ));

        for idx in 0..count {
            let Some(uv) = ts.uv_for_index(idx) else {
                continue;
            };
            let col = (idx % cols) as f32;
            let row = (idx / cols) as f32;
            let rect = Rect::from_xywh(
                PALETTE_ORIGIN.x + col * PALETTE_CELL,
                PALETTE_ORIGIN.y + row * PALETTE_CELL,
                PALETTE_CELL,
                PALETTE_CELL,
            );
            ctx.gfx.draw_texture(texture, rect, uv);
        }

        // obrys wybranego kafla
        let cols = cols.max(1);
        let scol = (self.selected_tile % cols) as f32;
        let srow = (self.selected_tile / cols) as f32;
        let sel = Rect::from_xywh(
            PALETTE_ORIGIN.x + scol * PALETTE_CELL,
            PALETTE_ORIGIN.y + srow * PALETTE_CELL,
            PALETTE_CELL,
            PALETTE_CELL,
        );
        ctx.gfx
            .color(Color::from_hex(0x39C6FF))
            .draw_rect_outline(sel, 2.0);
    }

    fn draw_panels(&mut self, ctx: &mut Ctx, font: Option<Font>) {
        let Some(font) = font else {
            return;
        };
        let size = ctx.window.size;

        // prawy panel: warstwy
        let mut y = 44.0;
        ctx.gfx.layer(2000).color(Color::WHITE);
        ctx.gfx.draw_text(font, "Warstwy ([ / ]):", Vec2::new(size.x - 210.0, y), 16.0, TextAlign::Left);
        y += 24.0;
        for (i, layer) in self.map.layers.iter().enumerate() {
            let active = i == self.active_layer;
            let color = if active {
                Color::from_hex(0x39C6FF)
            } else if layer.hidden {
                Color::from_hex(0x5A6273)
            } else {
                Color::from_hex(0xC4CDDC)
            };
            let label = format!(
                "{}{}{}",
                if active { "▶ " } else { "  " },
                layer.tiles.name,
                if layer.hidden { " (ukryta)" } else { "" }
            );
            ctx.gfx
                .color(color)
                .draw_text(font, &label, Vec2::new(size.x - 210.0, y), 15.0, TextAlign::Left);
            y += 20.0;
        }

        // informacje o arkuszu i kaflu
        y += 10.0;
        ctx.gfx.color(Color::from_hex(0x9AA5B8));
        ctx.gfx.draw_text(
            font,
            &format!("arkusz (Tab): {}", self.active_tileset_name().unwrap_or_else(|| "brak".to_string())),
            Vec2::new(size.x - 210.0, y),
            14.0,
            TextAlign::Left,
        );
        y += 18.0;
        ctx.gfx.draw_text(
            font,
            &format!("kafel (Q/E): {}", self.selected_tile),
            Vec2::new(size.x - 210.0, y),
            14.0,
            TextAlign::Left,
        );
        y += 18.0;
        ctx.gfx.draw_text(
            font,
            &format!("mapa: {}×{}", self.map.width(), self.map.height()),
            Vec2::new(size.x - 210.0, y),
            14.0,
            TextAlign::Left,
        );

        // pasek stanu na dole
        ctx.gfx
            .layer(2000)
            .color(Color::from_hex(0x1B1F2A).with_alpha(0.92))
            .draw_rect(Rect::from_xywh(0.0, size.y - 28.0, size.x, 28.0));
        ctx.gfx.color(Color::from_hex(0xD4DAE6));
        ctx.gfx.draw_text(font, &self.status, Vec2::new(8.0, size.y - 9.0), 15.0, TextAlign::Left);

        let help = "LPM rysuj · PPM usuń · ŚPM/Spacja pan · kółko zoom · Ctrl+S zapisz · Ctrl+O wczytaj · N warstwa · H ukryj · F solid · G siatka";
        ctx.gfx
            .color(Color::from_hex(0x8A93A6))
            .draw_text(font, help, Vec2::new(size.x - 8.0, size.y - 9.0), 13.0, TextAlign::Right);
    }
}




