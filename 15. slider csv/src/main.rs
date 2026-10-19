use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;

slint::include_modules!();

const CSV_PATH: &str = "data.csv";

fn simpan(s1: f32, s2: f32, s3: f32) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(CSV_PATH)?;

    // Tulis header kalau file masih kosong
    if file.metadata()?.len() == 0 {
        writeln!(file, "waktu,slider1,slider2,slider3")?;
    }

    let waktu = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    writeln!(file, "{},{:.2},{:.2},{:.2}", waktu, s1, s2, s3)?;
    Ok(())
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let weak = ui.as_weak();

    ui.on_simpan(move || {
        let ui = weak.unwrap();
        let status = match simpan(ui.get_s1(), ui.get_s2(), ui.get_s3()) {
            Ok(_) => format!("Tersimpan ke {}", CSV_PATH),
            Err(e) => format!("Gagal menyimpan: {}", e),
        };
        ui.set_status(status.into());
    });

    ui.run()
}
