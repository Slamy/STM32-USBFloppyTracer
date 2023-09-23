//! Probably useless class for USB stuff but I'm too lazy to clean up

use crate::FloppyTracerVendorClass;
use stm32f4xx_hal::otg_fs::{UsbBus, USB};
use usb_device::prelude::*;

/// A required data structures for polling based USB handling
pub struct UsbHandler<'a> {
    /// Custom class for a custom protocol
    pub vendor_class: FloppyTracerVendorClass<'a, UsbBus<USB>>,
    usb_dev: UsbDevice<'a, UsbBus<USB>>,
}

impl UsbHandler<'_> {
    #[must_use]
    /// Constructs with externally created structures
    pub fn new<'a>(
        usb_class: FloppyTracerVendorClass<'a, UsbBus<USB>>,
        usb_dev: UsbDevice<'a, UsbBus<USB>>,
    ) -> UsbHandler<'a> {
        UsbHandler {
            vendor_class: usb_class,
            usb_dev,
        }
    }

    /// Polls the USB controller for requests by the USB bus master
    /// Also checks if some data frames must be sent and does so
    pub fn handle(&mut self) {
        let vendor_class: &mut FloppyTracerVendorClass<UsbBus<USB>> = &mut self.vendor_class;

        self.usb_dev.poll(&mut [vendor_class]);
        vendor_class.handle_transmit();
    }
}
