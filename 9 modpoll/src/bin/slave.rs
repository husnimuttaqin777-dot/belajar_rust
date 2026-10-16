// Simulator slave Modbus RTU dengan GUI (pengaturan serial di jendela, tanpa argumen CLI).
// Jalankan: cargo run --release --bin slave
use rand::Rng;
use serialport::{ClearBuffer, DataBits, FlowControl, Parity, StopBits};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

slint::slint! {
    import { Button, ComboBox, LineEdit, ListView } from "std-widgets.slint";

    component Field inherits VerticalLayout {
        in property <string> label;
        spacing: 2px;
        Text { text: root.label; color: #a6adc8; font-size: 11px; }
    }

    export component SlaveWindow inherits Window {
        title: "Modbus RTU Slave Simulator";
        width: 900px;
        height: 640px;
        background: #11111b;

        in-out property <string> port: "";
        in property <[string]> ports;
        in-out property <int> baud-index: 3;
        in-out property <int> databits-index: 0;
        in-out property <int> parity-index: 0;
        in-out property <int> stopbits-index: 0;
        in-out property <string> slave-id: "1";

        in-out property <bool> running: false;
        in property <string> status: "Berhenti";
        in property <bool> status-ok: false;
        in property <[string]> regs;
        in property <[string]> log;

        callback toggle();
        callback refresh-ports();

        VerticalLayout {
            padding: 12px;
            spacing: 10px;

            HorizontalLayout {
                spacing: 10px;
                Field {
                    label: "Port";
                    horizontal-stretch: 1;
                    ComboBox {
                        model: root.ports;
                        enabled: !root.running;
                        selected(v) => { root.port = v; }
                    }
                }
                Field {
                    label: " ";
                    width: 80px;
                    Button {
                        text: "Refresh";
                        enabled: !root.running;
                        clicked => { root.refresh-ports(); }
                    }
                }
                Field {
                    label: "Baud";
                    width: 110px;
                    ComboBox {
                        model: ["1200", "2400", "4800", "9600", "19200", "38400", "57600", "115200"];
                        current-index <=> root.baud-index;
                        enabled: !root.running;
                    }
                }
                Field {
                    label: "Data bits";
                    width: 80px;
                    ComboBox {
                        model: ["8", "7"];
                        current-index <=> root.databits-index;
                        enabled: !root.running;
                    }
                }
                Field {
                    label: "Parity";
                    width: 100px;
                    ComboBox {
                        model: ["None", "Even", "Odd"];
                        current-index <=> root.parity-index;
                        enabled: !root.running;
                    }
                }
                Field {
                    label: "Stop bits";
                    width: 80px;
                    ComboBox {
                        model: ["1", "2"];
                        current-index <=> root.stopbits-index;
                        enabled: !root.running;
                    }
                }
                Field {
                    label: "Slave ID";
                    width: 70px;
                    LineEdit { text <=> root.slave-id; enabled: !root.running; }
                }
                Field {
                    label: " ";
                    width: 110px;
                    Button {
                        text: root.running ? "Stop" : "Start";
                        primary: true;
                        clicked => { root.toggle(); }
                    }
                }
            }

            HorizontalLayout {
                spacing: 10px;
                vertical-stretch: 1;

                Rectangle {
                    width: 300px;
                    background: #181825;
                    border-radius: 6px;
                    clip: true;
                    VerticalLayout {
                        Rectangle {
                            height: 28px;
                            background: #313244;
                            Text {
                                text: "Register 0-29 (live)";
                                color: #cdd6f4;
                                font-weight: 700;
                                x: 8px;
                                vertical-alignment: center;
                            }
                        }
                        ListView {
                            for r in root.regs: Text {
                                text: r;
                                font-family: "monospace";
                                color: #cdd6f4;
                                height: 20px;
                            }
                        }
                    }
                }

                Rectangle {
                    horizontal-stretch: 1;
                    background: #181825;
                    border-radius: 6px;
                    clip: true;
                    VerticalLayout {
                        Rectangle {
                            height: 28px;
                            background: #313244;
                            Text {
                                text: "Trafik RX/TX";
                                color: #cdd6f4;
                                font-weight: 700;
                                x: 8px;
                                vertical-alignment: center;
                            }
                        }
                        ListView {
                            for line in root.log: Text {
                                text: line;
                                font-family: "monospace";
                                font-size: 11px;
                                color: #a6adc8;
                            }
                        }
                    }
                }
            }

            Text {
                text: root.status;
                color: root.status-ok ? #a6e3a1 : #f38ba8;
            }
        }
    }
}

const BAUDS: [u32; 8] = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200];

