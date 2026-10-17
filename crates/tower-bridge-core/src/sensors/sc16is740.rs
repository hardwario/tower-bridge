//! NXP SC16IS740: a UART behind I2C, with 64-byte FIFOs (on the TOWER CO2
//! Module, at 0x4D, talking to the LP8 at 9600 baud, 8N2).
//!
//! Registers are addressed as `register << 3` (the low bits select a channel
//! on the bigger parts). The crystal is 13.56 MHz, so a divisor of 88 gives
//! 9630 baud, close enough to 9600.

use crate::bus::{BusError, I2cBus};

/// On the CO2 Module (A1 and A0 low).
pub const ADDR: u8 = 0x4D;
pub const FIFO: usize = 64;

const RHR_THR: u8 = 0x00;
const IER: u8 = 0x01 << 3;
const FCR: u8 = 0x02 << 3;
const LCR: u8 = 0x03 << 3;
const SPR: u8 = 0x07 << 3;
const TXLVL: u8 = 0x08 << 3;
const RXLVL: u8 = 0x09 << 3;
/// With LCR = 0x80.
const DLL: u8 = 0x00;
const DLH: u8 = 0x01 << 3;
/// With LCR = 0xBF.
const EFR: u8 = 0x02 << 3;

/// 13.56 MHz / 16 / 88 = 9630 baud.
const DIVISOR_9600: u8 = 88;

fn write_reg(bus: &mut dyn I2cBus, addr: u8, reg: u8, value: u8) -> Result<(), BusError> {
    bus.write(addr, &[reg, value])
}

fn read_reg(bus: &mut dyn I2cBus, addr: u8, reg: u8) -> Result<u8, BusError> {
    let mut b = [0u8];
    bus.write_read(addr, &[reg], &mut b)?;
    Ok(b[0])
}

/// Whether an SC16IS7xx answers at `addr`: its scratchpad keeps what is
/// written to it.
pub fn present(bus: &mut dyn I2cBus, addr: u8) -> Result<bool, BusError> {
    for pattern in [0x5A, 0xA5] {
        write_reg(bus, addr, SPR, pattern)?;
        if read_reg(bus, addr, SPR)? != pattern {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 9600 baud, 8 data bits, no parity, 2 stop bits; FIFOs on and empty; no
/// interrupts and no sleep mode (a sleeping UART can garble the first byte
/// it wakes up on).
pub fn init(bus: &mut dyn I2cBus, addr: u8) -> Result<(), BusError> {
    write_reg(bus, addr, LCR, 0x80)?;
    write_reg(bus, addr, DLL, DIVISOR_9600)?;
    write_reg(bus, addr, DLH, 0x00)?;
    write_reg(bus, addr, LCR, 0xBF)?;
    write_reg(bus, addr, EFR, 0x10)?;
    write_reg(bus, addr, LCR, 0x07)?;
    write_reg(bus, addr, FCR, 0x07)?;
    write_reg(bus, addr, IER, 0x00)
}

/// Throw away anything received so far.
pub fn clear_rx(bus: &mut dyn I2cBus, addr: u8) -> Result<(), BusError> {
    write_reg(bus, addr, FCR, 0x03)
}

pub fn rx_level(bus: &mut dyn I2cBus, addr: u8) -> Result<usize, BusError> {
    Ok(usize::from(read_reg(bus, addr, RXLVL)?))
}

/// Queue `data` for sending; it must fit the free transmit FIFO.
pub fn send(bus: &mut dyn I2cBus, addr: u8, data: &[u8]) -> Result<(), BusError> {
    if data.len() > FIFO || usize::from(read_reg(bus, addr, TXLVL)?) < data.len() {
        return Err(BusError::Protocol("UART transmit FIFO full".into()));
    }
    let mut frame = Vec::with_capacity(data.len() + 1);
    frame.push(RHR_THR);
    frame.extend_from_slice(data);
    bus.write(addr, &frame)
}

/// Take `buf.len()` received bytes (the caller checked they are there).
pub fn receive(bus: &mut dyn I2cBus, addr: u8, buf: &mut [u8]) -> Result<(), BusError> {
    bus.write_read(addr, &[RHR_THR], buf)
}
