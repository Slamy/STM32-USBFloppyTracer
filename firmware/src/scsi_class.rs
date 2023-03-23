use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
use async_trait::async_trait;
use cassette::Cassette;
use core::{convert::TryInto, future::Future, pin::Pin};

use num;
use num_derive::FromPrimitive;
use rtt_target::rprintln;

use usb_device::{
    class_prelude::*,
    control::{Request, RequestType},
    Result,
};

/// This should be used as `device_class` when building `UsbDevice`
///
/// Section 4.3 [USB Bulk Only Transport Spec](https://www.usb.org/document-library/mass-storage-bulk-only-10)
pub const USB_CLASS_MSC: u8 = 0x08;

pub const BULK_ONLY_TRANSPORT: u8 = 0x50;
pub const SCSI_TRANSPARENT_COMMAND_SET: u8 = 0x06;

const REQ_GET_MAX_LUN: u8 = 0xFE;
const REQ_BULK_ONLY_RESET: u8 = 0xFF;

const INQUIRY_DATA: [u8; 36] = [
    0x00, // magnetic disk
    0x80, // removable media
    0x06, // SPC-4
    2,    // Standard response data format
    32,   // Additional length
    0, 0, // BQue not supported
    0, // Command Queuing not supported
    // Vendor Identification
    b'S', b'l', b'a', b'm', b'y', b' ', b' ', b' ', //
    // Product Identification
    b'S', b'T', b'M', b'3', b'2', b'-', b'U', b'S', b'B', b'F', b'l', b'o', b'p', b'p', b'y', b' ',
    // Product Revision
    b'0', b'0', b'0', b'1',
];

enum DataDirection {
    DataOutHostToDevice,
    DataInDeviceToHost,
}

/// SCSI op codes as defined by SPC-3
#[derive(Clone, Copy, Eq, PartialEq, Debug, FromPrimitive)]
pub enum OpCode {
    TestUnitReady = 0x00,
    RequestSense = 0x03,
    Format = 0x04,
    Read6 = 0x08,
    Write6 = 0x0A,
    Inquiry = 0x12,
    ReadCapacity10 = 0x25,
    Read10 = 0x28,
    SendDiagnostic = 0x1D,
    ReportLuns = 0xA0,

    ModeSense6 = 0x1A,
    ModeSense10 = 0x5A,

    ModeSelect6 = 0x15,
    StartStopUnit = 0x1B,
    PreventAllowMediumRemoval = 0x1E,
    ReadFormatCapacities = 0x23,
    Write10 = 0x2A,
    Verify10 = 0x2F,
    SynchronizeCache10 = 0x35,
    ReadTocPmaAtip = 0x43,
    ModeSelect10 = 0x55,
    Read12 = 0xA8,
    Write12 = 0xAA,
}

#[async_trait(?Send)]
pub trait BlockDevice {
    fn medium_present(&self) -> bool;
    fn max_lba(&self) -> u32;
    async fn read_block(&mut self, lba: u32) -> Option<Vec<u8>>;
    async fn write_block(&mut self, lba: u32, block: &[u8]) -> Result<()>;
}

/// # USB Mass Storage Class Device
///
/// So far only tested with the Bulk Only protocol and the SCSI transparent command set - see
/// [Scsi](struct.Scsi.html) and [Bulk Only Transport](struct.BulkOnlyTransport.html)
pub struct MscClass<'a, B: UsbBus> {
    data_if: InterfaceNumber,
    read_ep: Rc<EndpointOut<'a, B>>,
    write_ep: Rc<EndpointIn<'a, B>>,
    async_process: Cassette<Pin<Box<dyn Future<Output = ()> + 'a>>>,
}

pub struct MscScsiProcess<'a, B: UsbBus> {
    read_ep: Rc<EndpointOut<'a, B>>,
    write_ep: Rc<EndpointIn<'a, B>>,
    block_device: Box<dyn BlockDevice>,
}

