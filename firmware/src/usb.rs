use crate::scsi_class::MscClass;
use stm32f4xx_hal::otg_fs::{UsbBus, USB};
use usb_device::class::UsbClass;
use usb_device::prelude::*;

pub struct UsbHandler<'a> {
    pub vendor_class: MscClass<'a, UsbBus<USB>>,
    usb_dev: UsbDevice<'a, UsbBus<USB>>,
}

impl UsbHandler<'_> {
    #[must_use]
    pub fn new<'a>(
        usb_vendor: MscClass<'a, UsbBus<USB>>,
        usb_dev: UsbDevice<'a, UsbBus<USB>>,
    ) -> UsbHandler<'a> {
        UsbHandler {
            vendor_class: usb_vendor,
            usb_dev,
        }
    }

    pub fn handle(&mut self) {
        let vendor_class: &mut MscClass<UsbBus<USB>> = &mut self.vendor_class;

        self.usb_dev.poll(&mut [vendor_class]);
        vendor_class.poll();
        vendor_class.handle();
    }
}
