use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode, VecModel, Weak};


slint::include_modules!();

// ---------------------------------------------------------------
// Konfigurasi MQTT 
// ---------------------------------------------------------------
const BROKER: &str = "127.0.0.1";
const PORT: u16 = 1883;
const CLIENT_ID: &str = "PC_1";
const TOPIC_SUB: &str = "sensor";
const TOPIC_LAT: &str = "lat";




// ---------------- pengaturan ----------------
//const START_LAT: f64 = -2.75819;
//const START_LON: f64 = 105.787;
//const HEADING:f32 = 0.0; // arah marker awal (derajat, 0° = utara, searah jarum jam)
const START_ZOOM: i32 = 14;
const MIN_ZOOM: i32 = 3;
const MAX_ZOOM: i32 = 18;
const TRACK_MAX: usize = 500; // titik track maksimum
//const SIMULATE: bool = true; // true = marker berputar mengelilingi titik awal (demo)
const USER_AGENT: &str = "PetaGUI/1.0 (Rust Slint demo)"; // wajib diisi sesuai kebijakan OSM

type TileKey = (i32, i32, i32); // zoom, x, y
type TileResult = (TileKey, Option<SharedPixelBuffer<Rgba8Pixel>>);

// ---------------- matematika peta (Web Mercator) ----------------
// "world" = koordinat 0..1 yang tidak tergantung zoom
fn to_world(lat: f64, lon: f64) -> (f64, f64) {
    let x = (lon + 180.0) / 360.0;
    let s = lat.to_radians().sin().clamp(-0.9999, 0.9999);
    let y = 0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * PI);
    (x, y)
}

fn to_latlon(wx: f64, wy: f64) -> (f64, f64) {
    let lon = wx * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * wy)).sinh().atan().to_degrees();
    (lat, lon)
}


fn set_status(ui: &Weak<MainWindow>, text: &str) {
    let text = text.to_string();
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(text.into()));
}


// ---------------- state ----------------
struct State {
    wx: f64, // pusat peta
    wy: f64,
    zoom: i32,
    w: f64, // ukuran area peta (px logis)
    h: f64,
    lat: f64, // posisi marker
    lon: f64,
    heading: f32,
    track: Vec<(f64, f64)>, // line1
    route: Vec<(f64, f64)>, // line (waypoint)
    follow: bool,
    last_drag: Option<(f32, f32)>,
    tiles: HashMap<TileKey, Image>,
    requested: HashSet<TileKey>,
}

impl State {
    fn scale(&self) -> f64 {
        256.0 * (1u64 << self.zoom) as f64
    }

    fn to_screen(&self, wx: f64, wy: f64) -> (f32, f32) {
        let s = self.scale();
        (
            ((wx - self.wx) * s + self.w / 2.0) as f32,
            ((wy - self.wy) * s + self.h / 2.0) as f32,
        )
    }

    fn from_screen(&self, x: f64, y: f64) -> (f64, f64) {
        let s = self.scale();
        (self.wx + (x - self.w / 2.0) / s, self.wy + (y - self.h / 2.0) / s)
    }

    fn set_position(&mut self, lat: f64, lon: f64, heading: f32) {
        let (wx, wy) = to_world(lat, lon);
            self.heading = heading;
            self.lat = lat;
            self.lon = lon;
            self.track.push((wx, wy));
            if self.track.len() > TRACK_MAX {
                self.track.remove(0);
            }
            if self.follow {
                self.wx = wx;
                self.wy = wy;
            }
        }
    }

// ---------------- gambar ulang ----------------
fn path_cmd(st: &State, pts: &[(f64, f64)]) -> String {
    let mut s = String::new();
    for (i, (wx, wy)) in pts.iter().enumerate() {
        let (x, y) = st.to_screen(*wx, *wy);
        s.push_str(&format!("{} {:.1} {:.1} ", if i == 0 { 'M' } else { 'L' }, x, y));
    }
    s
}

