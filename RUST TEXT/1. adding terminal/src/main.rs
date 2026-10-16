use std::io;

fn main() {
    let mut a = String::new();
    let mut b = String::new();

    println!("Masukkan nilai a:");
    io::stdin().read_line(&mut a).expect("Gagal membaca input");

    println!("Masukkan nilai b:");
    io::stdin().read_line(&mut b).expect("Gagal membaca input");

    let a: i32 = a.trim().parse().expect("Input harus berupa angka");
    let b: i32 = b.trim().parse().expect("Input harus berupa angka");

    println!("Hasil: {}", a + b);

    println!("Tekan Enter untuk keluar...");
    let mut exit = String::new();
    io::stdin().read_line(&mut exit).expect("Gagal membaca input");
}