struct Store {
    regs: Vec<u16>,
    coils: Vec<bool>,
}

struct Shared {
    running: bool,
    status: String,
    ok: bool,
    log: VecDeque<String>,
    requests: u64,
    errors: u64,
}

struct SerialCfg {
    port: String,
    baud: u32,
    data_bits: u8,
    parity: i32,
    stop_bits: u8,
    unit: u8,
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn respond(pdu: &[u8], st: &Mutex<Store>) -> Vec<u8> {
    let fc = pdu[0];
    let exc = |c: u8| vec![fc | 0x80, c];
    let addr = u16::from_be_bytes([pdu[1], pdu[2]]) as usize;
    let val = u16::from_be_bytes([pdu[3], pdu[4]]);
    let mut g = st.lock().unwrap();
    match fc {
        1 | 2 => {
            let n = val as usize;
            if n == 0 || addr + n > g.coils.len() {
                return exc(2);
            }
            let mut bytes = vec![0u8; (n + 7) / 8];
            for i in 0..n {
                if g.coils[addr + i] {
                    bytes[i / 8] |= 1 << (i % 8);
                }
            }
            let mut r = vec![fc, bytes.len() as u8];
            r.extend(bytes);
            r
        }
        3 | 4 => {
            let n = val as usize;
            if n == 0 || addr + n > g.regs.len() {
                return exc(2);
            }
            let mut r = vec![fc, (n * 2) as u8];
            for i in 0..n {
                r.extend_from_slice(&g.regs[addr + i].to_be_bytes());
            }
            r
        }
        5 => {
            if addr >= g.coils.len() {
                return exc(2);
            }
            g.coils[addr] = val == 0xFF00;
            pdu[..5].to_vec()
        }
        6 => {
            if addr >= g.regs.len() {
                return exc(2);
            }
            g.regs[addr] = val;
            pdu[..5].to_vec()
        }
        _ => exc(1),
    }
}

fn log(sh: &Mutex<Shared>, t0: Instant, line: String) {
    let mut g = sh.lock().unwrap();
    g.log.push_front(format!("{:8.3}  {}", t0.elapsed().as_secs_f32(), line));
    g.log.truncate(200);
}

fn slave_thread(cfg: SerialCfg, st: Arc<Mutex<Store>>, sh: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) {
    let opened = serialport::new(cfg.port.clone(), cfg.baud)
        .data_bits(if cfg.data_bits == 7 { DataBits::Seven } else { DataBits::Eight })
        .parity(match cfg.parity {
            1 => Parity::Even,
            2 => Parity::Odd,
            _ => Parity::None,
        })
        .stop_bits(if cfg.stop_bits == 2 { StopBits::Two } else { StopBits::One })
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(200))
        .open();

    let mut port = match opened {
        Ok(p) => p,
        Err(e) => {
            let mut g = sh.lock().unwrap();
            g.running = false;
            g.ok = false;
            g.status = format!("gagal buka {}: {e}", cfg.port);
            return;
        }
    };

    let parity = ["N", "E", "O"][cfg.parity.clamp(0, 2) as usize];
    {
        let mut g = sh.lock().unwrap();
        g.ok = true;
        g.status = format!(
            "Berjalan di {} @ {} {}{}{} | slave id {}",
            cfg.port, cfg.baud, cfg.data_bits, parity, cfg.stop_bits, cfg.unit
        );
    }

    let t0 = Instant::now();
    // semua request yang didukung (FC01-06) panjangnya 8 byte
    while !stop.load(Ordering::Relaxed) {
        let mut req = [0u8; 8];
        match port.read_exact(&mut req) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::TimedOut => continue,
            Err(e) => {
                let mut g = sh.lock().unwrap();
                g.ok = false;
                g.status = format!("error baca: {e}");
                break;
            }
        }
        let crc = crc16(&req[..6]);
        if req[6] != (crc & 0xFF) as u8 || req[7] != (crc >> 8) as u8 {
            let _ = port.clear(ClearBuffer::Input);
            sh.lock().unwrap().errors += 1;
            log(&sh, t0, format!("ERR CRC salah: {}", hex(&req)));
            continue;
        }
        if req[0] != cfg.unit {
            continue; // bukan untuk kita
        }
        sh.lock().unwrap().requests += 1;
        log(&sh, t0, format!("RX {}", hex(&req)));

