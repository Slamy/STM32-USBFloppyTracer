#![no_std]
#![no_main]
#![feature(let_chains)]
#![warn(clippy::indexing_slicing)]
#![warn(clippy::panic_in_result_fn)]
#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]

pub mod floppy_control;
pub mod floppy_drive_unit;
pub mod floppy_stepper;
pub mod flux_reader;
pub mod flux_writer;
pub mod index_sim;
pub mod interrupts;
pub mod track_raw;
pub mod usb;
pub mod vendor_class;

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use cassette::Cassette;
use core::cell::RefCell;
use cortex_m::interrupt::Mutex;
use cortex_m_rt::entry;
use floppy_control::FloppyControl;
use flux_reader::FluxReader;
use flux_writer::FluxWriter;
use heapless::spsc::Queue;
use index_sim::IndexSim;
use panic_persist::get_panic_message_bytes;
use rtt_target::{rprintln, rtt_init_print};
use stm32f4xx_hal::gpio::{Alternate, Edge, Output, Pin, Pull, PushPull};
use stm32f4xx_hal::otg_fs::USB;
use stm32f4xx_hal::pac::Interrupt;
use stm32f4xx_hal::{pac, prelude::*};
use track_raw::{RawTrackHandler, WriteVerifyError, WriteVerifySuccess};
use usb::UsbHandler;
use usb_device::class_prelude::UsbBusAllocator;
use usb_device::prelude::*;
use util::{USB_PID, USB_VID};
use vendor_class::Command;

static DEBUG_LED_GREEN: Mutex<RefCell<Option<Pin<'D', 12, Output>>>> =
    Mutex::new(RefCell::new(None));

static INDEX_SIM: Mutex<RefCell<Option<IndexSim>>> = Mutex::new(RefCell::new(None));

use alloc::sync::Arc;
use alloc_cortex_m::CortexMHeap;

use crate::floppy_drive_unit::FloppyDriveUnit;
use crate::floppy_stepper::FloppyStepperSignals;
use crate::vendor_class::FloppyTracerVendorClass;

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();

#[inline(always)]
pub fn orange(s: bool) {
    if s {
        unsafe { (*pac::GPIOD::ptr()).bsrr.write(|w| w.bits(1 << 13)) };
    } else {
        unsafe { (*pac::GPIOD::ptr()).bsrr.write(|w| w.bits(1 << (13 + 16))) };
    }
}

