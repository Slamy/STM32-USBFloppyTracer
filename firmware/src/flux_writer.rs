//! Writing of flux reversal pulses and general write head control

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::convert::Infallible;
use core::mem;
use cortex_m::interrupt::{CriticalSection, Mutex};
use heapless::spsc::Consumer;
use heapless::Vec;
use stm32f4xx_hal::hal::digital::v2::OutputPin;
use unwrap_infallible::UnwrapInfallible;

use stm32f4xx_hal::pac::{DMA1, TIM4};

/// Size of DMA buffer in bytes
pub const BUFFER_SIZE: usize = 16;

// Trackbuffer -> BitStream -> MfmEncoder -> FluxWriter

/**
 * Controls the write gate and write data pins of the floppy bus.
 * Falling edges are produced on the write data line with the distances between
 * them dictated by the data feeded into this.
 *
 * Output using Timer 4, Output Channel 3.
 * Connected to PB8.
 * Can be driven by DMA1, Channel 2, Stream 6 which reacts on TIM4_UP event.
 */
pub struct FluxWriter {
    tim4: TIM4,
    dma1: Arc<Mutex<DMA1>>,
    current_buffer: &'static mut Vec<u16, BUFFER_SIZE>, // used by the CPU
    back_buffer: &'static mut Vec<u16, BUFFER_SIZE>,    // used by the DMA unit
    last_dma_frame_active: bool,
    number_of_last_pulses: i32,
    cons: Consumer<'static, u32, 128>,
    write_gate: Box<dyn OutputPin<Error = Infallible> + Send>,
}

impl FluxWriter {
    /// IRQ Handler for Timer 4
    pub fn tim4_irq(&mut self, cs: &CriticalSection) {
        if self.tim4.sr.read().uif().is_update_pending() {
            self.tim4_pulse_complete_callback(cs);
            self.tim4.sr.write(|w| w.uif().clear()); // Clear interrupt
        } else {
            // Just ignore this. This can happen with the STM32F407.
            // The flag is not even set but I still get interrupts...
        }
    }

    /// IRQ Handler for DMA1
    /// Must be called when one DMA buffer was used and the DMA master has swapped to the other one
    /// We need to fill up the now used buffer up with fresh data.
    pub fn dma1_stream6_irq(&mut self, cs: &CriticalSection) {
        if self.dma1.borrow(cs).hisr.read().tcif6().is_complete() {
            self.dma_swapped_buffer_callback();
            self.dma1.borrow(cs).hifcr.write(|w| w.ctcif6().clear()); // Clear interrupt
        }

        assert!(
            !self.dma1.borrow(cs).hisr.read().teif6().is_error(),
            "DMA Error"
        );
    }

    fn tim4_pulse_complete_callback(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[6];

        if self.last_dma_frame_active {
            self.number_of_last_pulses -= 1;

            if self.number_of_last_pulses == 0 {
                dma_stream.cr.modify(|_, w| w.en().disabled()); // disable dma
            }
            if self.number_of_last_pulses == -1 {
                self.tim4
                    .ccmr2_output()
                    .modify(|_, w| w.oc3m().force_inactive());
            }
            if self.number_of_last_pulses == -2 {
                self.tim4.cr1.modify(|_, w| w.cen().clear_bit()); // disable timer
                self.write_gate.set_high().unwrap_infallible();
            }
        } else {
            panic!("Unexpected TIM4 IRQ ! Program flow error!");
        }
    }

    fn fill_buffer(&mut self) {
        // Clear the new current_buffer for new data
        self.current_buffer.clear();

        while self.cons.ready() && !self.current_buffer.is_full() {
            let pulse = self.cons.dequeue().expect("Is not possible to fail");
            self.current_buffer
                .push(pulse as u16)
                .expect("Is not possible to fail");
        }
    }

    /// Removes all entries from the FIFO
    pub fn clear_buffers(&mut self) {
        while self.cons.dequeue().is_some() {
            // Do nothing with the result
        }
    }

    fn dma_swapped_buffer_callback(&mut self) {
        // The current buffer with new data will now be moved to the back for the DMA unit
        mem::swap(&mut self.current_buffer, &mut self.back_buffer);

        if self.back_buffer.len() < self.back_buffer.capacity() {
            // We got less data than needed for a full transfer?
            self.last_dma_frame_active = true;
            self.number_of_last_pulses = self.back_buffer.len() as i32 + 1;
            self.tim4.dier.modify(|_, w| {
                w.uie().enabled() // enable update interrupt
            });
        }

        // load the current buffer with the next data to be ready for the next DMA request
        self.fill_buffer();
    }

    #[must_use]
    /// Returns `true` if write operation is currently in progress
    pub fn transmission_active(&self) -> bool {
        self.tim4.cr1.read().cen().is_enabled()
    }

