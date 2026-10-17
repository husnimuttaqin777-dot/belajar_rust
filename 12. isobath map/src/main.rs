//! Trenching Route — port dari PyQt5 + QML (main.py / main.qml) ke Rust + Slint.

mod data;
mod geo;
mod tiles;

slint::include_modules!();

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Write,
    rc::Rc,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use geo::{from_world_px, meters_per_px, world_px, MAX_LAT, TILE_SIZE};
use slint::{Color, ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel, Weak};
use tiles::{Decoded, TileKey};

const START_LAT: f64 = -7.744;
const START_LON: f64 = 109.000;
const START_ZOOM: f64 = 15.0;
const MIN_ZOOM: f64 = 2.0;
const MAX_ZOOM: f64 = 19.0;
const RETRY_AFTER: Duration = Duration::from_secs(10);
const MAX_CACHED_TILES: usize = 600;

/// Nama warna yang dipakai di main.py -> Color (nilai sama dengan warna SVG/Qt).
fn named_color(name: &str) -> Color {
    match name {
        "red" => Color::from_rgb_u8(255, 0, 0),
        "orange" => Color::from_rgb_u8(255, 165, 0),
        "green" => Color::from_rgb_u8(0, 128, 0),
        "navy" => Color::from_rgb_u8(0, 0, 128),
        _ => Color::from_rgb_u8(0, 0, 0),
    }
}

/// Warna isi poligon menurut kedalaman (sama dengan ekspresi `color:` di main.qml).
/// Catatan: versi QML memakai `value > 1 && value < 5` sehingga value == 1 jatuh ke
/// "transparent"; di sini batas 1 dimasukkan ke kelas 1–5 m.
fn isobath_fill(value: f64) -> Color {
    let rgb = if value < 1.0 {
        Some((0xD9, 0x80, 0x14))
    } else if value < 5.0 {
        Some((0xF2, 0xBB, 0x13))
    } else if value < 10.0 {
        Some((0x16, 0xB4, 0xF2))
    } else if value < 25.0 {
        Some((0x11, 0x7C, 0xD9))
    } else {
        None
    };
    match rgb {
        // opacity: 0.5 pada MapPolygon
        Some((r, g, b)) => Color::from_argb_u8(128, r, g, b),
        None => Color::from_argb_u8(0, 0, 0, 0),
    }
}

/// Transformasi geo -> piksel layar untuk satu frame.
struct View {
    zoom: f64,
    left: f64,
    top: f64,
}

impl View {
    fn to_screen(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = world_px(lat, lon, self.zoom);
        (x - self.left, y - self.top)
    }
}

struct MapState {
    lat: f64,
    lon: f64,
    zoom: f64,
    size: (f64, f64),
    drag_origin: Option<(f64, f64)>,

    polygons: Vec<data::Polygon>,
    routes: Vec<Vec<data::Waypoint>>,

    cache: HashMap<TileKey, Image>,
    pending: HashSet<TileKey>,
    failed: HashMap<TileKey, Instant>,
    loader: Sender<TileKey>,

    ui: Weak<MainWindow>,
    m_tiles: Rc<VecModel<TileItem>>,
    m_polys: Rc<VecModel<ShapeItem>>,
    m_iso_labels: Rc<VecModel<LabelItem>>,
    m_circles: Rc<VecModel<CircleItem>>,
    m_lines: Rc<VecModel<ShapeItem>>,
    m_wp_labels: Rc<VecModel<LabelItem>>,
}

thread_local! {
    static STATE: RefCell<Option<MapState>> = const { RefCell::new(None) };
}

fn with_state(f: impl FnOnce(&mut MapState)) {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            f(st);
        }
    });
}

impl MapState {
    fn view(&self) -> View {
        let (cx, cy) = world_px(self.lat, self.lon, self.zoom);
        View { zoom: self.zoom, left: cx - self.size.0 / 2.0, top: cy - self.size.1 / 2.0 }
    }