impl<'a, B: UsbBus> MscScsiProcess<'a, B> {
    async fn read_bulk(&self, buf: &mut [u8; 64]) -> usize {
        loop {
            if let Ok(count) = self.read_ep.read(buf) {
                // rprintln!("Read Bulk {}", count);
                return count;
            }
            cassette::yield_now().await;
        }
    }

    async fn write_bulk(&self, buf: &[u8]) {
        loop {
            //rprintln!("Write Bulk");
            if let Ok(count) = self.write_ep.write(buf) {
                //rprintln!("Write Bulk {}", count);

                assert_eq!(count, buf.len());
                return;
            }
            cassette::yield_now().await;
        }
    }

    async fn process(mut self) {
        loop {
            let mut buf = [0u8; 64];

            let count = self.read_bulk(&mut buf).await;
            // rprintln!("Read Command {:x?} {}", slice, count);

            let d_cbwsignature = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            let d_cbwtag = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            let _d_cbwdata_transfer_length = u32::from_le_bytes(buf[8..12].try_into().unwrap());
            let bm_cbwflags = buf[12];
            let d_cbwlun = buf[13] & 0x0f;
            let d_cbwcblength = (buf[14] & 0x1f) as usize;

            assert_eq!(0x43425355, d_cbwsignature);
            assert_eq!(count, 31);
            assert_eq!(d_cbwlun, 0);

            // rprintln!("dCBWSignature {:x?} {}", d_cbwsignature, d_cbwcblength);
            let cbwcb = &buf[15..(15 + d_cbwcblength)];
            // rprintln!("CBWCB {:x?}", cbwcb);

            /*
            rprintln!(
                "dCBWDataTransferLength {} dCBWTag {} bmCBWFlags {:x}",
                d_cbwdata_transfer_length,
                d_cbwtag,
                bm_cbwflags
            );
            */

            // 0 = Data-Out from host to the device,
            // 1 = Data-In from the device to the host.
            let direction = if (bm_cbwflags & 0x80) != 0 {
                DataDirection::DataInDeviceToHost
            } else {
                DataDirection::DataOutHostToDevice
            };

            let result = self.process_scsi_command(cbwcb, direction).await;
            let residue = if let Ok(count) = result {
                count
            } else {
                d_cbwcblength
            };
            let mut csw_buf: [u8; 13] = [0u8; 13];

            csw_buf[0..4].copy_from_slice(&u32::to_le_bytes(0x53425355));
            csw_buf[4..8].copy_from_slice(&u32::to_le_bytes(d_cbwtag));
            csw_buf[8..12].copy_from_slice(&u32::to_le_bytes(residue as u32));
            csw_buf[12] = if result.is_ok() { 0 } else { 1 };
            self.write_bulk(&csw_buf).await;
            //rprintln!("Sent CSW");
        }
    }

