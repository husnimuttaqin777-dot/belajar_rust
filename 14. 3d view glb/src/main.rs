use std::cell::RefCell;
use std::f32::consts::PI;
use std::ops::{Add, Mul, Neg, Sub};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rayon::prelude::*;
use slint::{Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode, Weak};

slint::include_modules!();

// ukuran gambar harus sama dengan area render di app.slint (540x500)
const W: usize = 540;
const H: usize = 500;
// supersampling maksimum (anti-aliasing) saat diam / auto-rotate.
// Saat drag mouse / geser slider otomatis turun ke 1x supaya responsif.
const SS_MAX: usize = 2;

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
    fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
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
// Software renderer: z-buffer + shading per piksel, paralel (rayon)
// ---------------------------------------------------------------
#[derive(Clone, Copy)]
struct Params {
    yaw: f32,
    pitch: f32,
    dist: f32,
    spin: f32,
    size: f32,
    show_model: bool,
    fast: bool, // true = render 1x (saat interaksi), false = SS_MAX
}

struct Pv {
    sx: f32,
    sy: f32,
    w: f32, // 1 / z
    n: V3,  // normal di ruang kamera
}

// potongan baris layar yang digarap satu thread
struct Band<'a> {
    rgb: &'a mut [u8],
    z: &'a mut [f32],
    hw: usize,
    y0: usize,
    y1: usize,
}

fn raster_tri(band: &mut Band, v: [&Pv; 3], color: [f32; 3], light: V3, half: V3) {
    let (x0, y0, x1, y1, x2, y2) = (v[0].sx, v[0].sy, v[1].sx, v[1].sy, v[2].sx, v[2].sy);
    let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if area.abs() < 1e-8 {
        return;
    }

    let fw = band.hw as f32;
    let min_x = x0.min(x1).min(x2).floor().max(0.0);
    let max_x = x0.max(x1).max(x2).ceil().min(fw - 1.0);
    let min_y = y0.min(y1).min(y2).floor().max(band.y0 as f32);
    let max_y = y0.max(y1).max(y2).ceil().min((band.y1 - 1) as f32);
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
            let idx = (py - band.y0) * band.hw + px;
            if z >= band.z[idx] {
                continue;
            }
            band.z[idx] = z;

            let mut n = (v[0].n * a0 + v[1].n * a1 + v[2].n * a2).norm();
            if n.z > 0.0 {
                n = -n;
            }
            let diff = n.dot(light).max(0.0);
            let spec = n.dot(half).max(0.0).powf(40.0) * 0.45;
            let k = 0.16 + 0.9 * diff;

            let o = idx * 3;
            for ch in 0..3 {
                band.rgb[o + ch] = ((color[ch] * k + spec).min(1.0) * 255.0) as u8;
            }
        }
    }
}

struct Renderer {
    rgb: Vec<u8>,
    z: Vec<f32>,
    out: Vec<u8>,
}

impl Renderer {
    fn new() -> Self {
        let n = (W * SS_MAX) * (H * SS_MAX);
        Self { rgb: vec![0; n * 3], z: vec![f32::INFINITY; n], out: vec![0; W * H * 3] }
    }