#[entry]
fn main() -> ! {
    // Initialize the allocator BEFORE you use it
    // give some static memory to the pool

    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 13509 * 8;
        static mut HEAP: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE) }
    }

    rtt_init_print!();

    // there is no alternative to unwrap here. It will and shall never fail
    let mut dp = pac::Peripherals::take().expect("Must never fail");
    let mut cp = cortex_m::Peripherals::take().expect("Must never fail");

    cp.DWT.enable_cycle_counter();
    dp.RCC.apb1enr.modify(|_, w| w.tim2en().set_bit());
    dp.RCC.apb1enr.modify(|_, w| w.tim4en().set_bit());
    dp.RCC.apb1enr.modify(|_, w| w.tim5en().set_bit()); // for index sim
    dp.RCC.ahb1enr.modify(|_, w| w.dma1en().set_bit());

    let rcc = dp.RCC.constrain();

    let clocks = rcc.cfgr.sysclk((168).MHz()).freeze();

    let gpioa = dp.GPIOA.split();
    let gpiob = dp.GPIOB.split();
    let gpiod = dp.GPIOD.split();

    // grab all important pins and configure them
    let debug_led_green = gpiod.pd12.into_push_pull_output();
    let _debug_led_orange = gpiod.pd13.into_push_pull_output();

    // flippy disk index simulator
    let _out_index_sim: Pin<'A', 1, Alternate<2, _>> = gpioa
        .pa1
        .into_alternate_open_drain()
        .internal_resistor(Pull::None); // index sim on PA1, connected to TIM5_CH2, AF2

    let index_sim = IndexSim::new(dp.TIM5);

    // now for the floppy bus pins in the order of the connector
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
    let in_write_protect = gpiob.pb14.into_pull_up_input();
    let _in_read_data: Pin<'A', 2, Alternate<1>> = gpioa.pa2.into_alternate(); // TIM2_CH3, AF1
    let out_head_select = gpiob
        .pb11
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    let _in_disk_change_ready = gpiob.pb12.into_pull_up_input();

    // Check if there was a panic message, if so, send to UART
    if let Some(msg) = get_panic_message_bytes() {
        rprintln!(
            "Panic: {}",
            core::str::from_utf8(msg).unwrap_or("UTF parse error")
        );
    }

    let drive_a = FloppyDriveUnit::new(Box::new(out_motor_enable_a), Box::new(out_drive_select_a));
    let drive_b = FloppyDriveUnit::new(Box::new(out_motor_enable_b), Box::new(out_drive_select_b));
    let stepper = FloppyStepperSignals::new(
        Box::new(out_step_direction),
        Box::new(out_step_perform),
        Box::new(in_track_00),
    );

    let floppy_control = FloppyControl::new(
        drive_a,
        drive_b,
        stepper,
        Box::new(out_head_select),
        Box::new(out_density_select),
        Box::new(in_write_protect),
    );

    let usb = USB {
        usb_global: dp.OTG_FS_GLOBAL,
        usb_device: dp.OTG_FS_DEVICE,
        usb_pwrclk: dp.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate(),
        pin_dp: gpioa.pa12.into_alternate(),
        hclk: clocks.hclk(),
    };

    let usb_memory = cortex_m::singleton!(: [u32; 1024] = [0; 1024]);
    let usb_bus = &*cortex_m::singleton!(: UsbBusAllocator<stm32f4xx_hal::otg_fs::UsbBusType> = stm32f4xx_hal::otg_fs::UsbBusType::new(usb, usb_memory.expect("Must never fail"))).unwrap();

    let dma1 = dp.DMA1;
    let dma1_arc1 = Arc::new(Mutex::new(dma1));
    let dma1_arc2 = Arc::clone(&dma1_arc1);
    rprintln!("Go Go!");

    let mut syst = cp.SYST;
    syst.set_reload(168_000 / 4);
    syst.clear_current();
    syst.enable_counter();
    syst.enable_interrupt();

    cortex_m::interrupt::free(|cs| {
        *INDEX_SIM.borrow(cs).borrow_mut() = Some(index_sim);
    });

    let reading_buffer = cortex_m::singleton!(: Queue<u32,512> = Queue::new()).unwrap();
    let writing_buffer = cortex_m::singleton!(: Queue<u32,128> = Queue::new()).unwrap();

    let (read_prod, read_cons) = reading_buffer.split();
    let (write_prod, write_cons) = writing_buffer.split();

    let flux_writer = FluxWriter::new(dp.TIM4, dma1_arc2, write_cons, Box::new(out_write_gate));
    let flux_reader = FluxReader::new(dp.TIM2, dma1_arc1, read_prod);

    let serial = FloppyTracerVendorClass::new(usb_bus, 64);

    let usb_device = UsbDeviceBuilder::new(usb_bus, UsbVidPid(USB_VID, USB_PID))
        .manufacturer("Slamy")
        .product("STM32-USBFloppyTracer")
        .device_class(0xff)
        .build();

    let usb_handler = UsbHandler::new(serial, usb_device);

    cortex_m::interrupt::free(|cs| {
        DEBUG_LED_GREEN
            .borrow(cs)
            .borrow_mut()
            .replace(debug_led_green);

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
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM4);
        cortex_m::peripheral::NVIC::unmask(Interrupt::DMA1_STREAM6); // flux writing
        cortex_m::peripheral::NVIC::unmask(Interrupt::DMA1_STREAM1); // flux reading
        cortex_m::peripheral::NVIC::unmask(in_index_int);
    }

    let raw_track_writer = track_raw::RawTrackHandler {
        read_cons,
        write_prod_cell: RefCell::new(write_prod),
    };

    mainloop(usb_handler, raw_track_writer);
}

fn mainloop(mut usb_handler: UsbHandler, mut raw_track_writer: RawTrackHandler) -> ! {
    let mut next_command: Option<Command>;

    loop {
        usb_handler.handle();
        next_command = usb_handler.vendor_class.take_command();

        match next_command.take() {
            Some(Command::ReadTrack {
                track,
                duration_to_record,
                wait_for_index,
            }) => {
                let write_verify_fut = Box::pin(raw_track_writer.read_track(
                    track,
                    duration_to_record,
                    wait_for_index,
                    &mut usb_handler,
                ));
                let cm = Cassette::new(write_verify_fut);

                let result = cm.block_on();
                if let Err(err) = result {
                    let str_response = format!("Fail {err:?}");
                    usb_handler.vendor_class.response(&str_response);
                }
            }
            Some(Command::WriteVerifyRawTrack {
                track,
                raw_cell_data,
                write_precompensation,
            }) => {
                usb_handler.vendor_class.response("GotCmd");

                cortex_m::interrupt::free(|cs| {
                    interrupts::FLOPPY_CONTROL
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .expect("Program flow error")
                        .spin_motor();
                });

                let write_verify_fut = Box::pin(raw_track_writer.write_and_verify(
                    track,
                    write_precompensation,
                    raw_cell_data,
                ));
                let mut cm = Cassette::new(write_verify_fut);

                let result = loop {
                    usb_handler.handle();

                    if let Some(result) = cm.poll_on() {
                        break result;
                    }
                };

                let str_response = match result {
                    Ok(WriteVerifySuccess {
                        write_operations,
                        verify_operations,
                        max_err,
                        write_precompensation,
                    }) => {
                        format!(
                            "WrittenAndVerified {} {} {} {} {} {}",
                            track.cylinder.0,
                            track.head.0,
                            write_operations,
                            verify_operations,
                            max_err.0,
                            write_precompensation.0
                        )
                    }
                    Err(WriteVerifyError {
                        write_operations,
                        verify_operations,
                        error,
                    }) => format!(
                        "Fail {} {} {} {} {:?}",
                        track.cylinder.0, track.head.0, write_operations, verify_operations, error
                    ),
                };

                usb_handler.vendor_class.response(&str_response);
            }
            _ => {}
        }
    }
}