    fn refresh(&mut self) {
        self.refresh_tiles();
        self.refresh_overlays();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_status(SharedString::from(format!(
                "lat {:.5}  lon {:.5}  zoom {:.1}",
                self.lat, self.lon, self.zoom
            )));
        }
    }

    fn refresh_tiles(&mut self) {
        let (w, h) = self.size;
        if w < 1.0 || h < 1.0 {
            return;
        }
        let v = self.view();
        let tz = (self.zoom.floor() as i64).clamp(0, 19) as u32;
        let tile_px = TILE_SIZE * 2f64.powf(self.zoom - tz as f64);
        let n = 1i64 << tz;

        let tx0 = (v.left / tile_px).floor() as i64;
        let tx1 = ((v.left + w) / tile_px).floor() as i64;
        let ty0 = ((v.top / tile_px).floor() as i64).max(0);
        let ty1 = (((v.top + h) / tile_px).floor() as i64).min(n - 1);

        let mut items = Vec::new();
        let now = Instant::now();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let key = TileKey { z: tz, x: tx.rem_euclid(n) as u32, y: ty as u32 };
                let sx = (tx as f64 * tile_px - v.left).floor();
                let sy = (ty as f64 * tile_px - v.top).floor();
                if let Some(img) = self.cache.get(&key) {
                    items.push(TileItem {
                        x: sx as f32,
                        y: sy as f32,
                        size: (tile_px.ceil() + 1.0) as f32,
                        source: img.clone(),
                    });
                } else if !self.pending.contains(&key)
                    && self.failed.get(&key).map_or(true, |t| now.duration_since(*t) > RETRY_AFTER)
                {
                    self.pending.insert(key);
                    let _ = self.loader.send(key);
                }
            }
        }
        self.m_tiles.set_vec(items);
    }

    fn refresh_overlays(&mut self) {
        let (w, h) = self.size;
        let v = self.view();

        // --- poligon isobath + label "x m" ---
        let mut polys = Vec::new();
        let mut iso_labels = Vec::new();
        for p in &self.polygons {
            let pts: Vec<(f64, f64)> = p.points.iter().map(|&(la, lo)| v.to_screen(la, lo)).collect();
            let (mut minx, mut maxx, mut miny, mut maxy) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for &(x, y) in &pts {
                minx = minx.min(x);
                maxx = maxx.max(x);
                miny = miny.min(y);
                maxy = maxy.max(y);
            }
            if maxx < 0.0 || minx > w || maxy < 0.0 || miny > h {
                continue;
            }
            polys.push(ShapeItem {
                commands: path_commands(&pts, true),
                fill: isobath_fill(p.value),
                stroke: Color::from_argb_u8(128, 0, 0, 0),
                stroke_width: 1.0,
            });
            let (cx, cy) = v.to_screen(p.center.0, p.center.1);
            iso_labels.push(LabelItem {
                x: (cx - 30.0) as f32,
                y: (cy - 10.0) as f32,
                text: format!("{} m", p.value).into(),
                bg: Color::from_argb_u8(179, 255, 255, 255), // opacity 0.7
                border: Color::from_argb_u8(0, 0, 0, 0),
                border_width: 0.0,
            });
        }

        // --- rute: garis, lingkaran (radius 10 m), label ---
        let mut lines = Vec::new();
        let mut circles = Vec::new();
        let mut wp_labels = Vec::new();
        for route in &self.routes {
            let scr: Vec<(f64, f64)> = route.iter().map(|p| v.to_screen(p.lat, p.lon)).collect();
            for (j, pair) in scr.windows(2).enumerate() {
                // segmen diwarnai sesuai titik awal
                lines.push(ShapeItem {
                    commands: path_commands(pair, false),
                    fill: Color::from_argb_u8(0, 0, 0, 0),
                    stroke: named_color(route[j].color),
                    stroke_width: 4.0,
                });
            }
            for (p, &(x, y)) in route.iter().zip(&scr) {
                let r = 10.0 / meters_per_px(p.lat, self.zoom) + 1.5; // + setengah border
                circles.push(CircleItem {
                    x: (x - r) as f32,
                    y: (y - r) as f32,
                    diameter: (2.0 * r) as f32,
                    border: named_color(p.color),
                });
                // anchorPoint (40, 60) pada MapQuickItem
                wp_labels.push(LabelItem {
                    x: (x - 40.0) as f32,
                    y: (y - 60.0) as f32,
                    text: p.name.as_str().into(),
                    bg: Color::from_rgb_u8(255, 255, 255),
                    border: Color::from_rgb_u8(0, 0, 0),
                    border_width: 1.0,
                });
            }
        }

        self.m_polys.set_vec(polys);
        self.m_iso_labels.set_vec(iso_labels);
        self.m_lines.set_vec(lines);
        self.m_circles.set_vec(circles);
        self.m_wp_labels.set_vec(wp_labels);
    }

    fn drag_start(&mut self) {
        self.drag_origin = Some(world_px(self.lat, self.lon, self.zoom));
    }

    fn drag_move(&mut self, dx: f64, dy: f64) {
        if let Some((ox, oy)) = self.drag_origin {
            let (lat, lon) = from_world_px(ox - dx, oy - dy, self.zoom);
            self.lat = lat.clamp(-MAX_LAT, MAX_LAT);
            self.lon = lon;
            self.refresh();
        }
    }

    /// Zoom dengan titik di bawah kursor tetap diam.
    fn zoom_at(&mut self, dz: f64, mx: f64, my: f64) {
        let new_zoom = (self.zoom + dz.clamp(-1.0, 1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        if (new_zoom - self.zoom).abs() < 1e-9 {
            return;
        }
        let v = self.view();
        let (cur_lat, cur_lon) = from_world_px(v.left + mx, v.top + my, self.zoom);
        let (wx, wy) = world_px(cur_lat, cur_lon, new_zoom);
        let (lat, lon) = from_world_px(wx - (mx - self.size.0 / 2.0), wy - (my - self.size.1 / 2.0), new_zoom);
        self.zoom = new_zoom;
        self.lat = lat.clamp(-MAX_LAT, MAX_LAT);
        self.lon = lon;
        self.drag_origin = None;
        self.refresh();
    }

    fn tile_arrived(&mut self, key: TileKey, res: Result<Decoded, String>) {
        self.pending.remove(&key);
        match res {
            Ok(d) => {
                let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&d.rgba, d.width, d.height);
                if self.cache.len() >= MAX_CACHED_TILES {
                    self.cache.clear();
                }
                self.cache.insert(key, Image::from_rgba8(buf));
                self.failed.remove(&key);
            }
            Err(e) => {
                eprintln!("tile {}/{}/{} gagal: {e}", key.z, key.x, key.y);
                self.failed.insert(key, Instant::now());
            }
        }
        self.refresh_tiles();
    }
}

