use slint::{CloseRequestResponse, ComponentHandle};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let main_win = MainWindow::new()?;
    let red = RedWindow::new()?; // wnd1
    let blue = BlueWindow::new()?; // wnd2

    // tombol kiri -> tampilkan window merah (wnd1.visible = true)
    main_win.on_show_red({
        let red = red.as_weak();
        move || {
            if let Some(w) = red.upgrade() {
                let _ = w.show();
            }
        }
    });

    // tombol kanan -> tampilkan window biru (wnd2.visible = true)
    main_win.on_show_blue({
        let blue = blue.as_weak();
        move || {
            if let Some(w) = blue.upgrade() {
                let _ = w.show();
            }
        }
    });

    // menutup window utama ikut menutup window merah & biru
    // (di QML keduanya adalah child dari window utama)
    main_win.window().on_close_requested({
        let red = red.as_weak();
        let blue = blue.as_weak();
        move || {
            if let Some(w) = red.upgrade() {
                let _ = w.hide();
            }
            if let Some(w) = blue.upgrade() {
                let _ = w.hide();
            }
            CloseRequestResponse::HideWindow
        }
    });

    main_win.run()
}
