//! Find out which chips answer on a switch channel, by their ID registers
//! where they have one and by a harmless soft-reset command for the SHT
//! humidity sensors (an idle SHT3x does not even acknowledge a plain read).
//! Nothing here changes what a chip does.

use crate::bus::{BusError, I2cBus};

/// A chip recognised at an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub address: u8,
    pub part: &'static str,
}

fn id16(bus: &mut dyn I2cBus, addr: u8, reg: u8) -> Result<Option<u16>, BusError> {
    let mut b = [0u8; 2];
    match bus.write_read(addr, &[reg], &mut b) {
        Ok(()) => Ok(Some(u16::from_be_bytes(b))),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => Ok(None),
    }
}

fn id8(bus: &mut dyn I2cBus, addr: u8, reg: u8) -> Result<Option<u8>, BusError> {
    let mut b = [0u8; 1];
    match bus.write_read(addr, &[reg], &mut b) {
        Ok(()) => Ok(Some(b[0])),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => Ok(None),
    }
}

fn acks(bus: &mut dyn I2cBus, addr: u8, command: &[u8]) -> Result<bool, BusError> {
    match bus.write(addr, command) {
        Ok(()) => Ok(true),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => Ok(false),
    }
}

/// Chips of the TOWER modules and tags on the currently selected channel,
/// by address.
pub fn scan(bus: &mut dyn I2cBus) -> Result<Vec<Found>, BusError> {
    let mut found = Vec::new();
    let mut add = |address, part| found.push(Found { address, part });
    for address in [0x40, 0x41] {
        // HDC2080: manufacturer ID 0x5449 ("TI"), little endian at 0xFC.
        if id16(bus, address, 0xFC)? == Some(0x4954) {
            add(address, "HDC2080");
        } else if address == 0x40 && acks(bus, address, &[0xFE])? {
            add(address, "SHT20");
        }
    }
    for address in [0x44, 0x45] {
        // OPT3001: device ID 0x3001 at 0x7F; otherwise an SHT3x soft reset.
        if id16(bus, address, 0x7F)? == Some(0x3001) {
            add(address, "OPT3001");
        } else if acks(bus, address, &[0x30, 0xA2])? {
            add(address, "SHT30");
        }
    }
    for address in 0x48..=0x4B {
        if id16(bus, address, 0x01)?.is_some() {
            add(address, "TMP112");
        }
    }
    if id8(bus, 0x5F, 0x0F)? == Some(0xBC) {
        add(0x5F, "HTS221");
    }
    if id8(bus, 0x60, 0x0C)? == Some(0xC4) {
        add(0x60, "MPL3115A2");
    }
    for address in crate::nfc::ADDRESSES {
        if let Some(part) = crate::nfc::identify(bus, address)? {
            add(address, part);
        }
    }
    // The expanders of the Relay, CO2 and GPS Modules; reading the
    // direction register leaves the outputs alone.
    for address in (0x20..=0x27).chain(0x38..=0x3F) {
        if id8(bus, address, 0x03)?.is_some() {
            add(
                address,
                if address >= 0x38 {
                    "TCA9534A"
                } else {
                    "TCA9534"
                },
            );
        }
    }
    // The GPS Module's receiver: the count of bytes it has waiting. It
    // answers only while powered (expander pin P0), which is left alone here.
    if id16(bus, crate::gps::RECEIVER, 0xFD)?.is_some() {
        add(crate::gps::RECEIVER, "SAM-M8Q");
    }
    // The CO2 Module's UART bridge keeps a byte in its scratch register.
    match crate::sensors::sc16is740::present(bus, crate::co2::UART) {
        Ok(true) => add(crate::co2::UART, "SC16IS740"),
        Ok(false) => {}
        Err(e) if e.is_fatal() => return Err(e),
        Err(_) => {}
    }
    if crate::sensors::sgpc3::probe(bus)?.is_some() {
        add(crate::sensors::sgpc3::ADDR, "SGPC3");
    }
    found.sort_by_key(|f| f.address);
    Ok(found)
}