    fn render(&mut self, objs: &[Object], p: &Params, ss: usize) -> &[u8] {
        let (hw, hh) = (W * ss, H * ss);

        // kamera di origin menghadap +z; cahaya dari kanan-atas-depan
        let light = V3::new(0.6, 0.8, -0.7).norm();
        let view = V3::new(0.0, 0.0, -1.0);
        let half = (light + view).norm();
        let f = hh as f32 * 1.05;
        let (cx, cy) = (hw as f32 / 2.0, hh as f32 / 2.0);

        // 1) transformasi + proyeksi semua vertex (paralel)
        let projected: Vec<Vec<Option<Pv>>> = objs
            .iter()
            .map(|obj| {
                let ang = p.spin * obj.speed;
                obj.mesh
                    .verts
                    .par_iter()
                    .map(|&(pos, nrm)| {
                        let mut q = rot_y(rot_x(pos * p.size, obj.tilt), ang) + obj.offset;
                        let mut n = rot_y(rot_x(nrm, obj.tilt), ang);
                        q = rot_x(rot_y(q, p.yaw), p.pitch);
                        n = rot_x(rot_y(n, p.yaw), p.pitch);
                        q.z += p.dist;
                        if q.z < 0.2 {
                            return None;
                        }
                        Some(Pv { sx: cx + f * q.x / q.z, sy: cy - f * q.y / q.z, w: 1.0 / q.z, n })
                    })
                    .collect()
            })
            .collect();

        // 2) rasterisasi: layar dibagi jadi pita-pita baris, tiap pita di thread berbeda
        let threads = rayon::current_num_threads().max(1);
        let band_rows = ((hh + threads * 2 - 1) / (threads * 2)).max(8);
        let (top, bot) = ([27.0f32, 31.0, 58.0], [10.0f32, 12.0, 26.0]);

        let rgb = &mut self.rgb[..hw * hh * 3];
        let zbuf = &mut self.z[..hw * hh];
        rgb.par_chunks_mut(hw * 3 * band_rows)
            .zip(zbuf.par_chunks_mut(hw * band_rows))
            .enumerate()
            .for_each(|(bi, (rgb_b, z_b))| {
                let y0 = bi * band_rows;
                let rows = z_b.len() / hw;
                let y1 = y0 + rows;

                // latar gradasi
                for r in 0..rows {
                    let t = (y0 + r) as f32 / hh as f32;
                    let c = [
                        (top[0] + (bot[0] - top[0]) * t) as u8,
                        (top[1] + (bot[1] - top[1]) * t) as u8,
                        (top[2] + (bot[2] - top[2]) * t) as u8,
                    ];
                    for x in 0..hw {
                        let i = (r * hw + x) * 3;
                        rgb_b[i..i + 3].copy_from_slice(&c);
                    }
                }
                z_b.fill(f32::INFINITY);

                let mut band = Band { rgb: rgb_b, z: z_b, hw, y0, y1 };
                for (obj, proj) in objs.iter().zip(projected.iter()) {
                    for t in &obj.mesh.tris {
                        if let (Some(a), Some(b), Some(c)) = (&proj[t[0]], &proj[t[1]], &proj[t[2]]) {
                            raster_tri(&mut band, [a, b, c], obj.mesh.color, light, half);
                        }
                    }
                }
            });

        // 3) downsample ss x ss -> ukuran akhir (anti-aliasing), paralel per baris
        let src = &self.rgb;
        let div = (ss * ss) as u32;
        self.out.par_chunks_mut(W * 3).enumerate().for_each(|(y, row)| {
            for x in 0..W {
                let mut sum = [0u32; 3];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let i = ((y * ss + sy) * hw + (x * ss + sx)) * 3;
                        sum[0] += src[i] as u32;
                        sum[1] += src[i + 1] as u32;
                        sum[2] += src[i + 2] as u32;
                    }
                }
                let o = x * 3;
                row[o] = (sum[0] / div) as u8;
                row[o + 1] = (sum[1] / div) as u8;
                row[o + 2] = (sum[2] / div) as u8;
            }
        });
        &self.out
    }
}

// ---------------------------------------------------------------
// Loader file .glb (glTF 2.0 binary)
// ---------------------------------------------------------------
const GLB_NAME: &str = "target.glb";

type Mat = [[f32; 4]; 4]; // kolom-mayor, sama seperti glTF

const IDENTITY: Mat = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k][row] * b[c][k];
            }
            r[c][row] = sum;
        }
    }
    r
}

// glTF: kanan-tangan, depan model = +Z. Renderer ini: kamera menghadap +Z.
// Sumbu Z dibalik agar bagian depan model menghadap kamera dan tidak terbalik kiri-kanan.
fn xform_point(m: &Mat, p: [f32; 3]) -> V3 {
    let x = m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0];
    let y = m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1];
    let z = m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2];
    V3::new(x, y, -z)
}

fn xform_normal(m: &Mat, n: [f32; 3]) -> V3 {
    let x = m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2];
    let y = m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2];
    let z = m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2];
    V3::new(x, y, -z).norm()
}

