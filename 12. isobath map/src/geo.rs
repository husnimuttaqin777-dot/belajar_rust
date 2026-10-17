//! Proyeksi Web Mercator (EPSG:3857) untuk tile OSM berukuran 256 px.

use std::f64::consts::PI;

pub const TILE_SIZE: f64 = 256.0;
pub const MAX_LAT: f64 = 85.051_128_78;

/// (lat, lon) derajat -> koordinat piksel dunia pada zoom (boleh pecahan).
pub fn world_px(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let n = TILE_SIZE * 2f64.powf(zoom);
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * n;
    (x, y)
}

/// Kebalikan dari `world_px`.
pub fn from_world_px(x: f64, y: f64, zoom: f64) -> (f64, f64) {
    let n = TILE_SIZE * 2f64.powf(zoom);
    let lon = x / n * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lat, lon)
}

/// Meter per piksel pada lintang & zoom tertentu.
pub fn meters_per_px(lat: f64, zoom: f64) -> f64 {
    156_543.033_92 * lat.to_radians().cos() / 2f64.powf(zoom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (x, y) = world_px(-7.744, 109.0, 15.0);
        let (lat, lon) = from_world_px(x, y, 15.0);
        assert!((lat + 7.744).abs() < 1e-9 && (lon - 109.0).abs() < 1e-9);
    }

    #[test]
    fn meters_per_pixel_sanity() {
        // ~4.73 m/px pada zoom 15 di lintang -7.7
        let m = meters_per_px(-7.744, 15.0);
        assert!((m - 4.73).abs() < 0.05, "{m}");
    }
}
