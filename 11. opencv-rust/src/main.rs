
use opencv::{
    imgproc,
    prelude::*,
    videoio,
};

use slint::{Image, SharedPixelBuffer, Timer, TimerMode};

use std::{
    cell::Cell,
    rc::Rc,
    time::Duration,
};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Buat aplikasi GUI
    let ui = MainWindow::new()?;

    // Buka kamera indeks 0 (biasanya webcam utama)
    let mut camera =
        videoio::VideoCapture::new(0, videoio::CAP_ANY)?;

    if !camera.is_opened()? {
        return Err(
            "Webcam tidak dapat dibuka. Periksa kamera dan indeksnya."
                .into(),
        );
    }

    // Minta resolusi 640 x 480 sebagai konfigurasi awal
    camera.set(videoio::CAP_PROP_FRAME_WIDTH, 640.0)?;
    camera.set(videoio::CAP_PROP_FRAME_HEIGHT, 480.0)?;

    let running = Rc::new(Cell::new(false));

    // Tombol Start
    {
        let running = running.clone();
        let ui_weak = ui.as_weak();

        ui.on_start_camera(move || {
            running.set(true);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status("Kamera berjalan".into());
            }
        });
    }

    // Tombol Stop
    {
        let running = running.clone();
        let ui_weak = ui.as_weak();

        ui.on_stop_camera(move || {
            running.set(false);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status("Kamera dihentikan".into());
            }
        });
    }

    // Timer untuk memperbarui frame
    let ui_weak = ui.as_weak();
    let timer = Timer::default();

    timer.start(
        TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            // Jangan membaca frame ketika kamera di-stop
            if !running.get() {
                return;
            }

            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            let mut frame = Mat::default();

            // Ambil frame dari webcam
            match camera.read(&mut frame) {
                Ok(true) if !frame.empty() => {}
                Ok(_) => return,
                Err(error) => {
                    eprintln!("Gagal membaca kamera: {error}");
                    return;
                }
            }

            // OpenCV biasanya menghasilkan format BGR.
            // Slint membutuhkan format RGB.
            let mut rgb = Mat::default();

            if let Err(error) = imgproc::cvt_color(
                &frame,
                &mut rgb,
                imgproc::COLOR_BGR2RGB,
                0,
            ) {
                eprintln!("Konversi warna gagal: {error}");
                return;
            }

            let width = rgb.cols() as u32;
            let height = rgb.rows() as u32;

            // Salin data RGB ke buffer milik Slint
            let Ok(bytes) = rgb.data_bytes() else {
                return;
            };

            let mut buffer =
                SharedPixelBuffer::<slint::Rgb8Pixel>::new(
                    width,
                    height,
                );

            buffer.make_mut_bytes().copy_from_slice(bytes);

            // Tampilkan frame terbaru pada GUI
            ui.set_camera_image(Image::from_rgb8(buffer));

            ui.set_resolution(
                format!("Resolusi: {} x {}", width, height).into(),
            );
        },
    );

    // Timer harus tetap hidup selama aplikasi berjalan
    ui.run()?;

    Ok(())
}