fn refresh(st: &mut State, ui: &MainWindow, jobs: &mpsc::Sender<TileKey>) {
    if st.w < 1.0 || st.h < 1.0 {
        return;
    }

    // buang tile zoom lain bila cache terlalu besar
    if st.tiles.len() > 300 {
        let z = st.zoom;
        st.tiles.retain(|k, _| k.0 == z);
        st.requested.retain(|k| k.0 == z);
    }

    let scale = st.scale();
    let n = 1i32 << st.zoom;
    let tlx = st.wx * scale - st.w / 2.0;
    let tly = st.wy * scale - st.h / 2.0;
    let x0 = (tlx / 256.0).floor() as i32;
    let x1 = ((tlx + st.w) / 256.0).floor() as i32;
    let y0 = ((tly / 256.0).floor() as i32).max(0);
    let y1 = (((tly + st.h) / 256.0).floor() as i32).min(n - 1);

    let mut tiles = Vec::new();
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let key = (st.zoom, tx.rem_euclid(n), ty);
            match st.tiles.get(&key) {
                Some(img) => tiles.push(TileItem {
                    x: (tx as f64 * 256.0 - tlx) as f32,
                    y: (ty as f64 * 256.0 - tly) as f32,
                    img: img.clone(),
                }),
                None => {
                    if st.requested.insert(key) {
                        let _ = jobs.send(key);
                    }
                }
            }
        }
    }

    let pins: Vec<PinItem> = st
        .route
        .iter()
        .enumerate()
        .map(|(i, (wx, wy))| {
            let (x, y) = st.to_screen(*wx, *wy);
            PinItem { x, y, label: (i + 1).to_string().into() }
        })
        .collect();

    let (mwx, mwy) = to_world(st.lat, st.lon);
    let (mx, my) = st.to_screen(mwx, mwy);

    ui.set_tiles(ModelRc::new(VecModel::from(tiles)));
    ui.set_pins(ModelRc::new(VecModel::from(pins)));
    ui.set_track_path(path_cmd(st, &st.track).into());
    ui.set_route_path(path_cmd(st, &st.route).into());
    ui.set_marker_x(mx);
    ui.set_marker_y(my);
    ui.set_heading(st.heading);
    ui.set_lat_text(format!("{:.5}", st.lat).into());
    ui.set_lon_text(format!("{:.5}", st.lon).into());
}

// ---------------- download tile (2 thread, sesuai batas OSM) ----------------


fn load_tile(
    (z, x, y): TileKey,
) -> Result<SharedPixelBuffer<Rgba8Pixel>, Box<dyn std::error::Error>> {
    let path = PathBuf::from(format!("tile_cache/{z}/{x}/{y}.png"));

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,

        Err(_) => {
            // Esri World Imagery (satelit)
            let url = format!("https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}");

            //topographic
            //let url = format!("https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}");

            // OpenStreetMap
            //let url = format!("https://tile.openstreetmap.org/{z}/{x}/{y}.png");

            // Ocean Base
            //let url = format!("https://server.arcgisonline.com/ArcGIS/rest/services/Ocean/World_Ocean_Base/MapServer/tile/{z}/{y}/{x}");

            let mut buf = Vec::new();

            ureq::get(&url)
                .set("User-Agent", USER_AGENT)
                .timeout(Duration::from_secs(10))
                .call()?
                .into_reader()
                .read_to_end(&mut buf)?;

            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }

            let _ = std::fs::write(&path, &buf);

            buf
        }
    };

    let img =
        image::load_from_memory(&bytes)?.to_rgba8();

    let (w, h) = img.dimensions();

    Ok(SharedPixelBuffer::clone_from_slice(
        img.as_raw(),
        w,
        h,
    ))
}






fn spawn_workers(jobs: mpsc::Receiver<TileKey>, out: mpsc::Sender<TileResult>) {
    let jobs = Arc::new(Mutex::new(jobs));
    for _ in 0..2 {
        let jobs = jobs.clone();
        let out = out.clone();
        thread::spawn(move || loop {
            let job = jobs.lock().unwrap().recv();
            let Ok(key) = job else { break };
            let res = load_tile(key);
            if let Err(e) = &res {
                eprintln!("Gagal tile {key:?}: {e}");
            }
            let _ = out.send((key, res.ok()));
        });
    }
}

