# Trenching Route (Rust + Slint)

Port dari `main.py` + `main.qml` (PyQt5/QtLocation).

    cargo run --release            # membaca ./isobath.csv
    cargo run --release -- path/ke/isobath.csv

Butuh Rust >= 1.91 dan koneksi internet (tile OpenStreetMap, di-cache ke `./tile_cache`).
Kontrol: drag = geser, scroll / double-click / tombol +/- = zoom.
