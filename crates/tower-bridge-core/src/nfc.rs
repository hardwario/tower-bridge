//! NFC Tag: an NXP NTAG I2C plus (NT3H2111, or NT3H2211 with 2k) whose NDEF
//! memory holds a URL that a phone opens when it touches the tag.
//!
//! Over I2C the memory is read and written in 16-byte blocks. Block 0 holds the
//! I2C address, the serial number, the static lock bytes and the capability
//! container (CC); it is never written here, since a wrong byte there moves
//! the chip to another address or locks it for good. The NDEF message lives in
//! user memory from block 1 on, wrapped in a TLV:
//! `03 <length> <message> FE`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::bus::{BusError, Clock, I2cBus};
use crate::climate::select_channel;

/// Addresses to look for an NFC Tag at: HARDWARIO sets its tags to 0x08 (as
/// its firmware SDK expects); 0x55 is the chip's factory default. The address
/// lives in block 0, so it survives power cycles.
pub const ADDRESSES: [u8; 2] = [0x08, 0x55];
const BLOCK: usize = 16;
const USER_START: u8 = 0x01;
/// User memory of the 1k part (blocks 0x01 to 0x37); the blocks after it hold
/// dynamic locks and configuration, so writing stops before them on any part.
const USER_BYTES: usize = 0x37 * BLOCK;
/// Longest URL accepted, in UTF-8 bytes: the record stays a short record.
pub const MAX_URL: usize = 240;
/// EEPROM programming of one block (the chip ignores its address meanwhile).
const WRITE_TIME: Duration = Duration::from_millis(5);

/// URI identifier codes of the NFC Forum URI record type: a code replaces the
/// prefix it stands for, saving tag memory.
const PREFIXES: [&str; 36] = [
    "",
    "http://www.",
    "https://www.",
    "http://",
    "https://",
    "tel:",
    "mailto:",
    "ftp://anonymous:anonymous@",
    "ftp://ftp.",
    "ftps://",
    "sftp://",
    "smb://",
    "nfs://",
    "ftp://",
    "dav://",
    "news:",
    "telnet://",
    "imap:",
    "rtsp://",
    "urn:",
    "pop:",
    "sip:",
    "sips:",
    "tftp:",
    "btspp://",
    "btl2cap://",
    "btgoep://",
    "tcpobex://",
    "irdaobex://",
    "file://",
    "urn:epc:id:",
    "urn:epc:tag:",
    "urn:epc:pat:",
    "urn:epc:raw:",
    "urn:epc:",
    "urn:nfc:",
];

#[derive(Debug, thiserror::Error)]
pub enum NfcError {
    #[error("the URL is empty")]
    Empty,
    #[error("the URL is longer than {MAX_URL} bytes")]
    TooLong,
    #[error("the TOWER NFC Tag is not formatted for NDEF")]
    NotFormatted,
    #[error("the TOWER NFC Tag did not keep the URL")]
    NotWritten,
    #[error(transparent)]
    Bus(#[from] BusError),
}

/// What the app knows about an NFC Tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NfcInfo {
    pub channel: u8,
    pub address: u8,
    pub part: String,
    /// The URL on the tag; `None` when it holds no URL (or something else).
    pub url: Option<String>,
    /// NDEF formatted, so a URL can be written.
    pub formatted: bool,
}

pub struct NfcTag {
    channel: u8,
    address: u8,
    part: &'static str,
    /// NDEF data area in bytes, from the capability container (0 when not formatted).
    capacity: usize,
    url: Option<String>,
}

fn absent_on_nack<T>(r: Result<T, BusError>) -> Result<Option<T>, BusError> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_fatal() => Err(e),
        Err(e) => {
            debug!("NFC probe: {e}");
            Ok(None)
        }
    }
}

/// One block. The memory address goes in its own write: the chip wants a STOP
/// before the read (no repeated start).
fn read_block(bus: &mut dyn I2cBus, addr: u8, block: u8) -> Result<[u8; BLOCK], BusError> {
    let mut b = [0u8; BLOCK];
    bus.write(addr, &[block])?;
    bus.read(addr, &mut b)?;
    Ok(b)
}

/// Which NTAG I2C plus answers at `addr`, if any: block 0 starts with the NXP
/// manufacturer ID, and the capability container tells 1k from 2k.
pub fn identify(bus: &mut dyn I2cBus, addr: u8) -> Result<Option<&'static str>, BusError> {
    Ok(absent_on_nack(read_block(bus, addr, 0))?.and_then(|b| part(&b)))
}