// warna rata-rata tekstur (tekstur sendiri tidak ditampilkan, hanya dihitung rata-ratanya)
fn average_color(img: &gltf::image::Data) -> Option<[f32; 3]> {
    let ch = match img.format {
        gltf::image::Format::R8G8B8 => 3,
        gltf::image::Format::R8G8B8A8 => 4,
        _ => return None,
    };
    let n = img.pixels.len() / ch;
    if n == 0 {
        return None;
    }
    let step = (n / 4096).max(1);
    let (mut r, mut g, mut b, mut cnt) = (0u64, 0u64, 0u64, 0u64);
    let mut i = 0;
    while i < n {
        let o = i * ch;
        r += img.pixels[o] as u64;
        g += img.pixels[o + 1] as u64;
        b += img.pixels[o + 2] as u64;
        cnt += 1;
        i += step;
    }
    let d = cnt as f32 * 255.0;
    Some([r as f32 / d, g as f32 / d, b as f32 / d])
}

fn material_color(m: &gltf::Material, images: &[gltf::image::Data]) -> [f32; 3] {
    let pbr = m.pbr_metallic_roughness();
    let f = pbr.base_color_factor();
    // faktor warna glTF bersifat linear -> ubah kasar ke sRGB
    let g = 1.0 / 2.2;
    let mut c = [f[0].powf(g), f[1].powf(g), f[2].powf(g)];
    if let Some(info) = pbr.base_color_texture() {
        let idx = info.texture().source().index();
        if let Some(avg) = images.get(idx).and_then(average_color) {
            c = [c[0] * avg[0], c[1] * avg[1], c[2] * avg[2]];
        }
    }
    c
}

fn visit_node(
    node: &gltf::Node,
    parent: &Mat,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    out: &mut Vec<Mesh>,
) {
    let world = mat_mul(parent, &node.transform().matrix());

    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue; // hanya segitiga biasa yang didukung
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()].0[..]));
            let positions: Vec<[f32; 3]> = match reader.read_positions() {
                Some(p) => p.collect(),
                None => continue,
            };
            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
            let indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            let color = material_color(&prim.material(), images);

            let mut verts: Vec<(V3, V3)> = Vec::new();
            let mut tris: Vec<[usize; 3]> = Vec::new();
            let valid = |t: &[u32]| t.iter().all(|&i| (i as usize) < positions.len());

            match normals {
                Some(ns) if ns.len() == positions.len() => {
                    for (p, n) in positions.iter().zip(ns.iter()) {
                        verts.push((xform_point(&world, *p), xform_normal(&world, *n)));
                    }
                    for t in indices.chunks_exact(3) {
                        if valid(t) {
                            tris.push([t[0] as usize, t[1] as usize, t[2] as usize]);
                        }
                    }
                }
                _ => {
                    // tidak ada normal di file -> normal rata per segitiga
                    for t in indices.chunks_exact(3) {
                        if !valid(t) {
                            continue;
                        }
                        let a = xform_point(&world, positions[t[0] as usize]);
                        let b = xform_point(&world, positions[t[1] as usize]);
                        let c = xform_point(&world, positions[t[2] as usize]);
                        let n = (b - a).cross(c - a).norm();
                        let base = verts.len();
                        verts.push((a, n));
                        verts.push((b, n));
                        verts.push((c, n));
                        tris.push([base, base + 1, base + 2]);
                    }
                }
            }
            if !tris.is_empty() {
                out.push(Mesh { verts, tris, color });
            }
        }
    }

    for child in node.children() {
        visit_node(&child, &world, buffers, images, out);
    }
}

