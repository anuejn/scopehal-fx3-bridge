/* this file is adapted from https://github.com/nicholasbishop/cyusb-rs/blob/043496d/src/bin/cyusb_programmer.rs
 * and thus licensed under Apache-2.0. 
 * It is then adapted from rusb to nusb.
 */

use log::info;
use nusb::{transfer::{Control, ControlType, Recipient}, Device};
use std::{
    array::TryFromSliceError, convert::TryInto, fs, io, path::Path, thread,
    time::Duration,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    IoError(io::Error),

    /// "CY" prefix is missing
    #[error("invalid prefix")]
    MissingMagic,

    #[error("image is not executable")]
    NotExecutable,

    #[error("abnormal image")]
    AbnormalFirmware,

    #[error("invalid checksum")]
    InvalidChecksum,

    #[error("truncated data: {0}")]
    TruncatedData(TryFromSliceError),

    #[error("usb control transfer error: {0}")]
    UsbTransferError(nusb::transfer::TransferError),
}

struct Checksum {
    value: u32,
}

impl Checksum {
    fn new() -> Checksum {
        Checksum { value: 0 }
    }

    fn update(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut offset = 0;
        while offset < data.len() {
            let chunk = &data[offset..offset + 4];
            let val = u32::from_le_bytes(
                chunk.try_into().map_err(Error::TruncatedData)?,
            );
            self.value = self.value.overflowing_add(val).0;
            offset += 4;
        }
        Ok(())
    }
}

fn write_control(
    device: &Device,
    address: u32,
    data: &[u8],
) -> Result<usize, Error> {
    let bytes_written = device
        .control_out_blocking(
            Control {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: 0xa0,
                value: (address & 0x0000ffff) as u16,
                index: (address >> 16) as u16,
            },
            data,
            Duration::from_secs(1),
        )
        .map_err(Error::UsbTransferError)?;
    Ok(bytes_written)
}

fn control_transfer(
    device: &Device,
    mut address: u32,
    data: &[u8],
) -> Result<(), Error> {
    let mut balance = data.len() as u32;
    let mut offset = 0;

    while balance > 0 {
        let mut b = if balance > 4096 { 4096 } else { balance };

        let bytes_written = write_control(
            device,
            address,
            &data[offset as usize..(offset + b) as usize],
        )?;

        b = bytes_written as u32;

        address += b;
        balance -= b;
        offset += b;
    }

    Ok(())
}

/// Download firmware to RAM on a Cypress FX3
pub fn program_fx3_ram(
    device: &Device,
    path: &Path,
) -> Result<(), Error> {
    // Firmware files should be quite small, so just load the whole
    // thing in memory
    let program = fs::read(path).map_err(Error::IoError)?;

    // Program must start with "CY"
    if program[0] != b'C' || program[1] != b'Y' {
        return Err(Error::MissingMagic);
    }

    // Check that the image contains executable code
    if (program[2] & 0x01) != 0 {
        return Err(Error::NotExecutable);
    }

    // Check for a normal FW binary with checksum
    if program[3] != 0xb0 {
        return Err(Error::AbnormalFirmware);
    }

    let mut offset = 4;
    let mut checksum = Checksum::new();
    let entry_address;

    let read_u32 = |offset: &mut usize| {
        let chunk = &program[*offset..*offset + 4];
        let val =
            u32::from_le_bytes(chunk.try_into().map_err(Error::TruncatedData)?);
        *offset += 4;
        Ok(val)
    };

    // Transfer the program to the FX3
    info!("transfering program to the device");
    loop {
        let length = read_u32(&mut offset)?;
        let address = read_u32(&mut offset)?;

        if length == 0 {
            entry_address = address;
            break;
        } else {
            let data = &program[offset..offset + (length as usize) * 4];
            offset += (length as usize) * 4;

            checksum.update(data)?;

            control_transfer(device, address, data)?;
        }
    }

    // Read checksum
    info!("validating checksum");
    let expected_checksum = read_u32(&mut offset)?;
    if expected_checksum != checksum.value {
        return Err(Error::InvalidChecksum);
    }

    thread::sleep(Duration::from_secs(1));

    write_control(device, entry_address, &[])?;

    Ok(())
}