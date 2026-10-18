use std::cell::RefCell;
use std::f32::consts::PI;
use std::ops::{Add, Mul, Neg, Sub};
use std::rc::Rc;
use std::time::Duration;

use slint::{Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode};

slint::include_modules!();

// ukuran gambar harus sama dengan area render di app.slint (540x500)
const W: usize = 540;
const H: usize = 500;
// supersampling (anti-aliasing): render 2x lebih besar lalu dirata-rata.
// Ubah ke 1 kalau terasa berat.
const SS: usize = 2;

// ---------------------------------------------------------------
// Matematika vektor 3D minimal
// ---------------------------------------------------------------
#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

impl V3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn norm(self) -> V3 {
        let l = self.dot(self).sqrt();
        if l > 1e-12 { self * (1.0 / l) } else { self }
    }
    fn mulc(self, o: V3) -> V3 {
        V3::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }
}
impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 { V3::new(self.x + o.x, self.y + o.y, self.z + o.z) }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 { V3::new(self.x - o.x, self.y - o.y, self.z - o.z) }
}
impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 { V3::new(self.x * s, self.y * s, self.z * s) }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 { V3::new(-self.x, -self.y, -self.z) }
}

fn rot_y(v: V3, a: f32) -> V3 {
    let (s, c) = a.sin_cos();
    V3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}
fn rot_x(v: V3, a: f32) -> V3 {
    let (s, c) = a.sin_cos();
    V3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

// ---------------------------------------------------------------
// Mesh: balok, silinder, bola
// ---------------------------------------------------------------
struct Mesh {
    verts: Vec<(V3, V3)>, // (posisi, normal)
    tris: Vec<[usize; 3]>,
    color: [f32; 3],
}

fn cuboid(hx: f32, hy: f32, hz: f32, color: [f32; 3]) -> Mesh {
    let half = V3::new(hx, hy, hz);
    let axes = [V3::new(1.0, 0.0, 0.0), V3::new(0.0, 1.0, 0.0), V3::new(0.0, 0.0, 1.0)];
    let mut verts = Vec::new();
    let mut tris = Vec::new();

    for a in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let n = axes[a] * sign;
            let u = axes[(a + 1) % 3];
            let v = axes[(a + 2) % 3];
            let c = n.mulc(half);
            let uh = u.mulc(half);
            let vh = v.mulc(half);
            let base = verts.len();
            for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                verts.push((c + uh * su + vh * sv, n)); // normal rata per sisi
            }
            tris.push([base, base + 1, base + 2]);
            tris.push([base, base + 2, base + 3]);
        }
    }
    Mesh { verts, tris, color }
}

fn cylinder(r: f32, h: f32, seg: usize, color: [f32; 3]) -> Mesh {
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    let hy = h / 2.0;

    // selimut (normal halus)
    for i in 0..=seg {
        let a = 2.0 * PI * i as f32 / seg as f32;
        let (s, c) = a.sin_cos();
        let n = V3::new(c, 0.0, s);
        verts.push((V3::new(r * c, hy, r * s), n)); // 2*i     : atas
        verts.push((V3::new(r * c, -hy, r * s), n)); // 2*i + 1 : bawah
    }
    for i in 0..seg {
        let (t0, b0, t1, b1) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        tris.push([t0, b0, t1]);
        tris.push([t1, b0, b1]);
    }

    // tutup atas & bawah
    for &sy in &[1.0f32, -1.0] {
        let n = V3::new(0.0, sy, 0.0);
        let center = verts.len();
        verts.push((V3::new(0.0, sy * hy, 0.0), n));
        let ring = verts.len();
        for i in 0..=seg {
            let a = 2.0 * PI * i as f32 / seg as f32;
            verts.push((V3::new(r * a.cos(), sy * hy, r * a.sin()), n));
        }
        for i in 0..seg {
            tris.push([center, ring + i, ring + i + 1]);
        }
    }
    Mesh { verts, tris, color }
}

fn sphere(r: f32, stacks: usize, slices: usize, color: [f32; 3]) -> Mesh {
    let mut verts = Vec::new();
    let mut tris = Vec::new();

    for i in 0..=stacks {
        let th = PI * i as f32 / stacks as f32;
        for j in 0..=slices {
            let ph = 2.0 * PI * j as f32 / slices as f32;
            let n = V3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
            verts.push((n * r, n));
        }
    }
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * (slices + 1) + j;
            let b = a + slices + 1;
            tris.push([a, b, a + 1]);
            tris.push([a + 1, b, b + 1]);
        }
    }
    Mesh { verts, tris, color }
}

struct Object {
    mesh: Mesh,
    offset: V3,
    tilt: f32,
    speed: f32,
}

