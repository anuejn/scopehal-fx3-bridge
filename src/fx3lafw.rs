use std::time::Duration;

use futures_lite::future::block_on;
use nusb::{
    transfer::{Control, ControlType, Recipient, RequestBuffer}, Device
};


use crate::fx3_programmer::{self, program_fx3_ram};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid response")]
    InvalidResponse,

    #[error("no fx3 device found")]
    NoFx3Device,

    #[error("fx3 programmer error: {0}")]
    Fx3ProgrammerError(fx3_programmer::Error),

    #[error("io error: {0}")]
    IoError(std::io::Error),

    #[error("usb control transfer error: {0}")]
    UsbTransferError(nusb::transfer::TransferError)
}

#[derive(Debug, Clone, Copy)]
pub enum Command {
    GetFwVersion = 0xb0,
    Start = 0xb1,
    _GetRevIdVersion = 0xb2,
}

// Structures
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VersionInfo {
    pub major: u8,
    pub minor: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CmdStartAcquisition {
    pub flags: u8,
    pub sample_delay_h: u8,
    pub sample_delay_l: u8,
}

fn encode_start_flags(sample_rate_mhz: u32, sample_size: u32) -> u8 {
    let bit_superwide = 3;
    let _bit_clk_ctl2 = 4;
    let bit_wide = 5;
    let bit_clk_src = 6;

    let mut flags = 0;

    match sample_rate_mhz {
        30 => flags |= 0 << bit_clk_src,
        48 => flags |= 1 << bit_clk_src,
        192 => flags |= 2 << bit_clk_src,
        _ => panic!("Invalid sample rate"),
    }

    match sample_size {
        8 => flags |= 0 << bit_wide,
        16 => flags |= 1 << bit_wide,
        24 => flags |= (0 << bit_wide) | (1 << bit_superwide),
        32 => flags |= (1 << bit_wide) | (1 << bit_superwide),
        _ => panic!("Invalid sample size"),
    }

    flags
}

pub fn get_version(device: &Device) -> Result<VersionInfo, Error> {
    let mut buffer = [0u8; std::mem::size_of::<VersionInfo>()];
    let bytes_written = device
        .control_in_blocking(
            Control {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: Command::GetFwVersion as u8,
                value: 0,
                index: 0,
            },
            &mut buffer,
            Duration::from_secs(1),
        )
        .map_err(Error::UsbTransferError)?;

    if bytes_written != buffer.len() {
        return Err(Error::InvalidResponse);
    }

    let version = unsafe { std::ptr::read(buffer.as_ptr() as *const VersionInfo) };
    Ok(version)
}

fn find_device() -> Option<nusb::DeviceInfo> {
    nusb::list_devices().ok()?
        .find(|dev| dev.vendor_id() == 0x04b4 && dev.product_id() == 0x00f3)
}

pub fn setup_device() -> Result<nusb::Device, Error> {
    if let Some(descriptor) = find_device() {
        eprintln!(
            "Found FX3 device {:04X}:{:04X}",
            descriptor.vendor_id(),
            descriptor.product_id()
        );
        if descriptor.product_string() != Some("fx3lafw") {
            let device = &descriptor.open().map_err(Error::IoError)?;
            eprintln!("Programming FX3 device...");
            program_fx3_ram(
                device,
                std::path::Path::new("fw/fx3lafw.img"),
            )
            .map_err(Error::Fx3ProgrammerError)?;
            device.reset().map_err(Error::IoError)?;
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }

        let descriptor = find_device().ok_or(Error::NoFx3Device)?;
        let device = descriptor.open().map_err(Error::IoError)?;
        let version: VersionInfo = get_version(&device)?;
        eprintln!("FX3LAFW version: {}.{}", version.major, version.minor);
        Ok(device)
    } else {
        Err(Error::NoFx3Device)
    }
}

pub fn start_acquisition(
    device: &Device,
    sample_rate_mhz: u32,
    sample_size: u32,
) -> Result<(), Error> {
    let flags = encode_start_flags(sample_rate_mhz, sample_size);
    let cmd = CmdStartAcquisition {
        flags,
        sample_delay_h: 0,
        sample_delay_l: 0,
    };

    let bytes_written = device
        .control_out_blocking(
            Control {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: Command::Start as u8,
                value: 0,
                index: 0,
            },
            unsafe {
                std::slice::from_raw_parts(
                    &cmd as *const _ as *const u8,
                    std::mem::size_of::<CmdStartAcquisition>(),
                )
            },
            Duration::from_secs(1),
        )
        .map_err(Error::UsbTransferError)?;

    if bytes_written != std::mem::size_of::<CmdStartAcquisition>() {
        return Err(Error::InvalidResponse);
    }

    Ok(())
}

pub fn acquisition(
    device: &Device,
    sample_rate_mhz: u32,
    sample_size: u32,
) -> Result<(), Error> {
    device.set_configuration(1).map_err(Error::IoError)?;
    let interface = device.claim_interface(0).map_err(Error::IoError)?;
    let mut queue = interface.bulk_in_queue(0x82);

    let n_transfers = 16;
    let transfer_size = 1024 * 1024;

    while queue.pending() < n_transfers {
        let request_buffer: RequestBuffer = RequestBuffer::new(transfer_size);
        let timer = std::time::Instant::now();
        queue.submit(request_buffer);
        log::debug!("submit in {:?}", timer.elapsed().as_micros());
    }

    eprintln!("sending start aquisition request...");
    start_acquisition(device, sample_rate_mhz, sample_size)?;

    let mut received = 0;
    let start_time = std::time::Instant::now();

    loop {
        let timer = std::time::Instant::now();
        let completion = block_on(queue.next_complete());
        log::debug!("got completion in {:?}", timer.elapsed().as_micros());
        if completion.status.is_err() {
            eprintln!("Error: {:?}", completion.status);
            break;
        }
        
        let request_buffer: RequestBuffer = RequestBuffer::new(transfer_size);
        let timer = std::time::Instant::now();
        queue.submit(request_buffer);
        log::debug!("submit in {:?}", timer.elapsed().as_micros());

        received += 1;
        if received % 100 == 0 {
            eprintln!("{:.02}Mb in {:.02}s ({:.02}) Mbps", received as f32 * transfer_size as f32 / 1024.0 / 1024.0, 
                start_time.elapsed().as_secs_f32(), received as f32 * transfer_size as f32 / 1024.0 / 1024.0 / start_time.elapsed().as_secs_f32());
        }
        
    }

    Ok(())
}
