use usb_device::class_prelude::*;
use usb_device::Result;

/// This should be used as `device_class` when building the `UsbDevice`.
const USB_CLASS_VENDOR: u8 = 0xff;
const SUBCLASS_NONE: u8 = 0x00;
const PROTOCOL_NONE: u8 = 0x00;
const WCID_VENDOR_CODE: u8 = 65; // ASCII 'A'
const COMPATIBILITY_ID_DESCRIPTOR_INDEX: u16 = 4;
const WCID_OS_STRING_DESC_INDEX: u8 = 0xEE;

use core::convert::TryInto;

use alloc::{collections::VecDeque, vec::Vec};
use usb_device::class_prelude::UsbBus;
use util::{
    Cylinder, Density, DensityMap, DensityMapEntry, DriveSelectState, Head, PulseDuration,
    RawCellData, Track,
};

use crate::{interrupts, rprintln, INDEX_SIM};

pub enum Command {
    WriteVerifyRawTrack {
        track: Track,
        raw_cell_data: RawCellData,
        write_precompensation: PulseDuration,
    },
    ReadTrack {
        track: Track,
        duration_to_record: u32,
        wait_for_index: bool,
    },
}

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
pub struct FloppyTracerVendorClass<'a, B: UsbBus> {
    data_if: InterfaceNumber,
    read_ep: EndpointOut<'a, B>,
    write_ep: EndpointIn<'a, B>,
    // private stuff
    receive_buffer: Vec<u8>,
    speeds: DensityMap,
    remaining_blocks: u32,
    expected_size: usize,
    cylinder: u32,
    head: u32,
    has_non_flux_reversal_area: bool,
    write_precompensation: PulseDuration,
    tx_buffer: VecDeque<Vec<u8>>,
    current_command: Option<Command>,
}

impl<B: UsbBus> FloppyTracerVendorClass<'_, B> {
    /// Creates a new VendorClass with the provided UsbBus and max_packet_size in bytes. For
    /// full-speed devices, max_packet_size has to be one of 8, 16, 32 or 64.
    pub fn new(alloc: &UsbBusAllocator<B>, max_packet_size: u16) -> FloppyTracerVendorClass<'_, B> {
        FloppyTracerVendorClass {
            data_if: alloc.interface(),
            read_ep: alloc.bulk(max_packet_size),
            write_ep: alloc.bulk(max_packet_size),
            receive_buffer: Vec::with_capacity(64),
            speeds: Vec::with_capacity(16),
            remaining_blocks: 0,
            expected_size: 0,
            cylinder: 0,
            head: 0,
            has_non_flux_reversal_area: false,
            write_precompensation: PulseDuration(0),
            tx_buffer: VecDeque::new(),
            current_command: None,
        }
    }

    pub fn take_command(&mut self) -> Option<Command> {
        self.current_command.take()
    }
    /// Gets the maximum packet size in bytes.
    pub fn max_packet_size(&self) -> u16 {
        // The size is the same for both endpoints.
        self.read_ep.max_packet_size()
    }

    /// Writes a single packet into the IN endpoint.
    pub fn write_packet(&self, data: &[u8]) -> Result<usize> {
        self.write_ep.write(data)
    }

    /// Reads a single packet from the OUT endpoint.
    pub fn read_packet(&self, data: &mut [u8]) -> Result<usize> {
        self.read_ep.read(data)
    }

    pub fn response(&mut self, text: &str) {
        assert!(text.len() < 60);

        let buf = text.as_bytes().into();
        self.tx_buffer.push_back(buf);
    }

    pub fn write(&mut self, data: &[u8]) {
        assert!(data.len() <= 64);
        self.tx_buffer.push_back(data.into());
    }

    pub fn write_consume(&mut self, data: Vec<u8>) {
        assert!(data.len() <= 64);
        self.tx_buffer.push_back(data);
    }

    pub fn handle_transmit(&mut self) {
        // Some data to send?
        if let Some(front) = self.tx_buffer.front() {
            if self.write_packet(front).is_ok() {
                self.tx_buffer.pop_front();
            }
        }
    }

    fn handle_command(&mut self, buf: &[u8]) -> Option<()> {
        let mut header = buf.chunks(4);

        let command = u32::from_le_bytes(header.next()?.try_into().ok()?);
        match command {
            // Write track
            0x1234_0001 => {
                self.expected_size = u32::from_le_bytes(header.next()?.try_into().ok()?) as usize;
                self.remaining_blocks = u32::from_le_bytes(header.next()?.try_into().ok()?);

                // Fields 00000000 PPPPPPPP 000000NH CCCCCCCC
                let packed_configuration = u32::from_le_bytes(header.next()?.try_into().ok()?);

                self.cylinder = packed_configuration & 0xff;
                self.head = (packed_configuration >> 8) & 1;
                self.has_non_flux_reversal_area = (packed_configuration & 0x200) != 0;
                self.write_precompensation =
                    PulseDuration(((packed_configuration >> 16) & 0xff) as i32);

                let speed_table_size = u32::from_le_bytes(header.next()?.try_into().ok()?);

                for _ in 0..speed_table_size {
                    let table_entry = u32::from_le_bytes(header.next()?.try_into().ok()?);

                    self.speeds.push(DensityMapEntry {
                        number_of_cellbytes: (table_entry >> 9) as usize,
                        cell_size: (PulseDuration((table_entry & 0x1ff) as i32)),
                    });
                }
                self.receive_buffer.reserve(self.expected_size);
            }
            // Configure drive
            0x1234_0002 => {
                let settings = u32::from_le_bytes(header.next()?.try_into().ok()?);
                let index_sim_frequency = u32::from_le_bytes(header.next()?.try_into().ok()?);

                let selected_drive = if settings & 1 == 0 {
                    DriveSelectState::A
                } else {
                    DriveSelectState::B
                };

                let floppy_density = if settings & 2 == 0 {
                    Density::SingleDouble
                } else {
                    Density::High
                };
                cortex_m::interrupt::free(|cs| {
                    INDEX_SIM
                        .borrow(cs)
                        .borrow_mut()
                        .as_ref()
                        .expect("Program flow error")
                        .configure(index_sim_frequency);

                    let mut floppy_control_borrow =
                        interrupts::FLOPPY_CONTROL.borrow(cs).borrow_mut();
                    let floppy_control =
                        floppy_control_borrow.as_mut().expect("Program flow error");

                    floppy_control.select_drive(selected_drive);
                    floppy_control.select_density(floppy_density);
                });
            }
            // step to track
            0x1234_0003 => {
                let cylinder = u32::from_le_bytes(header.next()?.try_into().ok()?);
                cortex_m::interrupt::free(|cs| {
                    let mut floppy_control_borrow =
                        interrupts::FLOPPY_CONTROL.borrow(cs).borrow_mut();
                    let floppy_control =
                        floppy_control_borrow.as_mut().expect("Program flow error");

                    rprintln!("Step to track {}", cylinder);
                    floppy_control.select_track(Track {
                        cylinder: Cylinder(cylinder as u8),
                        head: Head(0),
                    });
                });
            }
            // read track
            0x1234_0004 => {
                let packed_configuration = u32::from_le_bytes(header.next()?.try_into().ok()?);
                let duration_to_record = u32::from_le_bytes(header.next()?.try_into().ok()?);
                let cylinder = packed_configuration & 0xff;
                let head = (packed_configuration >> 8) & 1;
                let wait_for_index = ((packed_configuration >> 9) & 1) != 0;
                let new_command = Command::ReadTrack {
                    track: Track {
                        cylinder: Cylinder(cylinder as u8),
                        head: Head(head as u8),
                    },
                    duration_to_record,
                    wait_for_index,
                };

                let old_command = self.current_command.replace(new_command);

                // Last command shall be not existing.
                // If it exists, it was dropped now, which is not good
                assert!(old_command.is_none());
            }
            _ => {
                rprintln!("Unknown command");
            }
        }
        Some(())
    }
}

