//! Sensirion CRC-8 (polynomial 0x31).

/// CRC-8 with polynomial x^8 + x^5 + x^4 + 1. SHT3x uses `init = 0xFF`,
/// SHT2x uses `init = 0x00`.
pub fn crc8(data: &[u8], init: u8) -> u8 {
    let mut crc = init;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sht3x_datasheet_vector() {
        assert_eq!(crc8(&[0xBE, 0xEF], 0xFF), 0x92);
    }

    #[test]
    fn sht2x_datasheet_vector() {
        // SHT2x application note: 0x683A -> 0x7C
        assert_eq!(crc8(&[0x68, 0x3A], 0x00), 0x7C);
    }
}
