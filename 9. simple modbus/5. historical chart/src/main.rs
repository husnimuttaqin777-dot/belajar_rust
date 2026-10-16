// ###### PROGRAM MEMANGGIL WINDOWS SLINT (versi Rust) ##########################

use chrono::{Local, TimeZone};
use rmodbus::{client::ModbusRequest, ModbusProto};
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

slint::include_modules!();

//##########################################//
static ANALOG: AtomicI32 = AtomicI32::new(0);

// properti ChartView (cv)
const PERIOD_GRAPH: f64 = 30.0; // Seconds
const INTERVAL_TM: i64 = 200; // miliseconds

// area plot (harus sama dengan plot-x/y/w/h di main.slint)
const PLOT_W: f64 = 310.0;
const PLOT_H: f64 = 360.0;

//---------------SCAN AVAILABLE MODBUS PORT -------------------//
fn serial_ports() -> Vec<String> {
    // Setara serial_ports() di Python: hanya port yang bisa dibuka
    let mut result = Vec::new();
    if let Ok(ports) = serialport::available_ports() {
        for p in ports {
            if serialport::new(&p.port_name, 9600).open().is_ok() {
                result.push(p.port_name);
            }
        }
    }
    result
}

fn input(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut s = String::new();
    io::stdin().read_line(&mut s).unwrap();
    s.trim().to_string()
}

//------MODBUS RTU: kirim request, baca response------------------//
type Port = Box<dyn serialport::SerialPort>;
type BoxErr = Box<dyn std::error::Error>;

// Baca satu frame respon RTU untuk fungsi read (0x01 / 0x03)
fn transact(port: &mut Port, request: &[u8]) -> Result<Vec<u8>, BoxErr> {
    port.clear(serialport::ClearBuffer::Input)?;
    port.write_all(request)?;

    let mut response = vec![0u8; 2]; // unit + function
    port.read_exact(&mut response)?;

    if response[1] & 0x80 != 0 {
        // exception response: 1 byte kode + 2 byte CRC
        let mut rest = vec![0u8; 3];
        port.read_exact(&mut rest)?;
        response.extend(rest);
    } else {
        // byte count + data + 2 byte CRC
        let mut count = [0u8; 1];
        port.read_exact(&mut count)?;
        let mut rest = vec![0u8; count[0] as usize + 2];
        port.read_exact(&mut rest)?;
        response.push(count[0]);
        response.extend(rest);
    }
    Ok(response)
}

// client.read_coils(address, count, unit)
fn read_coils(port: &mut Port, unit: u8, address: u16, count: u16) -> Result<Vec<bool>, BoxErr> {
    let mut mreq = ModbusRequest::new(unit, ModbusProto::Rtu);
    let mut request = Vec::new();
    mreq.generate_get_coils(address, count, &mut request)?;
    let response = transact(port, &request)?;
    let mut bits = Vec::new();
    mreq.parse_bool(&response, &mut bits)?;
    Ok(bits)
}

// client.read_holding_registers(address, count, unit)
fn read_holding_registers(
    port: &mut Port,
    unit: u8,
    address: u16,
    count: u16,
) -> Result<Vec<u16>, BoxErr> {
    let mut mreq = ModbusRequest::new(unit, ModbusProto::Rtu);
    let mut request = Vec::new();
    mreq.generate_get_holdings(address, count, &mut request)?;
    let response = transact(port, &request)?;
    let mut regs = Vec::new();
    mreq.parse_u16(&response, &mut regs)?;
    Ok(regs)
}

//------MEMPROSES DATA MODBUS ------------------------------------//
fn modbus_data_process(mut port: Port, slave_id: u8) {
    loop {
        let request = read_holding_registers(&mut port, slave_id, 0, 0x7);
        let _request_coil = read_coils(&mut port, slave_id, 0, 15);

        // setara try/except: kalau gagal, lewati saja
        if let Ok(registers) = request {
            if let Some(&value) = registers.first() {
                ANALOG.store(value as i32, Ordering::SeqCst);
            }
        }
    }
}

//------DATA CHART (setara tm.onTriggered di QML)------------------//
struct Chart {
    timcnt: i64,
    start_time: i64,                // ms
    points: VecDeque<(i64, f64)>,   // (waktu ms, nilai)
}

// backend.get_tiempo(): detik sejak epoch
fn get_tiempo() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

// format "HH:mm:ss.z" (z = milidetik tanpa nol di depan), waktu lokal
fn format_time(ms: i64) -> String {
    let dt = Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now);
    format!("{}.{}", dt.format("%H:%M:%S"), dt.timestamp_subsec_millis())
}

fn refresh(ui: &MainWindow, c: &Chart) {
    let window_ms = (PERIOD_GRAPH * 100.0) as i64; // periodGRAPH*100
    let tmin = c.start_time - window_ms + c.timcnt * INTERVAL_TM;
    let tmax = c.start_time + c.timcnt * INTERVAL_TM;

    // garis series
    let mut commands = String::new();
    for (i, (t, v)) in c.points.iter().enumerate() {
        let x = (*t - tmin) as f64 / window_ms as f64 * PLOT_W;
        let y = PLOT_H - v / 100.0 * PLOT_H;
        commands.push_str(&format!("{}{:.2} {:.2} ", if i == 0 { "M" } else { "L" }, x, y));
    }
    ui.set_line_commands(SharedString::from(commands));

    // label sumbu X (5 tick)
    let labels: Vec<SharedString> = (0..5)
        .map(|i| SharedString::from(format_time(tmin + i * (tmax - tmin) / 4)))
        .collect();
    ui.set_x_labels(ModelRc::new(VecModel::from(labels)));
}

//########## main ######################//
fn main() {
    println!("{:?}", serial_ports());

    let port_name = input("modbus port : ");
    let baudrate = input("modbus baudrate : ");
    let slave_id = input("slave id : ");

    let port = serialport::new(&port_name, baudrate.parse::<u32>().unwrap())
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_secs(4))
        .open()
        .expect("gagal membuka port modbus");

    let slave_id: u8 = slave_id.parse().unwrap();
    let _t1 = thread::spawn(move || modbus_data_process(port, slave_id));

    let ui = MainWindow::new().unwrap();

    // Component.onCompleted: cv.startTIME = backend.get_tiempo()*1000
    let chart = Rc::new(RefCell::new(Chart {
        timcnt: 0,
        start_time: get_tiempo() * 1000,
        points: VecDeque::new(),
    }));
    refresh(&ui, &chart.borrow());

    // tm.onTriggered
    let ui_weak = ui.as_weak();
    let chart_cb = chart.clone();
    ui.on_tick(move || {
        let ui = ui_weak.unwrap();
        let mut c = chart_cb.borrow_mut();

        c.timcnt += 1;
        let value_ch1 = ANALOG.load(Ordering::SeqCst) as f64; // backend.sensor()

        // if (lines1.count > periodGRAPH*100/intervalTM) lines1.remove(0)
        if c.points.len() as f64 > PERIOD_GRAPH * 100.0 / INTERVAL_TM as f64 {
            c.points.pop_front();
        }

        // lines1.append(startTIME + timcnt*intervalTM, valueCH1)
        let t = c.start_time + c.timcnt * INTERVAL_TM;
        c.points.push_back((t, value_ch1));

        refresh(&ui, &c);
    });

    ui.run().unwrap();
}
