use alloc::sync::Arc;
use core::mem;
use cortex_m::interrupt::{CriticalSection, Mutex};
use heapless::spsc::Consumer;
use heapless::Vec;
use stm32f4xx_hal::gpio::{Output, Pin};

use stm32f4xx_hal::pac::{DMA1, TIM4};

const BUFFER_SIZE: usize = 16;

// Trackbuffer -> BitStream -> MfmEncoder -> FluxWriter

/*
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
    write_gate: Pin<'B', 5, Output>,
}

impl FluxWriter {
    pub fn tim4_irq(&mut self, cs: &CriticalSection) {
        if self.tim4.sr.read().uif().is_update_pending() {
            self.tim4_pulse_complete_callback(cs);
            self.tim4.sr.write(|w| w.uif().clear()); // Clear interrupt
        } else {
            // Just ignore this. This can happen with the STM32F407.
            // The flag is not even set but I still get interrupts...
        }
    }

    pub fn dma1_stream6_irq(&mut self, cs: &CriticalSection) {
        if self.dma1.borrow(cs).hisr.read().tcif6().is_complete() {
            self.dma_swapped_buffer_callback();
            self.dma1.borrow(cs).hifcr.write(|w| w.ctcif6().clear()); // Clear interrupt
        }

        if self.dma1.borrow(cs).hisr.read().teif6().is_error() {
            panic!("DMA Error");
        }
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
                self.write_gate.set_high();
            }
        } else {
            panic!("Wasted TIM4 IRQ !");
        }
    }

    fn fill_buffer(&mut self) {
        // Clear the new current_buffer for new data
        self.current_buffer.clear();

        while self.cons.ready() && !self.current_buffer.is_full() {
            let pulse = self.cons.dequeue().unwrap();
            self.current_buffer.push(pulse as u16).unwrap();
        }
    }

    pub fn clear_buffers(&mut self) {
        while self.cons.ready() {
            self.cons.dequeue().unwrap();
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

    pub fn transmission_active(&self) -> bool {
        self.tim4.cr1.read().cen().is_enabled()
    }

    pub fn start_transmit(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[6];

        assert!(dma_stream.cr.read().en().is_enabled() == false);
        assert!(self.tim4.cr1.read().cen().is_enabled() == false);

        self.current_buffer.clear();
        self.back_buffer.clear();

        // prefill the buffer with first data
        self.fill_buffer();

        // simulate a dma request to put the current first data into the backbuffer
        self.dma_swapped_buffer_callback();

        let dma_stream = &self.dma1.borrow(cs).st[6];

        self.last_dma_frame_active = false;
        self.number_of_last_pulses = 0;

        assert!(self.back_buffer.is_full() == true);

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

        self.tim4.cnt.write(|w| w.cnt().bits(0)); // reset count to 0
        self.tim4.arr.write(|w| w.arr().bits(400)); // count to 200 and reset
        self.write_gate.set_low();

        dma_stream.cr.modify(|_, w| w.en().enabled()); // enable dma
        self.tim4.cr1.modify(|_, w| w.cen().set_bit()); // enable timer
    }

    pub fn new(
        tim4: TIM4,
        dma1: Arc<Mutex<DMA1>>,
        cons: Consumer<'static, u32, 128>,
        write_gate: Pin<'B', 5, Output>,
    ) -> Self {
        const ACTIVE_PULSE_LEN: u16 = 40;

        tim4.cr1.modify(|_, w| w.dir().down());

        tim4.ccr3.write(|w| w.ccr().bits(ACTIVE_PULSE_LEN)); // output compare value
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
