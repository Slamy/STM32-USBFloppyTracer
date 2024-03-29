use core::{
    cell::{Cell, RefCell},
    future::Future,
    task::Poll,
};
use cortex_m::interrupt::Mutex;
use cortex_m_rt::exception;
use stm32f4xx_hal::{gpio::Pin, pac::interrupt, prelude::_stm32f4xx_hal_gpio_ExtiPin};

use cassette::futures::poll_fn;
use util::Track;

use crate::{
    floppy_control::FloppyControl, flux_reader::FluxReader, flux_writer::FluxWriter, rprintln,
};

pub static INDEX_OCCURED: Mutex<Cell<bool>> = Mutex::new(Cell::new(false));
pub static START_TRANSMIT_ON_INDEX: Mutex<Cell<bool>> = Mutex::new(Cell::new(false));
pub static START_RECEIVE_ON_INDEX: Mutex<Cell<bool>> = Mutex::new(Cell::new(false));

pub static FLUX_WRITER: Mutex<RefCell<Option<FluxWriter>>> = Mutex::new(RefCell::new(None));
pub static FLUX_READER: Mutex<RefCell<Option<FluxReader>>> = Mutex::new(RefCell::new(None));
pub static FLOPPY_CONTROL: Mutex<RefCell<Option<FloppyControl>>> = Mutex::new(RefCell::new(None));
pub static IN_INDEX: Mutex<RefCell<Option<Pin<'A', 3>>>> = Mutex::new(RefCell::new(None));

pub fn flux_reader_stop_reception() {
    cortex_m::interrupt::free(|cs| {
        FLUX_READER
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .expect("Program flow error")
            .stop_reception(cs);
    });
}

pub fn async_select_and_wait_for_track(track: Track) -> impl Future<Output = ()> {
    cortex_m::interrupt::free(|cs| {
        FLOPPY_CONTROL
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .expect("Program flow error")
            .select_track(track);
    });

    poll_fn(|_| {
        let reached = cortex_m::interrupt::free(|cs| {
            FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .expect("Program flow error")
                .reached_selected_cylinder()
        });

        if reached {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
}

pub fn async_wait_for_index() -> impl Future<Output = Result<(), ()>> {
    cortex_m::interrupt::free(|cs| {
        INDEX_OCCURED.borrow(cs).set(false);
    });

    poll_fn(|_| {
        let (index_occured, motor_spinning) = cortex_m::interrupt::free(|cs| {
            (
                INDEX_OCCURED.borrow(cs).get(),
                FLOPPY_CONTROL
                    .borrow(cs)
                    .borrow()
                    .as_ref()
                    .expect("Program flow error")
                    .is_spinning(),
            )
        });

        if index_occured {
            Poll::Ready(Ok(()))
        } else if !motor_spinning {
            Poll::Ready(Err(()))
        } else {
            Poll::Pending
        }
    })
}

pub fn async_wait_for_transmit() -> impl Future<Output = Result<(), ()>> {
    poll_fn(|_| {
        let (transmission_active, motor_spinning) = cortex_m::interrupt::free(|cs| {
            (
                FLUX_WRITER
                    .borrow(cs)
                    .borrow()
                    .as_ref()
                    .expect("Program flow error")
                    .transmission_active(),
                FLOPPY_CONTROL
                    .borrow(cs)
                    .borrow()
                    .as_ref()
                    .expect("Program flow error")
                    .is_spinning(),
            )
        });

        if transmission_active {
            Poll::Ready(Ok(()))
        } else if !motor_spinning {
            Poll::Ready(Err(()))
        } else {
            Poll::Pending
        }
    })
}

pub fn async_wait_for_receive() -> impl Future<Output = Result<(), ()>> {
    poll_fn(|_| {
        let (transmission_active, motor_spinning) = cortex_m::interrupt::free(|cs| {
            (
                FLUX_READER
                    .borrow(cs)
                    .borrow()
                    .as_ref()
                    .expect("Program flow error")
                    .transmission_active(),
                FLOPPY_CONTROL
                    .borrow(cs)
                    .borrow()
                    .as_ref()
                    .expect("Program flow error")
                    .is_spinning(),
            )
        });

        if transmission_active {
            Poll::Ready(Ok(()))
        } else if !motor_spinning {
            Poll::Ready(Err(()))
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
            .expect("Program flow error")
            .dma1_stream1_irq(cs);
    });
}

#[exception]
fn SysTick() {
    cortex_m::interrupt::free(|cs| {
        FLOPPY_CONTROL
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .expect("Program flow error")
            .run();
    });
}

#[interrupt]
fn EXTI3() {
    cortex_m::interrupt::free(|cs| {
        INDEX_OCCURED.borrow(cs).set(true);

        if FLUX_WRITER
            .borrow(cs)
            .borrow()
            .as_ref()
            .expect("Program flow error")
            .transmission_active()
        {
            rprintln!("Warning! Overwriting my own track!");
        }

        if START_TRANSMIT_ON_INDEX.borrow(cs).get() {
            START_TRANSMIT_ON_INDEX.borrow(cs).set(false);

            FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .expect("Program flow error")
                .start_transmit(cs);
        }

        if START_RECEIVE_ON_INDEX.borrow(cs).get() {
            START_RECEIVE_ON_INDEX.borrow(cs).set(false);

            FLUX_READER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .expect("Program flow error")
                .start_reception(cs);
        }

        IN_INDEX
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .expect("Program flow error")
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
            .expect("Program flow error")
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
            .expect("Program flow error")
            .dma1_stream6_irq(cs);
    });
}
