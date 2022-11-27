use rusb::{Context, Device, DeviceDescriptor, DeviceHandle, Direction, TransferType, UsbContext};

fn open_usb_device<T: UsbContext>(
    context: &mut T,
    vid: u16,
    pid: u16,
) -> Option<(Device<T>, DeviceDescriptor, DeviceHandle<T>)> {
    let devices = context.devices().ok()?;

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
                    return Some((device, device_desc, handle));
                }
                Err(e) => panic!("Device found but failed to open: {}", e),
            }
        }
    }

    None
}

pub fn init_usb() -> Option<(DeviceHandle<Context>, u8, u8)> {
    let vid = 0x16c0;
    let pid = 0x27dd;
    let mut context = Context::new().unwrap();

    let (device, _device_desc, handle) = open_usb_device(&mut context, vid, pid)?;

    let config_desc = device.config_descriptor(0).unwrap();

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

    let endpoint_in = endpoint_in_option.unwrap();
    let endpoint_out: u8 = endpoint_out_option.unwrap();

    Some((handle, endpoint_in, endpoint_out))
}
