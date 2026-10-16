use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Read;
use std::io::Write;
use std::rc::Rc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serialport::{ClearBuffer, DataBits, FlowControl, Parity, StopBits};
use slint::{Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

type Port = Box<dyn serialport::SerialPort>;

const BAUDS: [u32; 8] = [1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200];

// ======================= tipe data =======================

#[derive(Clone)]
struct PollCfg {
    slave: u8,
    fc: u8, // 1,2,3,4
    addr: u16,
    qty: u16,
    rate_ms: u64,
    fmt: i32,
}

#[derive(Clone)]
struct SerialCfg {
    port: String,
    baud: u32,
    data_bits: u8,
    parity: i32, // 0 none, 1 even, 2 odd
    stop_bits: u8,
    timeout_ms: u64,
}

enum Cmd {
    Write { slave: u8, fc: u8, addr: u16, value: u16 },
}

struct Session {
    tx: mpsc::Sender<Cmd>,
    poll: Arc<Mutex<PollCfg>>,
}

fn open_port(c: &SerialCfg) -> Result<Port, String> {
    if c.port.is_empty() {
        return Err("Port serial belum dipilih".into());
    }
    serialport::new(c.port.clone(), c.baud)
        .data_bits(if c.data_bits == 7 { DataBits::Seven } else { DataBits::Eight })
        .parity(match c.parity {
            1 => Parity::Even,
            2 => Parity::Odd,
            _ => Parity::None,
        })
        .stop_bits(if c.stop_bits == 2 { StopBits::Two } else { StopBits::One })
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(c.timeout_ms))
        .open()
        .map_err(|e| format!("gagal buka {}: {e}", c.port))
}

// ======================= protokol Modbus RTU =======================

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

fn read_rtu(port: &mut Port, unit: u8, lines: &mut Vec<String>) -> Result<Vec<u8>, String> {
    let e_io = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            "timeout (slave tidak menjawab)".to_string()
        } else {
            format!("baca gagal: {e}")
        }
    };
    let mut buf = vec![0u8; 2];
    port.read_exact(&mut buf).map_err(e_io)?;
    let rest = if buf[1] & 0x80 != 0 {
        3
    } else {
        match buf[1] {
            1..=4 => {
                let mut c = [0u8; 1];
                port.read_exact(&mut c).map_err(e_io)?;
                buf.push(c[0]);
                c[0] as usize + 2
            }
            5 | 6 | 15 | 16 => 6,
            _ => return Err("function code respons tidak dikenal".into()),
        }
    };
    let start = buf.len();
    buf.resize(start + rest, 0);
    port.read_exact(&mut buf[start..]).map_err(e_io)?;
    lines.push(format!("RX {}", hex(&buf)));

    let n = buf.len();
    let crc = crc16(&buf[..n - 2]);
    if buf[n - 2] != (crc & 0xFF) as u8 || buf[n - 1] != (crc >> 8) as u8 {
        return Err("CRC salah".into());
    }
    if buf[0] != unit {
        return Err("slave id respons tidak cocok".into());
    }
    Ok(buf[1..n - 2].to_vec())
}

/// Kirim PDU, kembalikan PDU respons.
fn transact(port: &mut Port, unit: u8, pdu: &[u8], lines: &mut Vec<String>) -> Result<Vec<u8>, String> {
    let mut frame = vec![unit];
    frame.extend_from_slice(pdu);
    let crc = crc16(&frame);
    frame.push((crc & 0xFF) as u8);
    frame.push((crc >> 8) as u8);

    lines.push(format!("TX {}", hex(&frame)));
    std::thread::sleep(Duration::from_millis(5)); // jeda antar frame
    let _ = port.clear(ClearBuffer::Input);
    port.write_all(&frame).map_err(|e| format!("kirim gagal: {e}"))?;
    port.flush().ok();
    read_rtu(port, unit, lines)
}

fn check_exception(pdu: &[u8], fc: u8) -> Result<(), String> {
    if pdu.is_empty() {
        return Err("respons kosong".into());
    }
    if pdu[0] == fc | 0x80 {
        let code = pdu.get(1).copied().unwrap_or(0);
        let name = match code {
            1 => "Illegal Function",
            2 => "Illegal Data Address",
            3 => "Illegal Data Value",
            4 => "Slave Device Failure",
            6 => "Slave Busy",
            _ => "Unknown",
        };
        return Err(format!("Exception {code:02X}: {name}"));
    }
    if pdu[0] != fc {
        return Err("function code respons tidak cocok".into());
    }
    Ok(())
}

