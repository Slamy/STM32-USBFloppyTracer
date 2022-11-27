use core::{
    cell::RefCell,
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};
use cortex_m::interrupt::Mutex;
use cortex_m_rt::exception;
use stm32f4xx_hal::{gpio::Pin, pac::interrupt, prelude::_stm32f4xx_hal_gpio_ExtiPin};

use cassette::futures::poll_fn;
use util::Track;

use crate::{
    floppy_control::FloppyControl, flux_reader::FluxReader, flux_writer::FluxWriter, safeiprintln,
    usb::UsbHandler,
};

//static SYSTICK_CNT: AtomicU32 = AtomicU32::new(0);
static INDEX_OCCURED: AtomicBool = AtomicBool::new(false);
static START_TRANSMIT_ON_INDEX: AtomicBool = AtomicBool::new(false);

pub static USB_HANDLER: Mutex<RefCell<Option<UsbHandler>>> = Mutex::new(RefCell::new(None));
pub static FLUX_WRITER: Mutex<RefCell<Option<FluxWriter>>> = Mutex::new(RefCell::new(None));
pub static FLUX_READER: Mutex<RefCell<Option<FluxReader>>> = Mutex::new(RefCell::new(None));
pub static FLOPPY_CONTROL: Mutex<RefCell<Option<FloppyControl>>> = Mutex::new(RefCell::new(None));
pub static IN_INDEX: Mutex<RefCell<Option<Pin<'A', 3>>>> = Mutex::new(RefCell::new(None));

pub fn async_select_and_wait_for_track(track: Track) -> impl Future<Output = ()> {
    cortex_m::interrupt::free(|cs| {
        FLOPPY_CONTROL
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .select_track(track);
    });

    poll_fn(|_| {
        let reached = cortex_m::interrupt::free(|cs| {
            FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .reached_selected_cylinder()
        });

        if reached {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
}

pub fn async_wait_for_index() -> impl Future<Output = ()> {
    INDEX_OCCURED.store(false, Ordering::Relaxed);

    poll_fn(|_| {
        if INDEX_OCCURED.swap(false, Ordering::Relaxed) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
}

#[interrupt]
fn DMA1_STREAM1() {
    cortex_m::interrupt::free(|cs| {
        FLUX_READER
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .dma1_stream1_irq(cs);
    });
}

#[interrupt]
fn OTG_FS() {
    cortex_m::interrupt::free(|cs| {
        USB_HANDLER
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .handle_interrupt(cs);
    });
}

#[exception]
fn SysTick() {
    //SYSTICK_CNT.fetch_add(1, Ordering::Relaxed);

    cortex_m::interrupt::free(|cs| {
        FLOPPY_CONTROL
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .run();
    });
}

#[interrupt]
fn EXTI3() {
    cortex_m::interrupt::free(|cs| {
        //safeiprintln!( "SysTick",);
        INDEX_OCCURED.store(true, Ordering::Relaxed);

        if FLUX_WRITER
            .borrow(cs)
            .borrow()
            .as_ref()
            .unwrap()
            .transmission_active()
        {
            safeiprintln!("Warning! Overwriting my own track!");
        }

        if START_TRANSMIT_ON_INDEX.swap(false, Ordering::Relaxed) {
            FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .start_transmit(cs);
        }

        IN_INDEX
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .clear_interrupt_pending_bit();
    });
}

#[interrupt]
fn TIM4() {
    cortex_m::interrupt::free(|cs| {
        FLUX_WRITER
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .tim4_irq(cs);
    });
}

#[interrupt]
fn DMA1_STREAM6() {
    cortex_m::interrupt::free(|cs| {
        FLUX_WRITER
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .unwrap()
            .dma1_stream6_irq(cs);
    });
}