// ---------------- bungkus agar mudah dipakai di callback ----------------
#[derive(Clone)]
struct App {
    st: Rc<RefCell<State>>,
    jobs: mpsc::Sender<TileKey>,
    weak: slint::Weak<MainWindow>,
}

impl App {
    fn redraw(&self) {
        if let Some(ui) = self.weak.upgrade() {
            refresh(&mut self.st.borrow_mut(), &ui, &self.jobs);
        }
    }

    fn change(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.st.borrow_mut());
        self.redraw();
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    
    // ---- MQTT client (setara paho.Client + loop_start) ----
    let mut opts = MqttOptions::new(CLIENT_ID, BROKER, PORT);
    opts.set_keep_alive(Duration::from_secs(60));
    let (client, mut connection) = Client::new(opts, 16);

    println!("connecting to broker {BROKER}");
    let (pos_tx, pos_rx) = mpsc::channel::<(f64, f64, f32)>();
    // thread jaringan (setara client.loop_start + on_message)
    {
        let ui_weak = ui.as_weak();
        let sub_client = client.clone();
        
        let pos_tx_mqtt = pos_tx.clone();
        thread::spawn(move || {
            let mut lat: f64 = START_LAT;
            let mut lon: f64 = START_LON;
            let mut heading: f32 = HEADING;

            for event in connection.iter() {
                match event {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        println!("{BROKER} connected");
                        let _ = sub_client.try_subscribe(TOPIC_SUB, QoS::AtMostOnce);
                        let _ = sub_client.try_subscribe(TOPIC_LAT, QoS::AtMostOnce);
                        let _ = sub_client.try_subscribe("lon", QoS::AtMostOnce);
                        let _ = sub_client.try_subscribe("heading", QoS::AtMostOnce);
                        set_status(&ui_weak, &format!("MQTT: connected ({BROKER}:{PORT})"));
                    }

                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let msg = String::from_utf8_lossy(&p.payload).trim().to_string();
                        let mut changed = false;

                        match p.topic.as_str() {
                            TOPIC_SUB => println!("sensor: {msg}"),
                            TOPIC_LAT => {
                                if let Ok(v) = msg.parse::<f64>() {
                                    lat = v;
                                    changed = true;
                                }
                            }
                            "lon" => {
                                if let Ok(v) = msg.parse::<f64>() {
                                    lon = v;
                                    changed = true;
                                }
                            }
                            "heading" => {
                                if let Ok(v) = msg.parse::<f32>() {
                                    heading = v;
                                    changed = true;
                                }
                            }
                            _ => {}
                        }

                        // kirim hanya kalau ada nilai yang benar-benar berubah
                        if changed {
                            let _ = pos_tx_mqtt.send((lat, lon, heading));
                        }
                    }

                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("MQTT error: {e}");
                        set_status(&ui_weak, "MQTT: disconnected, retrying...");
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });
   
    }
    
    
    
    

    
    const START_LAT: f64 = -3.75819;
    const START_LON: f64 = 105.787;
    const HEADING: f32 = 90.0; // arah marker awal (derajat, 0° = utara, searah jarum jam)

    let (wx, wy) = to_world(START_LAT, START_LON);
    let st = Rc::new(RefCell::new(State {
        wx,
        wy,
        zoom: START_ZOOM,
        w: 0.0,
        h: 0.0,
        lat: START_LAT,
        lon: START_LON,
        heading: HEADING,
        track: Vec::new(),
        route: Vec::new(),
        follow: true,
        last_drag: None,
        tiles: HashMap::new(),
        requested: HashSet::new(),
    }));

    let (job_tx, job_rx) = mpsc::channel::<TileKey>();
    let (tile_tx, tile_rx) = mpsc::channel::<TileResult>();
    spawn_workers(job_rx, tile_tx);

    // Kirim posisi baru lewat pos_tx dari thread mana saja
    // (serial, MQTT, dst): pos_tx.send((lat, lon)).
   
    /*
    if SIMULATE {
        let tx = pos_tx.clone();
        thread::spawn(move || {
            let mut t = 0.0f64;
            loop {
                t += 0.0;
                let _ = tx.send((START_LAT + 0.01 * t.sin(), START_LON + 0.01 * t.cos()));
                thread::sleep(Duration::from_secs(1));
                print(_)
            }
        });
    }
    */