fn read_values(port: &mut Port, cfg: &PollCfg, lines: &mut Vec<String>) -> Result<Vec<u16>, String> {
    let mut pdu = vec![cfg.fc];
    pdu.extend_from_slice(&cfg.addr.to_be_bytes());
    pdu.extend_from_slice(&cfg.qty.to_be_bytes());
    let resp = transact(port, cfg.slave, &pdu, lines)?;
    check_exception(&resp, cfg.fc)?;
    let count = *resp.get(1).ok_or("respons terpotong")? as usize;
    let data = resp.get(2..2 + count).ok_or("respons terpotong")?;
    if cfg.fc <= 2 {
        Ok((0..cfg.qty as usize)
            .map(|i| data.get(i / 8).map(|b| ((b >> (i % 8)) & 1) as u16).unwrap_or(0))
            .collect())
    } else {
        Ok(data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
    }
}

fn write_single(
    port: &mut Port,
    slave: u8,
    fc: u8,
    addr: u16,
    value: u16,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    // coil -> FC05, holding register -> FC06
    let (wfc, v) = if fc == 1 {
        (5u8, if value != 0 { 0xFF00u16 } else { 0 })
    } else {
        (6u8, value)
    };
    let mut pdu = vec![wfc];
    pdu.extend_from_slice(&addr.to_be_bytes());
    pdu.extend_from_slice(&v.to_be_bytes());
    let resp = transact(port, slave, &pdu, lines)?;
    check_exception(&resp, wfc)
}

// ======================= worker thread =======================

fn fmt_value(v: u16, fmt: i32, is_bit: bool) -> String {
    if is_bit {
        return v.to_string();
    }
    match fmt {
        1 => (v as i16).to_string(),
        2 => format!("0x{v:04X}"),
        3 => format!("{v:016b}"),
        _ => v.to_string(),
    }
}

fn ui_status(weak: &slint::Weak<MainWindow>, msg: String, ok: bool, connected: Option<bool>) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.set_status(msg.into());
        ui.set_status_ok(ok);
        if let Some(c) = connected {
            ui.set_connected(c);
        }
    });
}

fn push_log(log: &mut VecDeque<String>, start: Instant, lines: Vec<String>) {
    // terbaru di atas; urutan dalam 1 siklus tetap TX lalu RX
    for l in lines.into_iter().rev() {
        log.push_front(format!("{:8.3}  {}", start.elapsed().as_secs_f32(), l));
    }
    log.truncate(200);
}

fn update_rows(ui: &MainWindow, rows: Vec<Row>) {
    // update di tempat supaya posisi scroll tidak loncat
    let model = ui.get_rows();
    if let Some(vm) = model.as_any().downcast_ref::<VecModel<Row>>() {
        if vm.row_count() == rows.len() {
            for (i, r) in rows.into_iter().enumerate() {
                vm.set_row_data(i, r);
            }
            return;
        }
    }
    ui.set_rows(ModelRc::new(VecModel::from(rows)));
}

fn worker(
    scfg: SerialCfg,
    poll: Arc<Mutex<PollCfg>>,
    rx: mpsc::Receiver<Cmd>,
    weak: slint::Weak<MainWindow>,
) {
    let mut port = match open_port(&scfg) {
        Ok(p) => p,
        Err(e) => {
            ui_status(&weak, e, false, Some(false));
            return;
        }
    };
    ui_status(&weak, format!("Terhubung ke {}", scfg.port), true, None);

    let t_start = Instant::now();
    let mut log: VecDeque<String> = VecDeque::new();
    let (mut tx_n, mut err_n) = (0u64, 0u64);

    loop {
        let cfg = poll.lock().unwrap().clone();
        let mut lines = Vec::new();
        tx_n += 1;

        let t0 = Instant::now();
        let res = read_values(&mut port, &cfg, &mut lines);
        let ms = t0.elapsed().as_millis();

        let (status, ok, vals) = match res {
            Ok(v) => (format!("OK | {ms} ms | Tx {tx_n} | Error {err_n}"), true, Some(v)),
            Err(e) => {
                err_n += 1;
                lines.push(format!("ERR {e}"));
                (format!("{e} | Tx {tx_n} | Error {err_n}"), false, None)
            }
        };
        push_log(&mut log, t_start, lines);

        let log_vec: Vec<String> = log.iter().cloned().collect();
        let cfg2 = cfg.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !ui.get_connected() {
                return;
            }
            if let Some(v) = vals {
                let is_bit = cfg2.fc <= 2;
                let rows: Vec<Row> = v
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| {
                        let a = cfg2.addr as usize + i;
                        Row {
                            addr: a.to_string().into(),
                            addr_num: a as i32,
                            value: fmt_value(x, cfg2.fmt, is_bit).into(),
                            hex: (if is_bit { x.to_string() } else { format!("0x{x:04X}") }).into(),
                            raw: x as i32,
                        }
                    })
                    .collect();
                update_rows(&ui, rows);
            }
            ui.set_status(status.into());
            ui.set_status_ok(ok);
            let items: Vec<LogLine> = log_vec
                .into_iter()
                .map(|s| LogLine { err: s.contains("ERR"), text: SharedString::from(s) })
                .collect();
            ui.set_traffic(ModelRc::new(VecModel::from(items)));
        });

        // tunggu scan rate, tapi langsung bangun kalau ada perintah tulis
        match rx.recv_timeout(Duration::from_millis(cfg.rate_ms)) {
            Ok(Cmd::Write { slave, fc, addr, value }) => {
                let mut wl = Vec::new();
                match write_single(&mut port, slave, fc, addr, value, &mut wl) {
                    Ok(()) => wl.push(format!("WRITE OK addr {addr} = {value}")),
                    Err(e) => wl.push(format!("ERR WRITE: {e}")),
                }
                push_log(&mut log, t_start, wl);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break, // user menekan Disconnect
        }
    }
    // port ditutup otomatis saat `port` di-drop
}

