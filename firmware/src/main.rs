#![no_std]
#![no_main]
#![feature(default_alloc_error_handler)]

use alloc::boxed::Box;

use alloc::format;
use alloc::vec::Vec;

use flux_reader::FluxReader;
use flux_writer::FluxWriter;

use stm32f4xx_hal::gpio::{Alternate, Edge, Output, Pin, PinState, PushPull};

use core::borrow::{Borrow, BorrowMut};
use core::cell::RefCell;

use cortex_m::interrupt::Mutex;
use cortex_m::iprintln;
use cortex_m_rt::entry;
use heapless::spsc::Queue;
use stm32f4xx_hal::otg_fs::USB;
use stm32f4xx_hal::pac::Interrupt;
use stm32f4xx_hal::{pac, prelude::*};
use usb_device::class_prelude::UsbBusAllocator;
use usb_device::prelude::*;
use usbd_serial::SerialPort;

use cassette::Cassette;

pub mod usb;
use usb::UsbHandler;
pub mod interrupts;
pub mod track_raw;

extern crate alloc;

pub mod flux_reader;
pub mod flux_writer;

pub mod floppy_control;

use floppy_control::FloppyControl;

pub mod sector_iso;

pub mod custom_panic;

static DEBUG_LED_GREEN: Mutex<RefCell<Option<Pin<'D', 12, Output>>>> =
    Mutex::new(RefCell::new(None));

static ITM: Mutex<RefCell<Option<cortex_m::peripheral::ITM>>> = Mutex::new(RefCell::new(None));

use alloc::sync::Arc;
use alloc_cortex_m::CortexMHeap;

use crate::sector_iso::Order;
use crate::usb::CURRENT_COMMAND;

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();

#[macro_export]
macro_rules! safeiprintln {
    () => {
        cortex_m::interrupt::free(|cs| {
            cortex_m::itm::write_str(&mut crate::ITM.borrow(cs).borrow_mut().as_mut().unwrap().stim[0], "\n");
        });
    };
    ( $fmt:expr) => {
        cortex_m::interrupt::free(|cs| {
            cortex_m::itm::write_str(&mut crate::ITM.borrow(cs).borrow_mut().as_mut().unwrap().stim[0], concat!($fmt, "\n"));
        });
    };
    ( $fmt:expr, $($arg:tt)*) => {
        cortex_m::interrupt::free(|cs| {
            cortex_m::itm::write_fmt(&mut crate::ITM.borrow(cs).borrow_mut().as_mut().unwrap().stim[0], format_args!(concat!($fmt, "\n"), $($arg)*));
        });
    };
}

#[inline(always)]
fn orange(s: bool) {
    if s {
        unsafe { (*pac::GPIOD::ptr()).bsrr.write(|w| w.bits(1 << 13)) };
    } else {
        unsafe { (*pac::GPIOD::ptr()).bsrr.write(|w| w.bits(1 << (13 + 16))) };
    }
}

fn green(s: bool) {
    cortex_m::interrupt::free(|cs| {
        DEBUG_LED_GREEN
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .set_state(if s { PinState::High } else { PinState::Low });
    });
}

fn create_example_data() -> Vec<u8> {
    // prepare some test data to write
    let mut track_data_to_write: Vec<u8> = Vec::with_capacity(13509);

    // pretty ugly example of a very late sync word
    track_data_to_write.push(0x2a);
    for _ in 0..100 {
        track_data_to_write.push(0xaa);
    }
    track_data_to_write.push(0x44);
    track_data_to_write.push(0x89);
    track_data_to_write.push(0x2a);

    for _ in 0..24800 / 4 {
        track_data_to_write.push(0xaa);
        track_data_to_write.push(0x92);
        track_data_to_write.push(0xA4);
        track_data_to_write.push(0xaa);
    }

    track_data_to_write
}