fn load_glb(path: &Path) -> Result<(Vec<Object>, String), String> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| format!("gagal membaca: {e}"))?;

    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or("file tidak punya scene")?;

    let mut meshes: Vec<Mesh> = Vec::new();
    for node in scene.nodes() {
        visit_node(&node, &IDENTITY, &buffers, &images, &mut meshes);
    }
    if meshes.is_empty() {
        return Err("tidak ada mesh segitiga di file ini".into());
    }

    // pusatkan model di origin dan skalakan agar dimensi terbesarnya = 3 unit
    let (mut lo, mut hi) = (V3::new(f32::MAX, f32::MAX, f32::MAX), V3::new(f32::MIN, f32::MIN, f32::MIN));
    for m in &meshes {
        for (p, _) in &m.verts {
            lo = V3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = V3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
    }
    let center = (lo + hi) * 0.5;
    let ext = (hi.x - lo.x).max(hi.y - lo.y).max(hi.z - lo.z);
    let scale = if ext > 1e-9 { 3.0 / ext } else { 1.0 };

    let mut tri_count = 0;
    let mut objects = Vec::new();
    for mut m in meshes {
        for v in m.verts.iter_mut() {
            v.0 = (v.0 - center) * scale;
        }
        tri_count += m.tris.len();
        objects.push(Object { mesh: m, offset: V3::new(0.0, 0.0, 0.0), tilt: 0.0, speed: 0.5 });
    }
    Ok((objects, format!("{GLB_NAME}: {tri_count} segitiga")))
}

fn find_glb() -> Result<PathBuf, String> {
    let local = PathBuf::from(GLB_NAME);
    if local.exists() {
        return Ok(local);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(GLB_NAME);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    Err(format!("{GLB_NAME} tidak ditemukan. Taruh di folder tempat `cargo run` dijalankan."))
}

// ---------------------------------------------------------------
// Thread render: UI tidak pernah menunggu proses render
// ---------------------------------------------------------------
struct Inner {
    params: Params,
    dirty: bool,
    new_model: Option<Vec<Object>>,
}

struct Shared {
    inner: Mutex<Inner>,
    cv: Condvar,
    pending: AtomicBool, // true = frame terakhir belum ditampilkan UI
    quit: AtomicBool,
}

impl Shared {
    fn request(&self, params: Params) {
        let mut g = self.inner.lock().unwrap();
        g.params = params;
        g.dirty = true;
        drop(g);
        self.cv.notify_one();
    }
}

fn make_shapes() -> Vec<Object> {
    vec![
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
    ]
}

fn spawn_render_thread(shared: Arc<Shared>, ui: Weak<MainWindow>) {
    thread::spawn(move || {
        let mut renderer = Renderer::new();
        let shapes = make_shapes();
        let mut model: Vec<Object> = Vec::new();

        loop {
            // jangan render lebih cepat dari UI bisa menampilkan
            while shared.pending.load(Ordering::Acquire) {
                if shared.quit.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(1));
            }

            let (params, new_model) = {
                let mut g = shared.inner.lock().unwrap();
                while !g.dirty {
                    if shared.quit.load(Ordering::Relaxed) {
                        return;
                    }
                    g = shared.cv.wait_timeout(g, Duration::from_millis(200)).unwrap().0;
                }
                g.dirty = false;
                (g.params, g.new_model.take())
            };
            if let Some(m) = new_model {
                model = m;
            }

            let objs: &[Object] = if params.show_model && !model.is_empty() { &model } else { &shapes };
            let ss = if params.fast { 1 } else { SS_MAX };
            let out = renderer.render(objs, &params, ss);
            let buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(out, W as u32, H as u32);

            shared.pending.store(true, Ordering::Release);
            let sh = shared.clone();
            let sent = ui.upgrade_in_event_loop(move |ui| {
                ui.set_frame(slint::Image::from_rgb8(buf));
                sh.pending.store(false, Ordering::Release);
            });
            if sent.is_err() {
                return; // aplikasi sudah ditutup
            }
        }
    });
}

fn load_model(ui: &MainWindow, shared: &Shared) {
    match find_glb().and_then(|p| load_glb(&p)) {
        Ok((objs, msg)) => {
            shared.inner.lock().unwrap().new_model = Some(objs);
            ui.set_model_loaded(true);
            ui.set_show_model(true);
            ui.set_model_status(msg.into());
        }
        Err(e) => {
            shared.inner.lock().unwrap().new_model = Some(Vec::new());
            ui.set_model_loaded(false);
            ui.set_show_model(false);
            ui.set_model_status(e.into());
        }
    }
}

