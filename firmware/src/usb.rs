use core::{cell::RefCell, convert::TryInto};

use alloc::vec::Vec;
use cortex_m::interrupt::{CriticalSection, Mutex};
use stm32f4xx_hal::otg_fs::{UsbBus, USB};
use usb_device::prelude::*;
use usbd_serial::SerialPort;
use util::{
    Cylinder, Density, DensityMapEntry, DriveSelectState, Head, PulseDuration, RawCellData, Track,
};

use crate::{interrupts, orange, safeiprintln};

pub static CURRENT_COMMAND: Mutex<RefCell<Option<Command>>> = Mutex::new(RefCell::new(None));

pub enum Command {
    WriteVerifyRawTrack(Track, RawCellData, usize),
}

pub struct UsbHandler<'a> {
    usb_serial: SerialPort<'a, UsbBus<USB>>,
    usb_dev: UsbDevice<'a, UsbBus<USB>>,
    receive_buffer: Vec<u8>,
    speeds: Vec<DensityMapEntry>,
    offset: usize,
    remaining_blocks: u32,
    expected_size: usize,
    cylinder: u32,
    head: u32,
    first_significane_offset: u32,
}

impl UsbHandler<'_> {
    pub fn new<'a>(
        usb_serial: SerialPort<'a, UsbBus<USB>>,
        usb_dev: UsbDevice<'a, UsbBus<USB>>,
    ) -> UsbHandler<'a> {
        UsbHandler {
            usb_serial,
            usb_dev,
            receive_buffer: Vec::with_capacity(64),
            speeds: Vec::with_capacity(16),
            offset: 0,
            remaining_blocks: 0,
            expected_size: 0,
            cylinder: 0,
            head: 0,
            first_significane_offset: 0,
        }
    }

    pub fn response(&mut self, text: &str) {
        assert!(text.len() < 60);

        let serial: &mut SerialPort<UsbBus<USB>> = &mut self.usb_serial;
        match serial.write(text.as_bytes()) {
            Ok(len) if len > 0 => { /* Good */ }
            _ => {
                safeiprintln!("No!");
            }
        }
    }

    pub fn handle_interrupt(&mut self, cs: &CriticalSection) {
        let serial: &mut SerialPort<UsbBus<USB>> = &mut self.usb_serial;

        orange(true);
        if self.usb_dev.poll(&mut [serial]) {
            let mut buf = [0u8; 64];

            //let buf = &mut self.receive_buffer[self.offset..self.offset + 64];
            if let Ok(count) = serial.read(&mut buf) {
                if self.remaining_blocks == 0 {
                    let mut header = buf.chunks(4);

                    //safeiprintln!("{:?}", buf);

                    let command = u32::from_le_bytes(header.next().unwrap().try_into().unwrap());
                    match command {
                        0x12340001 => {
                            self.expected_size =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap())
                                    as usize;
                            self.remaining_blocks =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                            let cylinder_and_head =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                            self.cylinder = cylinder_and_head & 0xffff;
                            self.head = cylinder_and_head >> 16;

                            self.first_significane_offset =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                            let speed_table_size =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                            for _ in 0..speed_table_size {
                                let table_entry =
                                    u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                                self.speeds.push(DensityMapEntry {
                                    number_of_cells: (table_entry >> 9) as usize,
                                    cell_size: (PulseDuration((table_entry & 0x1ff) as u16)),
                                });
                            }
                            self.receive_buffer.reserve(self.expected_size);
                        }
                        0x12340002 => {
                            let settings =
                                u32::from_le_bytes(header.next().unwrap().try_into().unwrap());

                            let selected_drive = if settings & 1 != 0 {
                                DriveSelectState::B
                            } else {
                                DriveSelectState::A
                            };

                            let floppy_density = if settings & 2 != 0 {
                                Density::High
                            } else {
                                Density::SingleDouble
                            };
                            cortex_m::interrupt::free(|cs| {
                                let mut floppy_control_borrow =
                                    interrupts::FLOPPY_CONTROL.borrow(cs).borrow_mut();
                                let floppy_control = floppy_control_borrow.as_mut().unwrap();

                                floppy_control.select_drive(selected_drive);
                                floppy_control.select_density(floppy_density);
                            });
                        }
                        _ => {
                            panic!("Unknown command");
                        }
                    }

                    /*
                    safeiprintln!(
                        "Got command {} {} {} {}",
                        self.expected_size,
                        self.remaining_blocks,
                        cylinder,
                        head
                    );
                    */
                } else {
                    self.receive_buffer.extend(buf[0..count].iter());

                    self.remaining_blocks -= 1;

                    if self.remaining_blocks == 0 {
                        // We have receiving everything we need.
                        /*
                        safeiprintln!(
                            "Got command {} {}",
                            self.expected_size,
                            self.receive_buffer.len()
                        );
                        */

                        assert!(self.expected_size == self.receive_buffer.len());

                        // Create the next receive buffer and take the current one
                        let mut recv_buffer = Vec::with_capacity(64);
                        let mut speeds: Vec<DensityMapEntry> = Vec::with_capacity(64);

                        core::mem::swap(&mut recv_buffer, &mut self.receive_buffer);
                        core::mem::swap(&mut speeds, &mut self.speeds);

                        let x = CURRENT_COMMAND.borrow(cs).borrow_mut().replace(
                            Command::WriteVerifyRawTrack(
                                Track {
                                    cylinder: Cylinder(self.cylinder as u8),
                                    head: Head(self.head as u8),
                                },
                                RawCellData::construct(speeds, recv_buffer),
                                self.first_significane_offset as usize,
                            ),
                        );

                        assert!(x.is_none());

                        //safeiprintln!("Cool");
                    }
                }
            }
        }

        orange(false);
    }
}
