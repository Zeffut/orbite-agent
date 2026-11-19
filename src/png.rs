//! Writing a picture as a PNG file, without a library for it.

use std::io::Write;

/// The PNG file of a picture `width` across, given as red, green and blue
/// for each point, row after row.
pub(crate) fn encode(width: u32, height: u32, points: &[u8]) -> Vec<u8> {
    let across = width as usize * 3;
    // Each row: no filter, then its points.
    let mut rows = Vec::with_capacity(points.len() + height as usize);
    for row in points.chunks(across.max(1)).take(height as usize) {
        rows.push(0);
        rows.extend_from_slice(row);
    }
    let mut packed = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = packed.write_all(&rows);
    let packed = packed.finish().unwrap_or_default();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits a channel, red green blue
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    for (kind, data) in [(b"IHDR", &header[..]), (b"IDAT", &packed[..]), (b"IEND", &[][..])] {
        file.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut check = flate2::Crc::new();
        check.update(kind);
        check.update(data);
        file.extend_from_slice(kind);
        file.extend_from_slice(data);
        file.extend_from_slice(&check.sum().to_be_bytes());
    }
    file
}