// ---------------------------------------------------------------
// State sisi UI + GUI
// ---------------------------------------------------------------
struct UiState {
    yaw: f32,
    pitch: f32,
    base_yaw: f32,
    base_pitch: f32,
    spin: f32,
    last_interact: Instant,
    last_tick: Instant,
}

fn push(ui: &MainWindow, s: &UiState, shared: &Shared, fast: bool) {
    shared.request(Params {
        yaw: s.yaw,
        pitch: s.pitch,
        dist: ui.get_zoom(),
        spin: s.spin,
        size: ui.get_shape_size(),
        show_model: ui.get_show_model(),
        fast,
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    let now = Instant::now();
    let state = Rc::new(RefCell::new(UiState {
        yaw: 0.4,
        pitch: 0.3,
        base_yaw: 0.0,
        base_pitch: 0.0,
        spin: 0.0,
        last_interact: now,
        last_tick: now,
    }));

    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner {
            params: Params {
                yaw: 0.4,
                pitch: 0.3,
                dist: 8.5,
                spin: 0.0,
                size: 1.0,
                show_model: false,
                fast: false,
            },
            dirty: false,
            new_model: None,
        }),
        cv: Condvar::new(),
        pending: AtomicBool::new(false),
        quit: AtomicBool::new(false),
    });

    spawn_render_thread(shared.clone(), ui.as_weak());

    // coba buka target.glb saat start
    load_model(&ui, &shared);
    push(&ui, &state.borrow(), &shared, false);

    // "interaksi": render cepat (1x) dulu, lalu 150 ms setelah berhenti render halus (SS_MAX)
    let settle = Rc::new(Timer::default());
    let interact: Rc<dyn Fn()> = {
        let ui_weak = ui.as_weak();
        let st = state.clone();
        let sh = shared.clone();
        Rc::new(move || {
            if let Some(ui) = ui_weak.upgrade() {
                st.borrow_mut().last_interact = Instant::now();
                push(&ui, &st.borrow(), &sh, true);

                let (w2, s2, sh2) = (ui_weak.clone(), st.clone(), sh.clone());
                settle.start(TimerMode::SingleShot, Duration::from_millis(150), move || {
                    if let Some(ui) = w2.upgrade() {
                        push(&ui, &s2.borrow(), &sh2, false);
                    }
                });
            }
        })
    };

    ui.on_reload_model({
        let ui_weak = ui.as_weak();
        let sh = shared.clone();
        let interact = interact.clone();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                load_model(&ui, &sh);
                interact();
            }
        }
    });

    ui.on_drag_started({
        let st = state.clone();
        move || {
            let mut s = st.borrow_mut();
            s.base_yaw = s.yaw;
            s.base_pitch = s.pitch;
        }
    });

    ui.on_dragging({
        let st = state.clone();
        let interact = interact.clone();
        move |dx, dy| {
            {
                let mut s = st.borrow_mut();
                s.yaw = s.base_yaw + dx * 0.01;
                s.pitch = (s.base_pitch + dy * 0.01).clamp(-1.5, 1.5);
            }
            interact();
        }
    });

    ui.on_params_changed({
        let interact = interact.clone();
        move || interact()
    });

    ui.on_reset_view({
        let ui_weak = ui.as_weak();
        let st = state.clone();
        let interact = interact.clone();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_zoom(8.5);
                ui.set_shape_size(1.0);
                {
                    let mut s = st.borrow_mut();
                    s.yaw = 0.4;
                    s.pitch = 0.3;
                }
                interact();
            }
        }
    });

    // animasi auto-rotate: kecepatan berdasarkan waktu nyata, bukan jumlah frame
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        let sh = shared.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            let now = Instant::now();
            let dt = now.duration_since(s.last_tick).as_secs_f32().min(0.1);
            s.last_tick = now;
            if ui.get_auto_rotate() {
                s.spin += dt * 1.2;
                let fast = now.duration_since(s.last_interact) < Duration::from_millis(200);
                push(&ui, &s, &sh, fast);
            }
        }
    });

    let result = ui.run();
    shared.quit.store(true, Ordering::Relaxed);
    shared.cv.notify_all();
    result
}