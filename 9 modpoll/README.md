# modpoll RTU (Rust + Slint)

Modbus RTU master (serial) ala Modbus Poll.
Tipe data: Coil (FC01/05), Discrete Input (FC02), Input Register (FC04), Holding Register (FC03/06).
Setting serial: port, baud, data bits, parity, stop bits, timeout.

    cargo run --release            # aplikasi poll (master)
    cargo run --release --bin slave   # simulator slave (GUI)

Tes tanpa alat: buat pasangan port virtual (com0com di Windows), mis. COM10 <-> COM11.
Slave: pilih COM11 di GUI-nya lalu Start. Master: pilih COM10, Connect.
Baud/data bits/parity/stop bits di kedua sisi harus sama.

Alamat basis 0 (alamat 0 = 40001 di Modbus Poll).
Discrete Input & Input Register read-only menurut spesifikasi Modbus.