    /// Prefills DMA buffers. This function must be called before transmission is started.
    /// It is required that both DMA buffers are filled. With `BUFFER_SIZE == 16`
    /// this means that 32 pulses must be provided using the FIFO before calling this function.
    pub fn prepare_transmit(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[6];

        assert!(!dma_stream.cr.read().en().is_enabled());
        assert!(!self.tim4.cr1.read().cen().is_enabled());

        self.current_buffer.clear();
        self.back_buffer.clear();

        // prefill the buffer with first data
        self.fill_buffer();

        // simulate a dma request to put the current first data into the backbuffer
        self.dma_swapped_buffer_callback();

        let dma_stream = &self.dma1.borrow(cs).st[6];

        self.last_dma_frame_active = false;
        self.number_of_last_pulses = 0;

        assert!(self.back_buffer.is_full());

        #[rustfmt::skip] // keep the config readable!
            dma_stream.cr.write(|w| {
                w.chsel().bits(2)
                    .msize().bits16()
                    .psize().bits16()
                    .minc().incremented() //memory increment
                    .dir().memory_to_peripheral()
                    .tcie().enabled() // enable transfer complete interrupt
                    .teie().enabled() // enable transfer error interrupt
                    .dmeie().enabled() // enable direct mode error interrupt
                    .dbm().enabled() // Double buffer mode
                    .pfctrl().dma() // DMA is the flow controller
            });

        // always transfer full buffers
        dma_stream.ndtr.write(|w| w.ndt().bits(BUFFER_SIZE as u16));

        unsafe {
            dma_stream
                .m0ar
                .write(|w| w.m0a().bits(self.back_buffer.as_ptr() as u32));
            dma_stream
                .m1ar
                .write(|w| w.m1a().bits(self.current_buffer.as_ptr() as u32));
            // provide pointer of timer reload count register to DMA unit.
            // unsafe is required as this effects memory without us knowing

            dma_stream
                .par
                .write(|w| w.pa().bits(self.tim4.arr.as_ptr() as u32));
        }

        #[rustfmt::skip] // keep the config readable!
            self.tim4.dier.write(|w| {w
                .ude().enabled() // enable update DMA request
            });
        self.tim4.ccmr2_output().modify(|_, w| w.oc3m().pwm_mode1()); //select pwm mode

        self.tim4.sr.write(|w| w.uif().clear()); // Clear interrupt

        self.tim4.cnt.write(|w| w.cnt().bits(400)); // reset count to 0
        self.tim4.arr.write(|w| w.arr().bits(400)); // count to 200 and reset
    }

    /// Activates write gate. Usefull for degaussing/erasing a track
    pub fn enable_write_head(&mut self) {
        self.write_gate.set_low().unwrap_infallible();
    }

    /// Starts transmission of prefilled buffers.
    pub fn start_transmit(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[6];

        self.write_gate.set_low().unwrap_infallible();

        dma_stream.cr.modify(|_, w| w.en().enabled()); // enable dma
        self.tim4.cr1.modify(|_, w| w.cen().set_bit()); // enable timer
    }

    /// Constructs with injected dependencies
    pub fn new(
        tim4: TIM4,
        dma1: Arc<Mutex<DMA1>>,
        cons: Consumer<'static, u32, 128>,
        write_gate: Box<dyn OutputPin<Error = Infallible> + Send>,
    ) -> Self {
        const ACTIVE_PULSE_LEN: u16 = 40;

        tim4.cr1.modify(|_, w| w.dir().down());

        tim4.ccr3().write(|w| w.ccr().bits(ACTIVE_PULSE_LEN)); // output compare value
        tim4.ccmr2_output().modify(|_, w| w.oc3m().force_inactive());

        tim4.ccer.write(|w| w.cc3e().set_bit().cc3p().set_bit()); //activate channel 3 output with inverse polarity
        tim4.cr2.write(|w| w.ccds().on_update()); // DMA request on update

        // allocate static global safe buffers for double buffering DMA
        let first_buffer: &'static mut Vec<u16, BUFFER_SIZE> =
            cortex_m::singleton!(: Vec::<u16, BUFFER_SIZE> = Vec::<u16, BUFFER_SIZE>::new())
                .unwrap();
        let second_buffer: &'static mut Vec<u16, BUFFER_SIZE> =
            cortex_m::singleton!(: Vec::<u16, BUFFER_SIZE> = Vec::<u16, BUFFER_SIZE>::new())
                .unwrap();

        Self {
            tim4,
            dma1,
            current_buffer: first_buffer,
            back_buffer: second_buffer,
            last_dma_frame_active: false,
            number_of_last_pulses: 0,
            cons,
            write_gate,
        }
    }
}