fn part(block0: &[u8; BLOCK]) -> Option<&'static str> {
    (block0[0] == 0x04).then_some(if block0[14] == 0xEA {
        "NT3H2211"
    } else {
        "NT3H2111"
    })
}

impl NfcTag {
    /// Look for an NFC Tag on the given switch channels and read its URL.
    pub fn detect(bus: &mut dyn I2cBus, channels: &[u8]) -> Result<Option<Self>, BusError> {
        for &channel in channels {
            if absent_on_nack(select_channel(bus, channel))?.is_none() {
                continue;
            }
            let mut found = None;
            for address in ADDRESSES {
                if let Some(block0) = absent_on_nack(read_block(bus, address, 0))?
                    && let Some(part) = part(&block0)
                {
                    found = Some((address, block0, part));
                    break;
                }
            }
            let Some((address, block0, part)) = found else {
                continue;
            };
            // CC: magic E1, version, data area size / 8, access.
            let capacity = if block0[12] == 0xE1 {
                (usize::from(block0[14]) * 8).min(USER_BYTES)
            } else {
                0
            };
            let mut tag = Self {
                channel,
                address,
                part,
                capacity,
                url: None,
            };
            if capacity > 0 {
                tag.url = absent_on_nack(tag.read_url(bus))?.flatten();
            }
            return Ok(Some(tag));
        }
        Ok(None)
    }

    pub fn info(&self) -> NfcInfo {
        NfcInfo {
            channel: self.channel,
            address: self.address,
            part: self.part.into(),
            url: self.url.clone(),
            formatted: self.capacity > 0,
        }
    }

    /// The URL in the NDEF message, reading only as many blocks as it needs.
    fn read_url(&self, bus: &mut dyn I2cBus) -> Result<Option<String>, BusError> {
        let mut mem = Vec::new();
        loop {
            match ndef_extent(&mem) {
                Extent::Ndef(start, end) => return Ok(mem.get(start..end).and_then(parse_uri)),
                Extent::Empty => return Ok(None),
                // Beyond the data area: not a message of ours.
                Extent::Need(n) if n > self.capacity => return Ok(None),
                Extent::Need(n) => {
                    while mem.len() < n {
                        let block = USER_START + u8::try_from(mem.len() / BLOCK).unwrap_or(u8::MAX);
                        mem.extend_from_slice(&read_block(bus, self.address, block)?);
                    }
                }
            }
        }
    }

    /// Write `url` as the tag's only NDEF record and read it back.
    pub fn write_url(
        &mut self,
        bus: &mut dyn I2cBus,
        clock: &dyn Clock,
        url: &str,
    ) -> Result<(), NfcError> {
        if self.capacity == 0 {
            return Err(NfcError::NotFormatted);
        }
        let bytes = ndef_tlv(url)?;
        if bytes.len() > self.capacity {
            return Err(NfcError::TooLong);
        }
        select_channel(bus, self.channel)?;
        for (i, chunk) in bytes.chunks(BLOCK).enumerate() {
            let mut data = [0u8; BLOCK + 1];
            data[0] = USER_START + u8::try_from(i).unwrap_or(u8::MAX);
            data[1..=chunk.len()].copy_from_slice(chunk);
            write_block(bus, clock, self.address, &data)?;
        }
        self.url = self.read_url(bus)?;
        if self.url.as_deref() == Some(url) {
            Ok(())
        } else {
            Err(NfcError::NotWritten)
        }
    }
}

/// Write one block; while the previous one is still being programmed the
/// chip does not acknowledge, so try again a few times.
fn write_block(
    bus: &mut dyn I2cBus,
    clock: &dyn Clock,
    addr: u8,
    data: &[u8],
) -> Result<(), BusError> {
    let mut attempts = 0;
    loop {
        match bus.write(addr, data) {
            Ok(()) => {
                clock.sleep(WRITE_TIME);
                return Ok(());
            }
            Err(BusError::AddrNack) if attempts < 3 => {
                attempts += 1;
                clock.sleep(WRITE_TIME);
            }
            Err(e) => return Err(e),
        }
    }
}

