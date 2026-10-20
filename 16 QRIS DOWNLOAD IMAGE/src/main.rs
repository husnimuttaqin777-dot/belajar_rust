use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, REFERER};
use std::fs;
use std::path::Path;

const PNG_SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const PNG_IEND: [u8; 12] = [0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];

fn download_image(client: &Client, url: &str, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.get(url).send()?.error_for_status()?;

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("?")
        .to_string();
    let expected = resp.content_length();
    let bytes = resp.bytes()?;

    println!(
        "URL: {url}\n  Content-Type: {content_type}\n  Content-Length: {expected:?}\n  Diterima: {} bytes",
        bytes.len()
    );

    if let Some(len) = expected {
        if len != bytes.len() as u64 {
            return Err("download terpotong: ukuran tidak sama dengan Content-Length".into());
        }
    }
    if bytes.len() < 20 || bytes[..8] != PNG_SIG {
        return Err("bukan file PNG (header salah, mungkin HTML/error)".into());
    }
    if bytes[bytes.len() - 12..] != PNG_IEND {
        return Err("PNG tidak lengkap: chunk IEND tidak ditemukan di akhir file".into());
    }
    // Pastikan benar-benar bisa di-decode
    image::load_from_memory(&bytes)?;

    // Tulis ke file sementara dulu, lalu rename, supaya tidak ada file setengah jadi
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0 Safari/537.36")
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(ACCEPT, "image/png,image/*;q=0.8,*/*;q=0.5".parse()?);
            h.insert(REFERER, "https://web.mayar.id/".parse()?);
            h
        })
        .build()?;

    let urls = [
        //"https://media.mayar.id/images/resized/480/26a1a805-d4fa-46fd-b6e7-75ad08758999.png",
        "https://media.mayar.id/images/26a1a805-d4fa-46fd-b6e7-75ad08758999.png", // tebakan
    ];

    // Simpan di sebelah exe
    let exe_dir = std::env::current_exe()?.parent().unwrap().to_path_buf();
    let out = exe_dir.join("gambar.png");
    let out = out.to_str().unwrap();

    for url in urls {
        match download_image(&client, url, out) {
            Ok(()) => {
                println!("Berhasil, tersimpan di {}", Path::new(out).display());
                return Ok(());
            }
            Err(e) => eprintln!("  Gagal: {e}\n"),
        }
    }

    Err("semua URL gagal menghasilkan PNG yang valid".into())
}