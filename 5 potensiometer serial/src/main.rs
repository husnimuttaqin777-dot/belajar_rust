use std::io::{BufRead, BufReader, ErrorKind};
use std::thread;
use std::time::Duration;

slint::include_modules!();

const BAUD_RATE: u32 = 9600;

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;

    // Ganti sesuai port Arduino Anda
    let port_name: String = String::from("COM1");

    let weak = window.as_weak();
    thread::spawn(move || loop {
        match serialport::new(&port_name, BAUD_RATE)
            .timeout(Duration::from_millis(1000))
            .open()
        {
            Ok(port) => {
                let mut reader = BufReader::new(port);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if let Ok(v) = line.trim().parse::<i32>() {
                                let v = v.clamp(0, 1023) as f32;
                                let _ = weak.upgrade_in_event_loop(move |ui| {
                                    ui.set_analog_input(v);
                                });
                            }
                        }
                        Err(e) if e.kind() == ErrorKind::TimedOut => continue,
                        Err(_) => break,
                    }
                }
            }
            Err(e) => eprintln!("Gagal membuka {port_name}: {}", e.description),
        }
        thread::sleep(Duration::from_secs(2));
    });

    window.run()
}