/// An NDEF message with one URI record, as a TLV with the terminator.
pub fn ndef_tlv(url: &str) -> Result<Vec<u8>, NfcError> {
    if url.trim().is_empty() {
        return Err(NfcError::Empty);
    }
    if url.len() > MAX_URL {
        return Err(NfcError::TooLong);
    }
    // The longest prefix with a code (code 0 is the empty prefix).
    let (code, rest) = PREFIXES
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, p)| url.starts_with(**p))
        .max_by_key(|(_, p)| p.len())
        .map_or((0u8, url), |(i, p)| {
            (u8::try_from(i).unwrap_or(0), &url[p.len()..])
        });
    let payload_len = u8::try_from(rest.len() + 1).map_err(|_| NfcError::TooLong)?;
    // Message begin and end, short record, well-known type "U".
    let mut message = vec![0xD1, 0x01, payload_len, b'U', code];
    message.extend_from_slice(rest.as_bytes());
    let mut tlv = vec![0x03];
    match u8::try_from(message.len()) {
        Ok(n) if n < 0xFF => tlv.push(n),
        _ => {
            let n = u16::try_from(message.len()).map_err(|_| NfcError::TooLong)?;
            tlv.push(0xFF);
            tlv.extend_from_slice(&n.to_be_bytes());
        }
    }
    tlv.extend_from_slice(&message);
    tlv.push(0xFE);
    Ok(tlv)
}

enum Extent {
    /// The NDEF message is at these bytes.
    Ndef(usize, usize),
    /// No NDEF message before the terminator.
    Empty,
    /// More bytes are needed to tell.
    Need(usize),
}

/// Where the NDEF message is in user memory, skipping NULL, lock and memory
/// control TLVs.
fn ndef_extent(mem: &[u8]) -> Extent {
    let mut i = 0;
    loop {
        let Some(&t) = mem.get(i) else {
            return Extent::Need(i + 1);
        };
        match t {
            0x00 => i += 1,
            0xFE => return Extent::Empty,
            _ => {
                let Some(&l) = mem.get(i + 1) else {
                    return Extent::Need(i + 2);
                };
                let (len, header) = if l == 0xFF {
                    match (mem.get(i + 2), mem.get(i + 3)) {
                        (Some(&a), Some(&b)) => (usize::from(u16::from_be_bytes([a, b])), 4),
                        _ => return Extent::Need(i + 4),
                    }
                } else {
                    (usize::from(l), 2)
                };
                let end = i + header + len;
                if t == 0x03 {
                    return if mem.len() >= end {
                        Extent::Ndef(i + header, end)
                    } else {
                        Extent::Need(end)
                    };
                }
                i = end;
            }
        }
    }
}