// ======================= UI glue =======================

fn parse_num(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn read_cfg(ui: &MainWindow) -> PollCfg {
    // urutan combo: Coil, Discrete Input, Input Register, Holding Register
    let fc = [1u8, 2, 4, 3][ui.get_fc_index().clamp(0, 3) as usize];
    let max_q = if fc <= 2 { 2000 } else { 125 };
    PollCfg {
        slave: parse_num(&ui.get_slave_id()).unwrap_or(1).clamp(0, 247) as u8,
        fc,
        addr: parse_num(&ui.get_start_addr()).unwrap_or(0).clamp(0, 65535) as u16,
        qty: parse_num(&ui.get_quantity()).unwrap_or(10).clamp(1, max_q) as u16,
        rate_ms: parse_num(&ui.get_scan_rate()).unwrap_or(1000).clamp(20, 60000) as u64,
        fmt: ui.get_fmt_index(),
    }
}

fn read_serial_cfg(ui: &MainWindow) -> SerialCfg {
    SerialCfg {
        port: ui.get_port().to_string(),
        baud: BAUDS[ui.get_baud_index().clamp(0, 7) as usize],
        data_bits: if ui.get_databits_index() == 1 { 7 } else { 8 },
        parity: ui.get_parity_index(),
        stop_bits: if ui.get_stopbits_index() == 1 { 2 } else { 1 },
        timeout_ms: parse_num(&ui.get_timeout_ms()).unwrap_or(1000).clamp(50, 10000) as u64,
    }
}

fn refresh_ports(ui: &MainWindow) {
    let mut names: Vec<String> = serialport::available_ports()
        .map(|v| v.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default();
    names.sort();
    ui.set_port(names.first().cloned().unwrap_or_default().into());
    let model: Vec<SharedString> = names.into_iter().map(SharedString::from).collect();
    ui.set_ports(ModelRc::new(VecModel::from(model)));
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    refresh_ports(&ui);
    let session: Rc<RefCell<Option<Session>>> = Rc::new(RefCell::new(None));

    // ---- Refresh daftar port ----
    {
        let weak = ui.as_weak();
        ui.on_refresh_ports(move || refresh_ports(&weak.unwrap()));
    }

    // ---- Connect / Disconnect ----
    {
        let weak = ui.as_weak();
        let session = session.clone();
        ui.on_toggle_connect(move || {
            let ui = weak.unwrap();

            if ui.get_connected() {
                *session.borrow_mut() = None; // drop sender -> worker berhenti
                ui.set_connected(false);
                ui.set_status("Terputus".into());
                ui.set_status_ok(false);
                ui.set_rows(ModelRc::new(VecModel::from(Vec::<Row>::new())));
                return;
            }

            let scfg = read_serial_cfg(&ui);
            let poll = Arc::new(Mutex::new(read_cfg(&ui)));
            let (tx, rx) = mpsc::channel();
            let w = ui.as_weak();
            let p = poll.clone();
            std::thread::spawn(move || worker(scfg, p, rx, w));

            *session.borrow_mut() = Some(Session { tx, poll });
            ui.set_connected(true);
            ui.set_status("Menghubungkan...".into());
            ui.set_status_ok(false);
        });
    }

    // ---- parameter poll berubah (berlaku langsung) ----
    {
        let weak = ui.as_weak();
        let session = session.clone();
        ui.on_apply(move || {
            let ui = weak.unwrap();
            if let Some(s) = session.borrow().as_ref() {
                *s.poll.lock().unwrap() = read_cfg(&ui);
            }
        });
    }

    // ---- Write ----
    {
        let weak = ui.as_weak();
        let session = session.clone();
        ui.on_do_write(move || {
            let ui = weak.unwrap();
            let cfg = read_cfg(&ui);
            let fail = |msg: &str| {
                ui.set_status(msg.into());
                ui.set_status_ok(false);
            };
            if cfg.fc != 1 && cfg.fc != 3 {
                fail("Discrete Input & Input Register read-only (aturan Modbus). Pilih Coil atau Holding Register untuk menulis.");
                return;
            }
            let (Some(a), Some(v)) = (parse_num(&ui.get_write_addr()), parse_num(&ui.get_write_val()))
            else {
                fail("Alamat/nilai tulis tidak valid");
                return;
            };
            let value = if v < 0 { (v as i16) as u16 } else { v as u16 };
            match session.borrow().as_ref() {
                Some(s) => {
                    let _ = s.tx.send(Cmd::Write {
                        slave: cfg.slave,
                        fc: cfg.fc,
                        addr: a.clamp(0, 65535) as u16,
                        value,
                    });
                }
                None => fail("Belum terhubung"),
            }
        });
    }

    ui.run()
}
