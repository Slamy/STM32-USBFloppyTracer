use usb_device::class_prelude::*;
use usb_device::Result;

/// This should be used as `device_class` when building the `UsbDevice`.
const USB_CLASS_VENDOR: u8 = 0xff;
const SUBCLASS_NONE: u8 = 0x00;
const PROTOCOL_NONE: u8 = 0x00;
const WCID_VENDOR_CODE: u8 = 65; // ASCII 'A'
const COMPATIBILITY_ID_DESCRIPTOR_INDEX: u16 = 4;
const WCID_OS_STRING_DESC_INDEX: u8 = 0xEE;

/// taken from usbd_serial::CdcAcmClass and stripped down to the minimum but still compatible

///
/// This class can be used directly and it has the least overhead due to directly reading and
/// writing USB packets with no intermediate buffers, but it will not act like a stream-like serial
/// port. The following constraints must be followed if you use this class directly:
///
/// - `read_packet` must be called with a buffer large enough to hold max_packet_size bytes, and the
///   method will return a `WouldBlock` error if there is no packet to be read.
/// - `write_packet` must not be called with a buffer larger than max_packet_size bytes, and the
///   method will return a `WouldBlock` error if the previous packet has not been sent yet.
/// - If you write a packet that is exactly max_packet_size bytes long, it won't be processed by the
///   host operating system until a subsequent shorter packet is sent. A zero-length packet (ZLP)
///   can be sent if there is no other data to send. This is because USB bulk transactions must be
///   terminated with a short packet, even if the bulk endpoint is used for stream-like data.
pub struct MinimalVendorClass<'a, B: UsbBus> {
    data_if: InterfaceNumber,
    read_ep: EndpointOut<'a, B>,
    write_ep: EndpointIn<'a, B>,
}

impl<B: UsbBus> MinimalVendorClass<'_, B> {
    /// Creates a new VendorClass with the provided UsbBus and max_packet_size in bytes. For
    /// full-speed devices, max_packet_size has to be one of 8, 16, 32 or 64.
    pub fn new(alloc: &UsbBusAllocator<B>, max_packet_size: u16) -> MinimalVendorClass<'_, B> {
        MinimalVendorClass {
            data_if: alloc.interface(),
            read_ep: alloc.bulk(max_packet_size),
            write_ep: alloc.bulk(max_packet_size),
        }
    }

    /// Gets the maximum packet size in bytes.
    pub fn max_packet_size(&self) -> u16 {
        // The size is the same for both endpoints.
        self.read_ep.max_packet_size()
    }

    /// Writes a single packet into the IN endpoint.
    pub fn write_packet(&mut self, data: &[u8]) -> Result<usize> {
        self.write_ep.write(data)
    }

    /// Reads a single packet from the OUT endpoint.
    pub fn read_packet(&mut self, data: &mut [u8]) -> Result<usize> {
        self.read_ep.read(data)
    }
}

impl<B: UsbBus> UsbClass<B> for MinimalVendorClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(self.data_if, USB_CLASS_VENDOR, SUBCLASS_NONE, PROTOCOL_NONE)?;
        writer.endpoint(&self.write_ep)?;
        writer.endpoint(&self.read_ep)?;
        Ok(())
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();

        if req.request_type == control::RequestType::Vendor
            && req.recipient == control::Recipient::Device
            && req.index == COMPATIBILITY_ID_DESCRIPTOR_INDEX
            && req.request == WCID_VENDOR_CODE
        {
            // According to https://github.com/pbatard/libwdi/wiki/WCID-Devices
            // Provide "Microsoft Compatible ID Feature Descriptor"
            xfer.accept_with_static(&[
                0x28, 0x00, 0x00, 0x00, //  Descriptor length (40 bytes)
                0x00, 0x01, //  Version ('1.0')
                0x04, 0x00, // Compatibility ID Descriptor index (0x0004)
                0x01, // Number of sections (1)
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Reserved
                0x00, // Interface Number (Interface #0)
                0x01, // Reserved
                0x57, 0x49, 0x4E, 0x55, 0x53, 0x42, 0x00,
                0x00, // Compatible ID ("WINUSB\0\0")
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Sub-Compatible ID (unused)
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Reserved
            ])
            .unwrap();
            return;
        }

        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.data_if) as u16)
        {
            return;
        }

        match req.request {
            _ => {
                xfer.reject().ok();
            }
        }
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = xfer.request();

        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.data_if) as u16)
        {
            return;
        }

        match req.request {
            _ => {
                xfer.reject().ok();
            }
        };
    }

    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&str> {
        // According to https://github.com/pbatard/libwdi/wiki/WCID-Devices
        // Provide "Microsoft OS String Descriptor"
        if u8::from(index) == WCID_OS_STRING_DESC_INDEX {
            return Some("MSFT100A"); // Vendor Code is 65
        }

        let _ = (index, lang_id);
        None
    }
}