/// The URL of the message's first record, when that is a URI record.
fn parse_uri(message: &[u8]) -> Option<String> {
    let header = *message.first()?;
    let type_len = usize::from(*message.get(1)?);
    let short = header & 0x10 != 0;
    let has_id = header & 0x08 != 0;
    let (payload_len, mut pos) = if short {
        (usize::from(*message.get(2)?), 3)
    } else {
        let b = message.get(2..6)?;
        (
            usize::try_from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok()?,
            6,
        )
    };
    let id_len = if has_id {
        pos += 1;
        usize::from(*message.get(pos - 1)?)
    } else {
        0
    };
    let record_type = message.get(pos..pos + type_len)?;
    pos += type_len + id_len;
    let payload = message.get(pos..pos + payload_len)?;
    // Well-known type "U".
    if header & 0x07 != 0x01 || record_type != b"U" {
        return None;
    }
    let (&code, rest) = payload.split_first()?;
    let prefix = PREFIXES.get(usize::from(code)).copied().unwrap_or("");
    Some(format!("{prefix}{}", String::from_utf8_lossy(rest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::mock::{MockBus, MockClock, SimNtag};

    fn bus_with_tag(channel: usize, tag: SimNtag) -> MockBus {
        let mut bus = MockBus::new();
        bus.attach(channel, ADDRESSES[0], Box::new(tag));
        bus
    }

    #[test]
    fn encodes_a_url_with_its_prefix_code() -> Result<(), NfcError> {
        let tlv = ndef_tlv("https://www.hardwario.com")?;
        let mut expected = vec![0x03, 18, 0xD1, 0x01, 14, b'U', 0x02];
        expected.extend_from_slice(b"hardwario.com");
        expected.push(0xFE);
        assert_eq!(tlv, expected);
        assert_eq!(ndef_tlv("https://x.io")?[6], 0x04, "https:// without www.");
        assert_eq!(ndef_tlv("geo:50,14")?[6], 0x00, "no prefix code");
        Ok(())
    }

    #[test]
    fn rejects_empty_and_long_urls() {
        assert!(matches!(ndef_tlv(" "), Err(NfcError::Empty)));
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert!(matches!(ndef_tlv(&long), Err(NfcError::TooLong)));
    }

    #[test]
    fn reads_back_what_it_encodes() -> Result<(), NfcError> {
        for url in [
            "https://www.hardwario.com/tower",
            "tel:+420123456789",
            "geo:50,14",
            "https://ä.example/ü",
        ] {
            let tlv = ndef_tlv(url)?;
            match ndef_extent(&tlv) {
                Extent::Ndef(start, end) => {
                    assert_eq!(parse_uri(&tlv[start..end]).as_deref(), Some(url))
                }
                _ => panic!("no message in {tlv:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn skips_other_tlvs_and_ignores_other_records() {
        // A NULL TLV and a lock control TLV before the message.
        let mut mem = vec![0x00, 0x01, 0x03, 0xA0, 0x10, 0x44];
        mem.extend_from_slice(&[0x03, 6, 0xD1, 0x01, 2, b'U', 0x04, b'x', 0xFE]);
        match ndef_extent(&mem) {
            Extent::Ndef(s, e) => assert_eq!(parse_uri(&mem[s..e]).as_deref(), Some("https://x")),
            _ => panic!("message not found"),
        }
        // A text record is not a URL.
        let text = [0xD1, 0x01, 3, b'T', 0x02, b'e', b'n'];
        assert_eq!(parse_uri(&text), None);
        assert!(matches!(
            ndef_extent(&[0x03, 0x00, 0xFE]),
            Extent::Ndef(2, 2)
        ));
        assert!(matches!(ndef_extent(&[0xFE]), Extent::Empty));
        assert!(matches!(ndef_extent(&[0x03, 40]), Extent::Need(42)));
    }

    #[test]
    fn detects_a_fresh_tag_and_writes_a_url() -> Result<(), NfcError> {
        let tag = SimNtag::new();
        let memory = tag.memory();
        let mut bus = bus_with_tag(1, tag);
        let clock = MockClock::new();
        let Some(mut nfc) = NfcTag::detect(&mut bus, &[0, 1])? else {
            panic!("tag not found");
        };
        let info = nfc.info();
        assert_eq!(
            (info.channel, info.part.as_str(), info.formatted),
            (1, "NT3H2111", true)
        );
        assert_eq!(info.url, None, "a fresh tag holds an empty message");

        nfc.write_url(&mut bus, &clock, "https://www.hardwario.com")?;
        assert_eq!(nfc.info().url.as_deref(), Some("https://www.hardwario.com"));
        let mem = memory.lock().map(|m| m.clone()).unwrap_or_default();
        assert_eq!(&mem[16..23], &[0x03, 18, 0xD1, 0x01, 14, b'U', 0x02]);
        assert_eq!(&mem[0..1], &[0x04], "block 0 untouched");

        // A later detection reads it from the tag.
        let again = NfcTag::detect(&mut bus, &[0, 1])?.map(|t| t.info().url);
        assert_eq!(again, Some(Some("https://www.hardwario.com".into())));
        Ok(())
    }

    #[test]
    fn an_unformatted_tag_is_not_written() -> Result<(), BusError> {
        let mut bus = bus_with_tag(0, SimNtag::unformatted());
        let Some(mut nfc) = NfcTag::detect(&mut bus, &[0])? else {
            panic!("tag not found");
        };
        assert!(!nfc.info().formatted);
        assert!(matches!(
            nfc.write_url(&mut bus, &MockClock::new(), "https://x.io"),
            Err(NfcError::NotFormatted)
        ));
        Ok(())
    }

    #[test]
    fn finds_a_tag_at_the_factory_address_too() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        bus.attach(0, 0x55, Box::new(SimNtag::new()));
        let found = NfcTag::detect(&mut bus, &[0, 1])?.map(|t| t.info().address);
        assert_eq!(found, Some(0x55));
        Ok(())
    }

    #[test]
    fn no_tag_no_detection() -> Result<(), BusError> {
        let mut bus = MockBus::new();
        assert!(NfcTag::detect(&mut bus, &[0, 1])?.is_none());
        Ok(())
    }
}