    async fn process_scsi_command(
        &mut self,
        cbwcb: &[u8],
        _direction: DataDirection,
    ) -> core::result::Result<usize, ()> {
        let opcode: OpCode = num::FromPrimitive::from_u8(cbwcb[0]).unwrap();
        //rprintln!("== {:?}", opcode);

        match opcode {
            OpCode::Inquiry => {
                self.write_bulk(&INQUIRY_DATA).await;
                core::result::Result::Ok(INQUIRY_DATA.len())
            }
            OpCode::ReadCapacity10 => {
                if self.block_device.medium_present() {
                    let mut buf: [u8; 8] = [0u8; 8];
                    buf[0..4].copy_from_slice(&u32::to_be_bytes(self.block_device.max_lba()));
                    buf[4..8].copy_from_slice(&u32::to_be_bytes(512));

                    self.write_bulk(&buf).await;
                    core::result::Result::Ok(buf.len())
                } else {
                    core::result::Result::Err(())
                }
            }
            OpCode::TestUnitReady => {
                if self.block_device.medium_present() {
                    core::result::Result::Ok(0)
                } else {
                    core::result::Result::Err(())
                }
            }
            OpCode::ModeSense6 => {
                let buf: [u8; 7] = [
                    6,    // Mode data length
                    0,    // medium type
                    0x80, // write protected
                    0,    // block descriptor length
                    0x08, // Caching Mode page
                    1,    // page length
                    0,    // Write cache disabled and read Cache enabled
                ];
                self.write_bulk(&buf).await;
                core::result::Result::Ok(buf.len())
            }
            OpCode::Read10 => {
                if self.block_device.medium_present() {
                    let logical_block_address = u32::from_be_bytes(cbwcb[2..6].try_into().unwrap());
                    let transfer_length =
                        u16::from_be_bytes(cbwcb[7..9].try_into().unwrap()) as u32;
                    // rprintln!("Read10 {} {}", logical_block_address, transfer_length);
                    // let bytes_transferred = 0;
                    // let bulk_transfers_per_block = 512 / 64;
                    // let loops = transfer_length * bulk_transfers_per_block;

                    for block_offset in 0..transfer_length {
                        let block = self
                            .block_device
                            .read_block(logical_block_address + block_offset)
                            .await
                            .unwrap();

                        assert_eq!(block.len(), 512);
                        for block_chunk in block.chunks(64) {
                            self.write_bulk(block_chunk).await;
                        }
                    }

                    core::result::Result::Ok(0)
                } else {
                    core::result::Result::Err(())
                }
            }
            OpCode::Write10 => {
                if self.block_device.medium_present() {
                    let logical_block_address = u32::from_be_bytes(cbwcb[2..6].try_into().unwrap());
                    let transfer_length =
                        u16::from_be_bytes(cbwcb[7..9].try_into().unwrap()) as u32;
                    rprintln!("Write10 {} {}", logical_block_address, transfer_length);

                    for block_offset in 0..transfer_length {
                        let mut block: Vec<u8> = vec![0; 512];

                        for block_chunk in block.chunks_mut(64) {
                            self.read_bulk(block_chunk.try_into().unwrap()).await;
                        }

                        self.block_device
                            .write_block(logical_block_address + block_offset, &block)
                            .await
                            .unwrap();
                    }

                    core::result::Result::Ok(0)
                } else {
                    core::result::Result::Err(())
                }
            }
            OpCode::RequestSense => {
                if self.block_device.medium_present() {
                    let buf: [u8; 18] = [
                        0x70, // Current Error
                        0,    // Segment Number,
                        0x5,  // Sense Key
                        0, 0, 0, 0,  //Information
                        10, // Additional Sense Length
                        0, 0, 0, 0,    // Command specific information
                        0x20, // ASC
                        0x00, // ASCQ
                        0,    // Field Replaceable Unit Code
                        0,    // SKSV False
                        0, 0, // Sense Key Specific
                    ];
                    self.write_bulk(&buf).await;
                    core::result::Result::Ok(buf.len())
                } else {
                    //Medium Not Present, Drive Not Unloaded SenseKey: 5h, ASC 3Ah, ASCQ 00h, SKSV No

                    let buf: [u8; 18] = [
                        0x70, // Current Error
                        0,    // Segment Number,
                        0x2,  // Sense Key
                        0, 0, 0, 0,  //Information
                        10, // Additional Sense Length
                        0, 0, 0, 0,    // Command specific information
                        0x3a, // ASC
                        0x00, // ASCQ
                        0,    // Field Replaceable Unit Code
                        0,    // SKSV False
                        0, 0, // Sense Key Specific
                    ];
                    self.write_bulk(&buf).await;
                    core::result::Result::Ok(buf.len())
                }
            }
            OpCode::PreventAllowMediumRemoval => core::result::Result::Err(()),
            OpCode::ReadFormatCapacities => {

                let buf: [u8; 12] = [
                    0,0,0, // Reserved
                    0x08, // Capacity List Length
                    0x00,0x00,0x0B,0x40, // Number of Blocks. TODO fixed at 2880
                    0x02, // Descriptor Type: Formatted media
                    0x00,0x02,0x00, // Single Block is 512 byte in size
                    ];
                    self.write_bulk(&buf).await;
                    core::result::Result::Ok(0)

            },
            
            _ => {
                todo!("{:?} ignored", opcode);
            }
        }
    }
}
impl<'a, B: UsbBus> MscClass<'a, B> {
    pub fn new(
        alloc: &'a UsbBusAllocator<B>,
        max_packet_size: u16,
        block_device: Box<dyn BlockDevice>,
    ) -> MscClass<'a, B> {
        let read_ep: Rc<EndpointOut<B>> = Rc::new(alloc.bulk(max_packet_size));
        let write_ep: Rc<EndpointIn<B>> = Rc::new(alloc.bulk(max_packet_size));

