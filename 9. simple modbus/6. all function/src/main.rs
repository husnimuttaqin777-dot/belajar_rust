// ###### PROGRAM MEMANGGIL WINDOWS SLINT (versi Rust) ##########################

use rmodbus::{client::ModbusRequest, ModbusProto};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

slint::include_modules!();

//---------------DEKLARASI VARIABEL----------------------------//
static ANALOG: AtomicI32 = AtomicI32::new(0);

static BUTTON_STATUS: [AtomicI32; 4] = [
    AtomicI32::new(0),
    AtomicI32::new(0),
    AtomicI32::new(0),
    AtomicI32::new(0),
];

static INDICATOR_STATUS: [AtomicBool; 4] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

// -1 = belum ada data dari UI (di Python: slider = "")
static SLIDER: AtomicI32 = AtomicI32::new(-1);

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

// Respon fungsi write single (0x05 / 0x06): 8 byte, atau 5 byte kalau exception
fn transact_write(port: &mut Port, request: &[u8]) -> Result<Vec<u8>, BoxErr> {
    port.clear(serialport::ClearBuffer::Input)?;
    port.write_all(request)?;

    let mut response = vec![0u8; 2];
    port.read_exact(&mut response)?;
    let remaining = if response[1] & 0x80 != 0 { 3 } else { 6 };
    let mut rest = vec![0u8; remaining];
    port.read_exact(&mut rest)?;
    response.extend(rest);
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

// client.write_register(address, value, unit)
fn write_register(port: &mut Port, unit: u8, address: u16, value: u16) -> Result<(), BoxErr> {
    let mut mreq = ModbusRequest::new(unit, ModbusProto::Rtu);
    let mut request = Vec::new();
    mreq.generate_set_holding(address, value, &mut request)?;
    let response = transact_write(port, &request)?;
    mreq.parse_ok(&response)?;
    Ok(())
}

// client.write_coil(address, value, unit)
fn write_coil(port: &mut Port, unit: u8, address: u16, value: bool) -> Result<(), BoxErr> {
    let mut mreq = ModbusRequest::new(unit, ModbusProto::Rtu);
    let mut request = Vec::new();
    mreq.generate_set_coil(address, value, &mut request)?;
    let response = transact_write(port, &request)?;
    mreq.parse_ok(&response)?;
    Ok(())
}

//------MEMPROSES DATA MODBUS ------------------------------------//
fn modbus_data_process(mut port: Port, slave_id: u8) {
    loop {
        let request = read_holding_registers(&mut port, slave_id, 0, 0x7);
        let request_coil = read_coils(&mut port, slave_id, 0, 15);

        // setara try/except AttributeError: kalau pembacaan gagal, bagian ini dilewati
        if let Ok(registers) = &request {
            if let Some(&value) = registers.get(1) {
                ANALOG.store(value as i32, Ordering::SeqCst);
            }

            // client.write_register(0, slider*10)
            let slider = SLIDER.load(Ordering::SeqCst);
            if slider >= 0 {
                if let Ok(value) = u16::try_from(slider * 10) {
                    let _ = write_register(&mut port, slave_id, 0, value);
                }
            }

            println!("{:?}", registers);

            // indikator dari coil bits 8..11
            if let Ok(bits) = &request_coil {
                for i in 0..4 {
                    if let Some(&bit) = bits.get(8 + i) {
                        INDICATOR_STATUS[i].store(bit, Ordering::SeqCst);
                    }
                }
            }
        }

        // tombol -> coil 128..131
        for i in 0..4 {
            let on = BUTTON_STATUS[i].load(Ordering::SeqCst) == 1;
            let _ = write_coil(&mut port, slave_id, 128 + i as u16, on);
        }
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

    //---------------##### TOMBOL UI KE RUST #####------------//
    ui.on_button(|n, message| {
        if (1..=4).contains(&n) {
            BUTTON_STATUS[(n - 1) as usize].store(message, Ordering::SeqCst);
        }
    });

    ui.on_slider(|message| {
        SLIDER.store(message, Ordering::SeqCst);
    });

    ui.on_sensor(|| ANALOG.load(Ordering::SeqCst));

    ui.on_indicator_status(|n| {
        let on = (1..=4).contains(&n) && INDICATOR_STATUS[(n - 1) as usize].load(Ordering::SeqCst);
        if on { "on".into() } else { "off".into() }
    });

    ui.run().unwrap();
}
