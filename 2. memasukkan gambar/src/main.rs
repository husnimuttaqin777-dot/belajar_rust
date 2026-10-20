slint::include_modules!();

fn load_image(path: &std::path::Path) -> Result<slint::Image, Box<dyn std::error::Error>> {
    let rgba = image::open(path)?.to_rgba8();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    );
    Ok(slint::Image::from_rgba8(buffer))
}

fn main() {
    let ui = MainWindow::new().unwrap();

    // Folder tempat .exe berada
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let path = exe_dir.join("gambar.png");

    match load_image(&path) {
        Ok(img) => ui.set_foto(img),
        Err(e) => eprintln!("Gagal memuat {}: {}", path.display(), e),
    }

    ui.run().unwrap();
}