        let mut out = vec![cfg.unit];
        out.extend(respond(&req[1..6], &st));
        let c = crc16(&out);
        out.push((c & 0xFF) as u8);
        out.push((c >> 8) as u8);
        let _ = port.write_all(&out);
        let _ = port.flush();
        log(&sh, t0, format!("TX {}", hex(&out)));
    }

    sh.lock().unwrap().running = false;
}

fn refresh_ports(ui: &SlaveWindow) {
    let mut names: Vec<String> = serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default();
    names.sort();
    ui.set_port(names.first().cloned().unwrap_or_default().into());
    let model: Vec<SharedString> = names.into_iter().map(SharedString::from).collect();
    ui.set_ports(ModelRc::new(VecModel::from(model)));
}

fn main() -> Result<(), slint::PlatformError> {
    let st = Arc::new(Mutex::new(Store {
        regs: (0..200).map(|i| (i * 10) as u16).collect(),
        coils: (0..200).map(|i| i % 3 == 0).collect(),
    }));
    let sh = Arc::new(Mutex::new(Shared {
        running: false,
        status: "Berhenti".into(),
        ok: false,
        log: VecDeque::new(),
        requests: 0,
        errors: 0,
    }));

    // register 0..4 berubah acak, register 10 = counter
    {
        let st = st.clone();
        std::thread::spawn(move || {
            let mut rng = rand::thread_rng();
            let mut counter = 0u16;
            loop {
                {
                    let mut g = st.lock().unwrap();
                    for i in 0..5 {
                        g.regs[i] = rng.gen_range(0..1000);
                    }
                    counter = counter.wrapping_add(1);
                    g.regs[10] = counter;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    let ui = SlaveWindow::new()?;
    refresh_ports(&ui);

    {
        let weak = ui.as_weak();
        ui.on_refresh_ports(move || refresh_ports(&weak.unwrap()));
    }

    // ---- Start / Stop ----
    let stop_flag: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));
    {
        let weak = ui.as_weak();
        let st = st.clone();
        let sh = sh.clone();
        let stop_flag = stop_flag.clone();
        ui.on_toggle(move || {
            let ui = weak.unwrap();
            if ui.get_running() {
                if let Some(f) = stop_flag.borrow_mut().take() {
                    f.store(true, Ordering::Relaxed);
                }
                ui.set_running(false);
                let mut g = sh.lock().unwrap();
                g.ok = false;
                g.status = "Berhenti".into();
                return;
            }

            let cfg = SerialCfg {
                port: ui.get_port().to_string(),
                baud: BAUDS[ui.get_baud_index().clamp(0, 7) as usize],
                data_bits: if ui.get_databits_index() == 1 { 7 } else { 8 },
                parity: ui.get_parity_index(),
                stop_bits: if ui.get_stopbits_index() == 1 { 2 } else { 1 },
                unit: ui.get_slave_id().trim().parse().unwrap_or(1),
            };
            if cfg.port.is_empty() {
                let mut g = sh.lock().unwrap();
                g.ok = false;
                g.status = "Port serial belum dipilih".into();
                return;
            }

            let stop = Arc::new(AtomicBool::new(false));
            *stop_flag.borrow_mut() = Some(stop.clone());
            {
                let mut g = sh.lock().unwrap();
                g.running = true;
                g.ok = false;
                g.status = "Membuka port...".into();
                g.requests = 0;
                g.errors = 0;
                g.log.clear();
            }
            ui.set_running(true);
            let (st, sh) = (st.clone(), sh.clone());
            std::thread::spawn(move || slave_thread(cfg, st, sh, stop));
        });
    }

    // ---- refresh tampilan tiap 300 ms ----
    let timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let st = st.clone();
        let sh = sh.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(300), move || {
            let ui = weak.unwrap();

            let regs: Vec<SharedString> = {
                let g = st.lock().unwrap();
                g.regs
                    .iter()
                    .take(30)
                    .enumerate()
                    .map(|(i, v)| SharedString::from(format!("{i:>3}: {v:>5}  (0x{v:04X})")))
                    .collect()
            };
            ui.set_regs(ModelRc::new(VecModel::from(regs)));

            let g = sh.lock().unwrap();
            if ui.get_running() && !g.running {
                ui.set_running(false); // thread berhenti sendiri (error)
            }
            let status = if g.running {
                format!("{} | request {} | error {}", g.status, g.requests, g.errors)
            } else {
                g.status.clone()
            };
            ui.set_status(status.into());
            ui.set_status_ok(g.ok);
            let lines: Vec<SharedString> = g.log.iter().cloned().map(SharedString::from).collect();
            ui.set_log(ModelRc::new(VecModel::from(lines)));
        });
    }

    ui.run()
}
