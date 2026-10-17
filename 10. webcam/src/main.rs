use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use slint::{Rgb8Pixel, SharedPixelBuffer, Weak};

slint::include_modules!();

// ---------------------------------------------------------------
// Worker kamera (setara CVCapture di Python)
// ---------------------------------------------------------------
struct CaptureWorker {
    stop: Arc<AtomicBool>,
    capture: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

fn report_error(ui: &Weak<MainWindow>, msg: String) {
    eprintln!("{msg}");
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_status(msg.into());
        ui.set_camera_on(false);
    });
}

impl CaptureWorker {
    fn start(index: u32, ui: Weak<MainWindow>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let capture = Arc::new(AtomicBool::new(false));
        let (stop_t, capture_t) = (stop.clone(), capture.clone());

        let handle = thread::spawn(move || {
            let fmt = RequestedFormat::new::<RgbFormat>(
                RequestedFormatType::AbsoluteHighestFrameRate,
            );
            let mut cam = match Camera::new(CameraIndex::Index(index), fmt) {
                Ok(c) => c,
                Err(e) => return report_error(&ui, format!("disconnect: {e}")),
            };
            if let Err(e) = cam.open_stream() {
                return report_error(&ui, format!("disconnect: {e}"));
            }
            println!("connect");

            // mencegah antrean frame menumpuk jika UI lebih lambat dari kamera
            let pending = Arc::new(AtomicBool::new(false));

            while !stop_t.load(Ordering::Relaxed) {
                let frame = match cam.frame() {
                    Ok(f) => f,
                    Err(e) => {
                        report_error(&ui, format!("read error: {e}"));
                        break;
                    }
                };
                let img = match frame.decode_image::<RgbFormat>() {
                    Ok(i) => i,
                    Err(e) => {
                        report_error(&ui, format!("decode error: {e}"));
                        break;
                    }
                };
                let (w, h) = (img.width() as usize, img.height() as usize);
                let rgb: &[u8] = img.as_raw();

                // tombol "capture" -> simpan PNG (nama: tanggal + jam)
                if capture_t.swap(false, Ordering::Relaxed) {
                    let name = chrono::Local::now()
                        .format("%Y-%m-%d %H-%M-%S %p.png")
                        .to_string();
                    match image::save_buffer(
                        &name,
                        rgb,
                        w as u32,
                        h as u32,
                        image::ColorType::Rgb8,
                    ) {
                        Ok(_) => println!("saved: {name}"),
                        Err(e) => eprintln!("gagal simpan: {e}"),
                    }
                }

                if pending.swap(true, Ordering::Relaxed) {
                    continue; // frame sebelumnya belum ditampilkan, lewati
                }

                // preview dibalik horizontal (efek cermin, seperti cv2.flip(.., 1))
                let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(w as u32, h as u32);
                let dst = buf.make_mut_bytes();
                for y in 0..h {
                    for x in 0..w {
                        let s = (y * w + x) * 3;
                        let d = (y * w + (w - 1 - x)) * 3;
                        dst[d..d + 3].copy_from_slice(&rgb[s..s + 3]);
                    }
                }

                let pending_ui = pending.clone();
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    ui.set_camera_image(slint::Image::from_rgb8(buf));
                    pending_ui.store(false, Ordering::Relaxed);
                });
            }
            let _ = cam.stop_stream();
        });

        Self { stop, capture, handle: Some(handle) }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------
// GUI (setara main.qml)
// ---------------------------------------------------------------
fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let worker: Rc<RefCell<Option<CaptureWorker>>> = Rc::new(RefCell::new(None));

    // "Camera Switch"
    ui.on_camera_toggled({
        let ui_weak = ui.as_weak();
        let worker = worker.clone();
        move |on, index| {
            if let Some(mut w) = worker.borrow_mut().take() {
                w.stop();
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_status("".into());
            }
            if on {
                *worker.borrow_mut() = Some(CaptureWorker::start(index as u32, ui_weak.clone()));
            }
        }
    });

    // tombol "capture"
    ui.on_capture_clicked({
        let worker = worker.clone();
        move || {
            if let Some(w) = worker.borrow().as_ref() {
                w.capture.store(true, Ordering::Relaxed);
            }
        }
    });

    ui.run()?;

    // jendela ditutup -> matikan kamera
    if let Some(mut w) = worker.borrow_mut().take() {
        w.stop();
    }
    Ok(())
}