// ---------------------------------------------------------------
// Software renderer (z-buffer + shading per piksel)
// ---------------------------------------------------------------
struct Params {
    yaw: f32,
    pitch: f32,
    dist: f32,
    spin: f32,
    size: f32,
}

struct Pv {
    sx: f32,
    sy: f32,
    w: f32, // 1 / z
    n: V3,  // normal di ruang kamera
}

struct Renderer {
    hw: usize,
    hh: usize,
    rgb: Vec<u8>,
    z: Vec<f32>,
    out: Vec<u8>,
}

impl Renderer {
    fn new() -> Self {
        let (hw, hh) = (W * SS, H * SS);
        Self {
            hw,
            hh,
            rgb: vec![0; hw * hh * 3],
            z: vec![f32::INFINITY; hw * hh],
            out: vec![0; W * H * 3],
        }
    }

    fn clear(&mut self) {
        let (top, bot) = ([27.0f32, 31.0, 58.0], [10.0f32, 12.0, 26.0]);
        for y in 0..self.hh {
            let t = y as f32 / self.hh as f32;
            let c = [
                (top[0] + (bot[0] - top[0]) * t) as u8,
                (top[1] + (bot[1] - top[1]) * t) as u8,
                (top[2] + (bot[2] - top[2]) * t) as u8,
            ];
            for x in 0..self.hw {
                let i = (y * self.hw + x) * 3;
                self.rgb[i..i + 3].copy_from_slice(&c);
            }
        }
        self.z.fill(f32::INFINITY);
    }

    fn tri(&mut self, v: [&Pv; 3], color: [f32; 3], light: V3, half: V3) {
        let (x0, y0, x1, y1, x2, y2) = (v[0].sx, v[0].sy, v[1].sx, v[1].sy, v[2].sx, v[2].sy);
        let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        if area.abs() < 1e-8 {
            return;
        }

        let (fw, fh) = (self.hw as f32, self.hh as f32);
        let min_x = x0.min(x1).min(x2).floor().max(0.0);
        let max_x = x0.max(x1).max(x2).ceil().min(fw - 1.0);
        let min_y = y0.min(y1).min(y2).floor().max(0.0);
        let max_y = y0.max(y1).max(y2).ceil().min(fh - 1.0);
        if max_x < min_x || max_y < min_y {
            return;
        }

        let eps = -1e-4;
        for py in (min_y as usize)..=(max_y as usize) {
            for px in (min_x as usize)..=(max_x as usize) {
                let (pxf, pyf) = (px as f32 + 0.5, py as f32 + 0.5);
                let l0 = ((x1 - pxf) * (y2 - pyf) - (x2 - pxf) * (y1 - pyf)) / area;
                let l1 = ((pxf - x0) * (y2 - y0) - (x2 - x0) * (pyf - y0)) / area;
                let l2 = 1.0 - l0 - l1;
                if l0 < eps || l1 < eps || l2 < eps {
                    continue;
                }

                // interpolasi perspective-correct
                let (a0, a1, a2) = (l0 * v[0].w, l1 * v[1].w, l2 * v[2].w);
                let iw = a0 + a1 + a2;
                if iw <= 0.0 {
                    continue;
                }
                let z = 1.0 / iw;
                let idx = py * self.hw + px;
                if z >= self.z[idx] {
                    continue;
                }
                self.z[idx] = z;

                let mut n = (v[0].n * a0 + v[1].n * a1 + v[2].n * a2).norm();
                if n.z > 0.0 {
                    n = -n;
                }
                let diff = n.dot(light).max(0.0);
                let spec = n.dot(half).max(0.0).powf(40.0) * 0.45;
                let k = 0.16 + 0.9 * diff;

                let o = idx * 3;
                for ch in 0..3 {
                    self.rgb[o + ch] = ((color[ch] * k + spec).min(1.0) * 255.0) as u8;
                }
            }
        }
    }