/// Bangun data path SVG ("M x y L x y ... Z") dari titik layar.
fn path_commands(pts: &[(f64, f64)], close: bool) -> SharedString {
    let mut s = String::with_capacity(pts.len() * 16 + 2);
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(s, "{} {:.1} {:.1} ", if i == 0 { 'M' } else { 'L' }, x, y);
    }
    if close {
        s.push('Z');
    }
    s.into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let csv_path = std::env::args().nth(1).unwrap_or_else(|| "isobath.csv".to_string());
    let polygons = data::load_polygons(&csv_path)
        .map_err(|e| format!("gagal membaca '{csv_path}': {e}"))?;

    let ui = MainWindow::new()?;

    let m_tiles = Rc::new(VecModel::<TileItem>::default());
    let m_polys = Rc::new(VecModel::<ShapeItem>::default());
    let m_iso_labels = Rc::new(VecModel::<LabelItem>::default());
    let m_circles = Rc::new(VecModel::<CircleItem>::default());
    let m_lines = Rc::new(VecModel::<ShapeItem>::default());
    let m_wp_labels = Rc::new(VecModel::<LabelItem>::default());
    ui.set_tiles(ModelRc::from(m_tiles.clone()));
    ui.set_polygons(ModelRc::from(m_polys.clone()));
    ui.set_iso_labels(ModelRc::from(m_iso_labels.clone()));
    ui.set_circles(ModelRc::from(m_circles.clone()));
    ui.set_lines(ModelRc::from(m_lines.clone()));
    ui.set_wp_labels(ModelRc::from(m_wp_labels.clone()));

    // Worker tile -> hasil dikirim balik ke thread UI.
    let loader = tiles::spawn_workers(4, |key, res| {
        let _ = slint::invoke_from_event_loop(move || {
            with_state(|s| s.tile_arrived(key, res));
        });
    });

    STATE.with(|s| {
        *s.borrow_mut() = Some(MapState {
            lat: START_LAT,
            lon: START_LON,
            zoom: START_ZOOM,
            size: (0.0, 0.0),
            drag_origin: None,
            polygons,
            routes: vec![data::route1(), data::route2()],
            cache: HashMap::new(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            loader,
            ui: ui.as_weak(),
            m_tiles,
            m_polys,
            m_iso_labels,
            m_circles,
            m_lines,
            m_wp_labels,
        });
    });

    let weak = ui.as_weak();
    ui.on_viewport_changed(move || {
        if let Some(ui) = weak.upgrade() {
            let (w, h) = (ui.get_map_width() as f64, ui.get_map_height() as f64);
            with_state(|s| {
                s.size = (w, h);
                s.refresh();
            });
        }
    });
    ui.on_drag_start(|| with_state(|s| s.drag_start()));
    ui.on_drag_move(|dx, dy| with_state(|s| s.drag_move(dx as f64, dy as f64)));
    ui.on_zoom_at(|dz, x, y| with_state(|s| s.zoom_at(dz as f64, x as f64, y as f64)));

    ui.run()?;
    Ok(())
}
