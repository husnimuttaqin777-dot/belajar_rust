//! Data rute (waypoint) dan poligon isobath (pengganti `load_polygons()` + list di main.py).

use std::{collections::HashMap, error::Error};

#[derive(Debug, Clone)]
pub struct Waypoint {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
    pub color: &'static str,
}

#[derive(Debug, Clone)]
pub struct Polygon {
    /// (lat, lon)
    pub points: Vec<(f64, f64)>,
    pub kind: String,
    pub value: f64,
    /// Rata-rata titik, dipakai untuk posisi label "x m".
    pub center: (f64, f64),
}

fn build(prefix: &str, lat: &[f64], lon: &[f64], colors: &[&'static str]) -> Vec<Waypoint> {
    assert!(lat.len() == lon.len() && lat.len() == colors.len());
    (0..lat.len())
        .map(|i| Waypoint {
            lat: lat[i],
            lon: lon[i],
            name: format!("{prefix} WP{}", i + 1),
            color: colors[i],
        })
        .collect()
}

/// Jalur 1. Koordinat memakai presisi penuh dari blok komentar di main.qml.
///
/// Catatan: di main.py `rpl_lat` hanya berisi 12 nilai sedangkan `rpl_long`/`names`/`colors`
/// berisi 13, sehingga `zip()` menggeser lat/long dan memotong titik terakhir.
/// Di sini 13 titik dibuat konsisten.
pub fn route1() -> Vec<Waypoint> {
    build(
        "L1",
        &[
            -7.743054166666667, -7.743340555555555, -7.7435986111111115, -7.7439350000000005,
            -7.744126944444445, -7.744532777777778, -7.7447880555555555, -7.7453119444444445,
            -7.7453125, -7.745313055555556, -7.745313611111111, -7.745315833333334,
            -7.745431388888889,
        ],
        &[
            108.99883638888889, 108.99827944444445, 108.9977775, 108.99719388888889,
            108.99681166666667, 108.99582277777778, 108.99520277777778, 108.99388111111111,
            108.99387944444445, 108.9938775, 108.99387472222222, 108.9938638888889,
            108.99312222222223,
        ],
        &[
            "red", "orange", "green", "green", "green", "green", "orange", "orange", "red",
            "red", "red", "red", "red",
        ],
    )
}

pub fn route2() -> Vec<Waypoint> {
    build(
        "L2",
        &[
            -7.743055, -7.743775, -7.744123, -7.744329, -7.744441, -7.744598, -7.744754,
            -7.744933, -7.744985, -7.745072, -7.745401, -7.745446, -7.745456,
        ],
        &[
            108.998837, 108.997869, 108.997261, 108.996855, 108.996634, 108.996303, 108.995905,
            108.995424, 108.995217, 108.994877, 108.993900, 108.993432, 108.993341,
        ],
        &[
            "red", "navy", "navy", "navy", "navy", "navy", "navy", "navy", "navy", "orange",
            "red", "red", "red",
        ],
    )
}

/// Baca isobath.csv, kelompokkan per `ComponentId` (urutan kemunculan dipertahankan).
/// Baris yang gagal di-parse dilewati; poligon dengan < 3 titik dibuang.
pub fn load_polygons(path: &str) -> Result<Vec<Polygon>, Box<dyn Error>> {
    let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).from_path(path)?;
    let headers = rdr.headers()?.clone();
    let col = |name: &str| {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("kolom '{name}' tidak ada di {path}"))
    };
    let (c_id, c_type, c_lat, c_lon, c_val) = (
        col("ComponentId")?,
        col("Type")?,
        col("Latitude")?,
        col("Longitude")?,
        col("Value1")?,
    );

    let mut order: Vec<i64> = Vec::new();
    let mut comps: HashMap<i64, Polygon> = HashMap::new();

    for rec in rdr.records() {
        let rec = rec?;
        let (Ok(id), Ok(lat), Ok(lon)) = (
            rec[c_id].parse::<i64>(),
            rec[c_lat].parse::<f64>(),
            rec[c_lon].parse::<f64>(),
        ) else {
            continue;
        };
        let raw = &rec[c_val];
        let value = if raw.is_empty() {
            0.0
        } else {
            match raw.parse() {
                Ok(v) => v,
                Err(_) => continue,
            }
        };

        let poly = comps.entry(id).or_insert_with(|| {
            order.push(id);
            Polygon { points: vec![], kind: String::new(), value: 0.0, center: (0.0, 0.0) }
        });
        poly.points.push((lat, lon));
        poly.kind = rec[c_type].to_string();
        poly.value = value;
    }

    Ok(order
        .into_iter()
        .filter_map(|id| comps.remove(&id))
        .filter(|p| p.points.len() >= 3)
        .map(|mut p| {
            let n = p.points.len() as f64;
            p.center = (
                p.points.iter().map(|c| c.0).sum::<f64>() / n,
                p.points.iter().map(|c| c.1).sum::<f64>() / n,
            );
            p
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_sample_csv() {
        let p = load_polygons(concat!(env!("CARGO_MANIFEST_DIR"), "/isobath.csv")).unwrap();
        assert_eq!(p.len(), 43); // 44 komponen, 1 dibuang karena < 3 titik (sama dengan main.py)
        assert!(p.iter().all(|x| x.points.len() >= 3));
        assert!(p.iter().any(|x| x.kind.starts_with("Isobath_hole")));
    }

    #[test]
    fn routes_are_consistent() {
        assert_eq!(route1().len(), 13);
        assert_eq!(route2().len(), 13);
    }
}
