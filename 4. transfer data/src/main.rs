// Padanan main.py (class table + slot pyqtSlot) untuk versi Rust + Slint.
slint::include_modules!();
use std::thread;
use std::time::Duration;

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;

    // ---- nilai awal: padanan variabel global di main.py
    //      analog = 60, input1_color = "#df1c39", input2_color = "#df1c39"
    //      (padanan get_analog, get_input1_color, get_input2_color)
    window.set_analog_input(10.0);
    window.set_input1_color(slint::Color::from_rgb_u8(0xdf, 0x1c, 0x39));
    window.set_input2_color(slint::Color::from_rgb_u8(0xdf, 0x1c, 0x39));

    // ---- tombol QML -> Rust (padanan button1, button2, button3)
    window.on_button1(|pesan| println!("{pesan}"));
    window.on_button2(|pesan| println!("{pesan}"));
    window.on_button3(|pesan| println!("{pesan}"));

    // ---- slider QML -> Rust (padanan analog_output)
    window.on_analog_output(|nilai| println!("{nilai}"));
    
    // Cetak tick setiap satu detik
    const MIN: i32 = 0;
    const MAX: i32 = 100;
    let weak = window.as_weak();
    
    thread::spawn(move || {
    let mut nilai: i32 = 10;
    let mut arah: i32 = 1; // +1 naik, -1 turun

    loop {
        println!("tick {nilai}");

        let v = nilai as f32;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_analog_input(v);
        });

        // balik arah saat mencapai batas
        if nilai >= MAX {
            arah = -1;
        } else if nilai <= MIN {
            arah = 1;
        }
        nilai += arah;

        thread::sleep(Duration::from_secs(1));
    }
    });

    window.run()
}
