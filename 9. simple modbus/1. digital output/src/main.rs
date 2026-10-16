// ###### PROGRAM MEMANGGIL WINDOWS SLINT (versi Rust) ##########################

use rmodbus::{client::ModbusRequest, ModbusProto};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

slint::include_modules!();

//---------------DEKLARASI VARIABEL----------------------------//
static BUTTON1_STATUS: AtomicI32 = AtomicI32::new(0);

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

//------MEMPROSES DATA MODBUS ------------------------------------//
fn write_coil(
    port: &mut Box<dyn serialport::SerialPort>,
    unit: u8,
    address: u16,
    value: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut mreq = ModbusRequest::new(unit, ModbusProto::Rtu);
    let mut request = Vec::new();
    mreq.generate_set_coil(address, value, &mut request)?;
    port.write_all(&request)?;

    // Baca respon RTU: 8 byte (normal) atau 5 byte (exception)
    let mut response = vec![0u8; 2];
    port.read_exact(&mut response)?;
    let remaining = if response[1] & 0x80 != 0 { 3 } else { 6 };
    let mut rest = vec![0u8; remaining];
    port.read_exact(&mut rest)?;
    response.extend(rest);
    mreq.parse_ok(&response)?;
    Ok(())
}

fn modbus_data_process(mut port: Box<dyn serialport::SerialPort>, slave_id: u8) {
    loop {
        // note : jika button bernilai 1 maka rust mengirim nilai true ke alamat
        // modbus 128 alias B1 untuk menyalakan lampu, begitupun sebaliknya
        let state = BUTTON1_STATUS.load(Ordering::SeqCst) == 1;
        let _ = write_coil(&mut port, slave_id, 128, state);

    }
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

    // PROGRAM UNTUK MENERIMA DATA DARI UI
    ui.on_button1(|message| {
        let message: i32 = message.trim().parse().unwrap_or(0);
        //println!("{}", message);
        BUTTON1_STATUS.store(message, Ordering::SeqCst);
    });

    ui.run().unwrap();
}
