//! Pengunduh tile OSM di thread latar belakang + cache disk.

use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const USER_AGENT: &str = "trenching-route-slint/0.1 (desktop app)";

fn cache_path(k: TileKey) -> PathBuf {
    PathBuf::from("tile_cache").join(format!("{}/{}/{}.png", k.z, k.x, k.y))
}

fn fetch(k: TileKey) -> Result<Decoded, String> {
    let path = cache_path(k);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let url = format!("https://tile.openstreetmap.org/{}/{}/{}.png", k.z, k.x, k.y);
            let resp = ureq::get(&url)
                .set("User-Agent", USER_AGENT)
                .call()
                .map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf).map_err(|e| e.to_string())?;
            if let Some(dir) = path.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&path, &buf);
            buf
        }
    };
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Decoded { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

/// Jalankan `workers` thread; kirim `TileKey` ke Sender yang dikembalikan.
/// `on_done` dipanggil dari thread worker (harus meneruskan ke event loop UI sendiri).
pub fn spawn_workers<F>(workers: usize, on_done: F) -> mpsc::Sender<TileKey>
where
    F: Fn(TileKey, Result<Decoded, String>) + Send + Sync + 'static,
{
    let (tx, rx) = mpsc::channel::<TileKey>();
    let rx = Arc::new(Mutex::new(rx));
    let on_done = Arc::new(on_done);
    for _ in 0..workers {
        let rx = rx.clone();
        let on_done = on_done.clone();
        thread::spawn(move || loop {
            let key = match rx.lock().unwrap().recv() {
                Ok(k) => k,
                Err(_) => break,
            };
            on_done(key, fetch(key));
        });
    }
    tx
}