        let process = MscScsiProcess {
            write_ep: write_ep.clone(),
            read_ep: read_ep.clone(),
            block_device,
        };

        let async_process: Cassette<Pin<Box<dyn Future<Output = ()> + 'a>>> =
            Cassette::new(Box::pin(process.process()));

        MscClass {
            data_if: alloc.interface(),
            write_ep,
            read_ep,
            async_process,
        }
    }

    pub fn max_packet_size(&self) -> u16 {
        // The size is the same for both endpoints.
        self.read_ep.max_packet_size()
    }

    pub fn handle(&mut self) {
        let result = self.async_process.poll_on();
        assert!(result.is_none());
    }
}

impl<B: UsbBus> UsbClass<B> for MscClass<'_, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.data_if,
            USB_CLASS_MSC,
            SCSI_TRANSPARENT_COMMAND_SET,
            BULK_ONLY_TRANSPORT,
        )?;

        writer.endpoint(&self.read_ep)?;
        writer.endpoint(&self.write_ep)?;

        Ok(())
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        // rprintln!("Control In!");
        let req = xfer.request();

        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.data_if) as u16)
        {
            return;
        }

        let handled_res = match req {
            // Get max lun
            Request {
                request_type: RequestType::Class,
                request: REQ_GET_MAX_LUN,
                ..
            } => Some(xfer.accept(|data| {
                let max_lun = 0;
                rprintln!("USB_CONTROL> Get max lun. Response: {}", max_lun);
                data[0] = max_lun;
                Ok(1)
            })),

            // Bulk only mass storage reset
            Request {
                request_type: RequestType::Class,
                request: REQ_BULK_ONLY_RESET,
                ..
            } => {
                // There's some more functionality around this request to allow the reset to take
                // more time - NAK the status until the reset is done.
                // This isn't implemented.
                // See Section 3.1 [USB Bulk Only Transport Spec](https://www.usb.org/document-library/mass-storage-bulk-only-10)
                self.reset();
                Some(xfer.accept(|_| {
                    rprintln!("USB_CONTROL> Bulk only mass storage reset");
                    Ok(0)
                }))
            }
            _ => {
                xfer.reject().ok();
                None
            }
        };

        if let Some(Err(e)) = handled_res {
            rprintln!("Error from ControlIn.accept: {:?}", e);
        }
    }

    fn get_bos_descriptors(&self, writer: &mut BosWriter) -> Result<()> {
        let _ = writer;
        Ok(())
    }

    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&str> {
        let _ = (index, lang_id);
        None
    }

    fn reset(&mut self) {}

    fn poll(&mut self) {
        //rprintln!("poll");
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
    fn endpoint_setup(&mut self, addr: EndpointAddress) {
        let _ = addr;
    }

    fn endpoint_out(&mut self, addr: EndpointAddress) {
        let _ = addr;
    }

    fn endpoint_in_complete(&mut self, addr: EndpointAddress) {
        let _ = addr;
    }
}
