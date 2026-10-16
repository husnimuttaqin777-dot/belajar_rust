use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const TICK_TOPIC: &str = "tick"; // ganti ke "namaanda/tick" di broker publik
const TICK_MESSAGE: &str = "yes";

use rumqttc::{Client, Event, MqttOptions, Outgoing, Packet, QoS};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;

    // Client MQTT aktif (hanya diakses dari thread UI)
    let client: Rc<RefCell<Option<Client>>> = Rc::new(RefCell::new(None));
    // ---- Hubungkan ----
    {
        let weak = window.as_weak();
        let client_cell = client.clone();

        window.on_connect_broker(move |host, port, sensor_topic| {
            let host = host.to_string();
            let sensor_topic = sensor_topic.to_string();

            // client id unik supaya tidak bentrok di broker publik
            let id = format!("rust-gui-{}", std::process::id());
            let mut opts = MqttOptions::new(id, host.clone(), port as u16);
            opts.set_keep_alive(Duration::from_secs(5));

            let (cli, mut connection) = Client::new(opts, 10);
            *client_cell.borrow_mut() = Some(cli.clone());

            if let Some(ui) = weak.upgrade() {
                ui.set_status(format!("Menghubungkan ke {host}:{port}...").into());
            }

            let weak = weak.clone();
            
            thread::spawn(move || {
                let stop_tick = Arc::new(AtomicBool::new(false));
                let mut tick_started = false;

                for event in connection.iter() {
                    match event {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        let _ = cli.try_subscribe(&sensor_topic, QoS::AtMostOnce);

                        // mulai loop tick (sekali saja, walau terjadi reconnect)
                        if !tick_started {
                            tick_started = true;
                            let tick_cli = cli.clone();
                            let stop = stop_tick.clone();
                            thread::spawn(move || {
                                while !stop.load(Ordering::Relaxed) {
                                    let _ = tick_cli.try_publish(
                                        TICK_TOPIC,
                                        QoS::AtMostOnce,
                                        false,
                                        TICK_MESSAGE,
                                    );

                                    let _ = tick_cli.try_publish(
                                        "hehehe",
                                        QoS::AtMostOnce,
                                        false,
                                        "hohoho",
                                    );
                                    
                                    
                                    thread::sleep(Duration::from_secs(1));
                                }
                            });

                        }
                        let t = sensor_topic.clone();
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            ui.set_connected(true);
                            ui.set_status(format!("Terhubung, subscribe: {t}").into());
                        });
                        }

                        // Pesan masuk dari topic sensor    
                        Ok(Event::Incoming(Packet::Publish(p))) => {
                            let text = String::from_utf8_lossy(&p.payload).to_string();
                            let _ = weak.upgrade_in_event_loop(move |ui| {
                                ui.set_sensor_value(text.into());
                            });
                        }

                        // Diputus oleh tombol "Putuskan"
                        Ok(Event::Outgoing(Outgoing::Disconnect)) => break,

                        // Gagal konek / koneksi putus
                        Err(e) => {
                            let msg = format!("Error: {e}");
                            let _ = weak.upgrade_in_event_loop(move |ui| {
                                ui.set_connected(false);
                                ui.set_status(msg.into());
                            });
                            break;
                        }

                        _ => {}
                    }
                }
            });
        });
    }

    // ---- Putuskan ----
    {
        let weak = window.as_weak();
        let client_cell = client.clone();
        window.on_disconnect_broker(move || {
            if let Some(cli) = client_cell.borrow_mut().take() {
                let _ = cli.try_disconnect();
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_connected(false);
                ui.set_sensor_value("-".into());
                ui.set_status("Terputus".into());
            }
        });
    }

    // ---- Publish LED ----
    {
        let weak = window.as_weak();
        let client_cell = client.clone();
        window.on_led_changed(move |topic, on| {
            let payload = if on { "ON" } else { "OFF" };
            let result = match client_cell.borrow().as_ref() {
                Some(cli) => cli
                    .try_publish(topic.as_str(), QoS::AtLeastOnce, false, payload)
                    .map_err(|e| e.to_string()),
                None => Err("belum terhubung".to_string()),
            };
            if let Some(ui) = weak.upgrade() {
                ui.set_status(match result {
                    Ok(()) => format!("Publish {topic}: {payload}"),
                    Err(e) => format!("Gagal publish: {e}"),
                }.into());
            }
        });
    }

    window.run()
}