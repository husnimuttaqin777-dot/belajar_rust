use std::thread;
use std::time::Duration;

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use slint::Weak;

slint::include_modules!();

// ---------------------------------------------------------------
// Konfigurasi MQTT (sama seperti main_1.py)
// ---------------------------------------------------------------
const BROKER: &str = "127.0.0.1";
// const BROKER: &str = "mqtt.ardumeka.com"; // atau "broker.emqx.io"
const PORT: u16 = 1883;
const CLIENT_ID: &str = "PC_1";
const TOPIC_SUB: &str = "sensor";
const TOPIC_PUB: &str = "led";

fn set_status(ui: &Weak<MainWindow>, text: &str) {
    let text = text.to_string();
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(text.into()));
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // ---- MQTT client (setara paho.Client + loop_start) ----
    let mut opts = MqttOptions::new(CLIENT_ID, BROKER, PORT);
    opts.set_keep_alive(Duration::from_secs(60));
    let (client, mut connection) = Client::new(opts, 16);

    println!("connecting to broker {BROKER}");

    // thread jaringan (setara client.loop_start + on_message)
    {
        let ui_weak = ui.as_weak();
        let sub_client = client.clone();
        thread::spawn(move || {
            for event in connection.iter() {
                match event {
                    // subscribe tiap kali (re)connect berhasil
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        println!("{BROKER} connected");
                        println!("Subscribing");
                        let _ = sub_client.try_subscribe(TOPIC_SUB, QoS::AtMostOnce);
                        set_status(&ui_weak, &format!("MQTT: connected ({BROKER}:{PORT})"));
                    }
                    // pesan masuk
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        if p.topic == TOPIC_SUB {
                            let msg = String::from_utf8_lossy(&p.payload).to_string();
                            println!("{msg}");
                            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                                ui.set_message(msg.into());
                            });
                        }
                    }
                    Ok(_) => {}
                    // putus / gagal connect -> coba lagi otomatis
                    Err(e) => {
                        eprintln!("MQTT error: {e}");
                        set_status(&ui_weak, "MQTT: disconnected, retrying...");
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        });
    }

    // ---- tombol LAMPU -> publish ke topic "led" ----
    ui.on_lamp_clicked(move |state| {
        println!("{state}");
        if let Err(e) = client.try_publish(TOPIC_PUB, QoS::AtMostOnce, false, state.to_string()) {
            eprintln!("gagal publish: {e}");
        }
    });

    ui.run()
}
