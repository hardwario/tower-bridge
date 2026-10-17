//! Find out which sensor chips answer on a switch channel, by their ID
//! registers where they have one and by a harmless soft-reset command for the
//! SHT humidity sensors (an idle SHT3x does not even acknowledge a plain read).

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

/// Sensor chips used on the Climate Module and the TOWER tags, on the
/// currently selected channel.
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
    Ok(found)
}