#[entry]
fn main() -> ! {
    // Initialize the allocator BEFORE you use it
    // give some static memory to the pool

    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 13509 * 7;
        static mut HEAP: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE) }
    }

    let mut dp = pac::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

    cp.DWT.enable_cycle_counter();
    dp.RCC.apb1enr.modify(|_, w| w.tim2en().set_bit());
    dp.RCC.apb1enr.modify(|_, w| w.tim4en().set_bit());
    dp.RCC.ahb1enr.modify(|_, w| w.dma1en().set_bit());

    let rcc = dp.RCC.constrain();

    let clocks = rcc.cfgr.sysclk((168).MHz()).freeze();
    // let clocks = rcc.cfgr.sysclk((168).MHz()).pclk1((8).MHz()).freeze(); // slower peripherals

    let gpioa = dp.GPIOA.split();
    let gpiob = dp.GPIOB.split();
    let gpiod = dp.GPIOD.split();

    // grab all important pins and configure them
    let debug_led_green = gpiod.pd12.into_push_pull_output();
    let _debug_led_orange = gpiod.pd13.into_push_pull_output();

    // now for the floppy bus pins
    let out_density_select = gpiob
        .pb13
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let mut in_index = gpioa.pa3.into_pull_up_input();
    let out_motor_enable_a = gpioa
        .pa8
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let out_drive_select_b = gpioa
        .pa15
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let out_drive_select_a = gpiob
        .pb0
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let out_motor_enable_b = gpiob
        .pb1
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let out_step_direction = gpiob
        .pb2
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let out_step_perform = gpiob
        .pb4
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let _out_write_data: Pin<'B', 8, Alternate<2, PushPull>> = gpiob.pb8.into_alternate(); // write data on PB8, connected to TIM4_CH3, AF2
    let out_write_gate = gpiob
        .pb5
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let in_track_00 = gpiob.pb7.into_pull_up_input();
    let _in_read_data: Pin<'A', 2, Alternate<1>> = gpioa.pa2.into_alternate(); // TIM2_CH3, AF1
    let out_head_select = gpiob
        .pb11
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let _in_disk_change_ready = gpiob.pb12.into_pull_up_input();

    let floppy_control = FloppyControl::new(
        out_motor_enable_a,
        out_drive_select_b,
        out_drive_select_a,
        out_motor_enable_b,
        out_step_direction,
        out_step_perform,
        in_track_00,
        out_head_select,
        out_density_select,
    );

    let usb = USB {
        usb_global: dp.OTG_FS_GLOBAL,
        usb_device: dp.OTG_FS_DEVICE,
        usb_pwrclk: dp.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate(),
        pin_dp: gpioa.pa12.into_alternate(),
        hclk: clocks.hclk(),
    };

    let x = cortex_m::singleton!(: [u32; 1024] = [0; 1024]);
    let usb_bus = &*cortex_m::singleton!(: UsbBusAllocator<stm32f4xx_hal::otg_fs::UsbBusType> = stm32f4xx_hal::otg_fs::UsbBusType::new(usb, x.unwrap())).unwrap();

    let dma1 = dp.DMA1;
    let dma1_arc1 = Arc::new(Mutex::new(dma1));
    let dma1_arc2 = Arc::clone(&dma1_arc1);
    let mut itm = cp.ITM;
    iprintln!(&mut itm.stim[0], "Go Go!");

    let mut syst = cp.SYST;
    syst.set_reload(168000 / 4);
    syst.clear_current();
    syst.enable_counter();
    syst.enable_interrupt();

    cortex_m::interrupt::free(|cs| {
        *ITM.borrow(cs).borrow_mut() = Some(itm);
    });

    let reading_buffer: &mut Queue<u32, 512> =
        cortex_m::singleton!(: Queue<u32,512> = Queue::new()).unwrap();

    let writing_buffer: &mut Queue<u32, 128> =
        cortex_m::singleton!(: Queue<u32,128> = Queue::new()).unwrap();

    let (read_prod, read_cons) = reading_buffer.split();
    let (write_prod, write_cons) = writing_buffer.split();

    let flux_writer = FluxWriter::new(dp.TIM4, dma1_arc2, write_cons, out_write_gate);
    let flux_reader = FluxReader::new(dp.TIM2, dma1_arc1, read_prod);

    cortex_m::interrupt::free(|cs| {
        DEBUG_LED_GREEN
            .borrow(cs)
            .borrow_mut()
            .replace(debug_led_green);

        let serial = SerialPort::new(usb_bus);

        let usb_device = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .manufacturer("Slamy")
            .product("WuselDerpy")
            .device_class(0xff)
            .build();

        let usb_handler = UsbHandler::new(serial, usb_device);

        *interrupts::USB_HANDLER.borrow(cs).borrow_mut() = Some(usb_handler);
        *interrupts::FLUX_WRITER.borrow(cs).borrow_mut() = Some(flux_writer);
        *interrupts::FLUX_READER.borrow(cs).borrow_mut() = Some(flux_reader);
    });

    let in_index_int = in_index.interrupt();

    let mut syscfg = dp.SYSCFG.constrain();

    in_index.make_interrupt_source(&mut syscfg);
    in_index.enable_interrupt(&mut dp.EXTI);
    in_index.trigger_on_edge(&mut dp.EXTI, Edge::Falling);

    cortex_m::interrupt::free(|cs| {
        *interrupts::IN_INDEX.borrow(cs).borrow_mut() = Some(in_index);
        *interrupts::FLOPPY_CONTROL.borrow(cs).borrow_mut() = Some(floppy_control);
    });

    unsafe {
        cortex_m::peripheral::NVIC::unmask(Interrupt::OTG_FS);
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM4);
        cortex_m::peripheral::NVIC::unmask(Interrupt::DMA1_STREAM6); // flux writing
        cortex_m::peripheral::NVIC::unmask(Interrupt::DMA1_STREAM1); // flux reading
        cortex_m::peripheral::NVIC::unmask(in_index_int);
    }

    let mut next_command: Option<usb::Command> = None;

    let mut raw_track_writer = track_raw::RawTrackWriter {
        read_cons,
        write_prod_cell: RefCell::new(write_prod),
        track_data_to_write: None,
    };

    loop {
        cortex_m::interrupt::free(|cs| {
            next_command = CURRENT_COMMAND.borrow(cs).borrow_mut().take();
        });

        match next_command.take() {
            Some(usb::Command::WriteVerifyRawTrack(
                track,
                raw_cell_data,
                first_significance_offset,
            )) => {
                cortex_m::interrupt::free(|cs| {
                    interrupts::USB_HANDLER
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .response("GotCmd");

                    interrupts::FLOPPY_CONTROL
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .spin_motor();
                });

                raw_track_writer.track_data_to_write = Some(raw_cell_data);
                let write_verify_fut =
                    Box::pin(raw_track_writer.write_and_verify(track, first_significance_offset));
                let cm = Cassette::new(write_verify_fut);
                let result = cm.block_on();

                let str_response = if result.2 {
                    format!(
                        "WrittenAndVerified {} {} {} {}",
                        track.cylinder.0, track.head.0, result.0, result.1
                    )
                } else {
                    format!(
                        "Fail {} {} {} {}",
                        track.cylinder.0, track.head.0, result.0, result.1
                    )
                };

                cortex_m::interrupt::free(|cs| {
                    interrupts::USB_HANDLER
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .response(&str_response);
                });
            }
            _ => {}
        }
    }
}
