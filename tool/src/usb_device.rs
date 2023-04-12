use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use rusb::{Device, DeviceDescriptor, DeviceHandle, Direction, TransferType, UsbContext};
use util::{USB_PID, USB_VID};

fn open_usb_device<T: UsbContext>(
    context: &mut T,
    vid: u16,
    pid: u16,
) -> anyhow::Result<(Device<T>, DeviceDescriptor, DeviceHandle<T>)> {
    let devices = context.devices()?;

    for device in devices.iter() {
        let device_desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => {
                continue;
            }
        };

        if device_desc.vendor_id() == vid && device_desc.product_id() == pid {
            match device.open() {
                Ok(handle) => {
                    return Ok((device, device_desc, handle));
                }
                Err(e) => bail!("Device found but failed to open: {}", e),
            }
        }
    }

    Err(anyhow!("Unable to find USB Floppy Tracer"))
}

pub fn clear_buffers(handles: &(DeviceHandle<rusb::Context>, u8, u8)) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_millis(10);
    let mut in_buf = [0u8; 64];

    loop {
        let Ok(size) = handle.read_bulk(*endpoint_in, &mut in_buf, timeout) else {
            return;
        };
        println!("Cleared residual USB buffer of size {size}");
    }
}

pub fn init_usb() -> anyhow::Result<(DeviceHandle<rusb::Context>, u8, u8)> {
    let mut context = rusb::Context::new()?;

    let (device, _device_desc, mut handle) = open_usb_device(&mut context, USB_VID, USB_PID)?;

    // This seems to be optional for Linux but is required for Windows
    handle.claim_interface(0)?;

    let config_desc = device.config_descriptor(0)?;

    let mut endpoint_in_option: Option<u8> = None;
    let mut endpoint_out_option: Option<u8> = None;

    for interface in config_desc.interfaces() {
        for interface_desc in interface.descriptors() {
            for endpoint_desc in interface_desc.endpoint_descriptors() {
                if endpoint_desc.direction() == Direction::Out
                    && endpoint_desc.transfer_type() == TransferType::Bulk
                {
                    endpoint_out_option = Some(endpoint_desc.address());
                }

                if endpoint_desc.direction() == Direction::In
                    && endpoint_desc.transfer_type() == TransferType::Bulk
                {
                    endpoint_in_option = Some(endpoint_desc.address());
                }
            }
        }
    }

    let endpoint_in = endpoint_in_option.context("Endpoint In missing")?;
    let endpoint_out: u8 = endpoint_out_option.context("Endpoint Out missing")?;

    Ok((handle, endpoint_in, endpoint_out))
}