impl<B: UsbBus> UsbClass<B> for FloppyTracerVendorClass<'_, B> {
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
            .expect("Unexpected USB problem");
            return;
        }

        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.data_if) as u16)
        {
            return;
        }

        xfer.reject().ok();
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = xfer.request();

        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.data_if) as u16)
        {
            return;
        }

        xfer.reject().ok();
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

    fn get_bos_descriptors(&self, writer: &mut BosWriter) -> Result<()> {
        let _ = writer;
        Ok(())
    }

    fn poll(&mut self) {
        let mut buf = [0u8; 64];

        if let Ok(count) = self.read_packet(&mut buf) {
            if self.remaining_blocks == 0 {
                self.handle_command(&buf);
            } else {
                let buf = buf.get(0..count).expect("Cannot fail.");
                self.receive_buffer.extend(buf.iter());

                self.remaining_blocks -= 1;

                if self.remaining_blocks == 0 {
                    // We have received everything we need.
                    assert!(self.expected_size == self.receive_buffer.len());

                    // Create the next receive buffer and take the current one
                    let mut recv_buffer = Vec::with_capacity(64);
                    let mut speeds: DensityMap = Vec::with_capacity(64);

                    core::mem::swap(&mut recv_buffer, &mut self.receive_buffer);
                    core::mem::swap(&mut speeds, &mut self.speeds);

                    let new_command = Command::WriteVerifyRawTrack {
                        track: Track {
                            cylinder: Cylinder(self.cylinder as u8),
                            head: Head(self.head as u8),
                        },
                        raw_cell_data: RawCellData::construct(
                            speeds,
                            recv_buffer,
                            self.has_non_flux_reversal_area,
                        ),
                        write_precompensation: self.write_precompensation,
                    };

                    let old_command = self.current_command.replace(new_command);

                    // Last command shall be not existing.
                    // If it exists, it was dropped now, which is not good
                    assert!(old_command.is_none());
                }
            }
        }
    }
}