    let app = App { st: st.clone(), jobs: job_tx, weak: ui.as_weak() };

    // ---- drag (geser) + koordinat kursor ----
    {
        let app = app.clone();
        ui.on_pointer(move |x, y, pressed| {
            let mut moved = false;
            let hover;
            {
                let mut st = app.st.borrow_mut();
                if pressed {
                    if let Some((lx, ly)) = st.last_drag {
                        let s = st.scale();
                        st.wx = (st.wx - (x - lx) as f64 / s).rem_euclid(1.0);
                        st.wy = (st.wy - (y - ly) as f64 / s).clamp(0.0, 1.0);
                        st.follow = false;
                        moved = true;
                    }
                    st.last_drag = Some((x, y));
                } else {
                    st.last_drag = None;
                }
                let (wx, wy) = st.from_screen(x as f64, y as f64);
                let (lat, lon) = to_latlon(wx, wy);
                hover = format!("{lat:.6}, {lon:.6}");
            }
            if let Some(ui) = app.weak.upgrade() {
                ui.set_hover_text(hover.into());
            }
            if moved {
                app.redraw();
            }
        });
    }

    // ---- tekan-tahan: tambah waypoint (md + line) ----
    {
        let app = app.clone();
        ui.on_long_press(move |x, y| {
            app.change(|st| {
                let p = st.from_screen(x as f64, y as f64);
                st.route.push(p);
            });
        });
    }

    // ---- klik dua kali: tambah titik track (md1 + line1) ----
    {
        let app = app.clone();
        ui.on_double_tap(move |x, y| {
            app.change(|st| {
                let p = st.from_screen(x as f64, y as f64);
                st.track.push(p);
                if st.track.len() > TRACK_MAX {
                    st.track.remove(0);
                }
            });
        });
    }

    // ---- zoom (scroll / tombol), titik di bawah kursor tetap ----
    {
        let app = app.clone();
        ui.on_zoom(move |dir, x, y| {
            app.change(|st| {
                let nz = (st.zoom + dir).clamp(MIN_ZOOM, MAX_ZOOM);
                if nz == st.zoom {
                    return;
                }
                let (x, y) = (x as f64, y as f64);
                let (px, py) = st.from_screen(x, y);
                st.zoom = nz;
                let s = st.scale();
                st.wx = px - (x - st.w / 2.0) / s;
                st.wy = (py - (y - st.h / 2.0) / s).clamp(0.0, 1.0);
                st.follow = false;
            });
        });
    }

    // ---- tombol clear line track ----
    {
        let app = app.clone();
        ui.on_clear_all(move || {
            app.change(|st| {
                st.route.clear();
                st.track.clear();
            });
        });
    }

    // ---- tombol pusatkan marker ----
    {
        let app = app.clone();
        ui.on_center_on_marker(move || {
            app.change(|st| {
                let (wx, wy) = to_world(st.lat, st.lon);
                st.wx = wx;
                st.wy = wy;
                st.follow = true;
            });
        });
    }

    // ---- timer UI: ukuran jendela, tile yang selesai diunduh, posisi baru ----
    let timer = Timer::default();
    {
        let app = app.clone();
        timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
            let Some(ui) = app.weak.upgrade() else { return };
            let mut dirty = false;
            {
                let mut st = app.st.borrow_mut();

                let size = ui.window().size().to_logical(ui.window().scale_factor());
                if (size.width as f64 - st.w).abs() > 0.5 || (size.height as f64 - st.h).abs() > 0.5 {
                    st.w = size.width as f64;
                    st.h = size.height as f64;
                    dirty = true;
                }

                while let Ok((key, buf)) = tile_rx.try_recv() {
                    if let Some(b) = buf {
                        st.tiles.insert(key, Image::from_rgba8(b));
                        dirty = true;
                    }
                }

                while let Ok((lat, lon, heading)) = pos_rx.try_recv() {
                    st.set_position(lat, lon, heading);
                    dirty = true;
                }
            }
            if dirty {
                app.redraw();
            }
        });
    }

    ui.run()
}