    fn render(&mut self, objs: &[Object], p: &Params) -> &[u8] {
        self.clear();

        // kamera di origin menghadap +z; cahaya dari kanan-atas-depan
        let light = V3::new(0.6, 0.8, -0.7).norm();
        let view = V3::new(0.0, 0.0, -1.0);
        let half = (light + view).norm();
        let f = self.hh as f32 * 1.05;
        let (cx, cy) = (self.hw as f32 / 2.0, self.hh as f32 / 2.0);

        for obj in objs {
            let ang = p.spin * obj.speed;
            let proj: Vec<Option<Pv>> = obj
                .mesh
                .verts
                .iter()
                .map(|&(pos, nrm)| {
                    // model -> dunia
                    let mut q = rot_y(rot_x(pos * p.size, obj.tilt), ang) + obj.offset;
                    let mut n = rot_y(rot_x(nrm, obj.tilt), ang);
                    // dunia -> kamera (orbit)
                    q = rot_x(rot_y(q, p.yaw), p.pitch);
                    n = rot_x(rot_y(n, p.yaw), p.pitch);
                    q.z += p.dist;
                    if q.z < 0.2 {
                        return None;
                    }
                    Some(Pv {
                        sx: cx + f * q.x / q.z,
                        sy: cy - f * q.y / q.z,
                        w: 1.0 / q.z,
                        n,
                    })
                })
                .collect();

            for t in &obj.mesh.tris {
                if let (Some(a), Some(b), Some(c)) = (&proj[t[0]], &proj[t[1]], &proj[t[2]]) {
                    self.tri([a, b, c], obj.mesh.color, light, half);
                }
            }
        }

        // downsample SS x SS -> ukuran akhir (anti-aliasing)
        let div = (SS * SS) as u32;
        for y in 0..H {
            for x in 0..W {
                let mut sum = [0u32; 3];
                for sy in 0..SS {
                    for sx in 0..SS {
                        let i = ((y * SS + sy) * self.hw + (x * SS + sx)) * 3;
                        sum[0] += self.rgb[i] as u32;
                        sum[1] += self.rgb[i + 1] as u32;
                        sum[2] += self.rgb[i + 2] as u32;
                    }
                }
                let o = (y * W + x) * 3;
                self.out[o] = (sum[0] / div) as u8;
                self.out[o + 1] = (sum[1] / div) as u8;
                self.out[o + 2] = (sum[2] / div) as u8;
            }
        }
        &self.out
    }
}

// ---------------------------------------------------------------
// State aplikasi + GUI
// ---------------------------------------------------------------
struct State {
    renderer: Renderer,
    objects: Vec<Object>,
    yaw: f32,
    pitch: f32,
    base_yaw: f32,
    base_pitch: f32,
    spin: f32,
}

impl State {
    fn new() -> Self {
        let objects = vec![
            Object {
                mesh: cuboid(0.9, 0.6, 0.6, [1.0, 0.54, 0.24]),
                offset: V3::new(-2.3, 0.0, 0.0),
                tilt: 0.0,
                speed: 1.0,
            },
            Object {
                mesh: cylinder(0.6, 1.5, 48, [0.24, 0.78, 1.0]),
                offset: V3::new(0.0, 0.0, 0.0),
                tilt: 0.35,
                speed: 1.4,
            },
            Object {
                mesh: sphere(0.85, 32, 48, [0.48, 0.93, 0.42]),
                offset: V3::new(2.3, 0.0, 0.0),
                tilt: 0.0,
                speed: 0.7,
            },
        ];
        Self {
            renderer: Renderer::new(),
            objects,
            yaw: 0.4,
            pitch: 0.3,
            base_yaw: 0.0,
            base_pitch: 0.0,
            spin: 0.0,
        }
    }
}

fn redraw(ui: &MainWindow, s: &mut State) {
    let params = Params {
        yaw: s.yaw,
        pitch: s.pitch,
        dist: ui.get_zoom(),
        spin: s.spin,
        size: ui.get_shape_size(),
    };
    let rgb = s.renderer.render(&s.objects, &params);
    let buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(rgb, W as u32, H as u32);
    ui.set_frame(slint::Image::from_rgb8(buf));
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let state = Rc::new(RefCell::new(State::new()));

    redraw(&ui, &mut state.borrow_mut());

    // mulai drag: simpan sudut kamera saat ini
    ui.on_drag_started({
        let st = state.clone();
        move || {
            let mut s = st.borrow_mut();
            s.base_yaw = s.yaw;
            s.base_pitch = s.pitch;
        }
    });

    // saat drag: ubah yaw / pitch kamera
    ui.on_dragging({
        let st = state.clone();
        let ui_weak = ui.as_weak();
        move |dx, dy| {
            if let Some(ui) = ui_weak.upgrade() {
                let mut s = st.borrow_mut();
                s.yaw = s.base_yaw + dx * 0.01;
                s.pitch = (s.base_pitch + dy * 0.01).clamp(-1.5, 1.5);
                redraw(&ui, &mut s);
            }
        }
    });

    // slider zoom / ukuran berubah
    ui.on_params_changed({
        let st = state.clone();
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                redraw(&ui, &mut st.borrow_mut());
            }
        }
    });

    ui.on_reset_view({
        let st = state.clone();
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_zoom(8.5);
                ui.set_shape_size(1.0);
                let mut s = st.borrow_mut();
                s.yaw = 0.4;
                s.pitch = 0.3;
                redraw(&ui, &mut s);
            }
        }
    });

    // animasi auto-rotate (~60 fps)
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                if ui.get_auto_rotate() {
                    let mut s = st.borrow_mut();
                    s.spin += 0.02;
                    redraw(&ui, &mut s);
                }
            }
        }
    });

    ui.run()
}
