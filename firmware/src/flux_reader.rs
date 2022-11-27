use alloc::sync::Arc;

use core::mem;
use cortex_m::interrupt::{CriticalSection, Mutex};
use heapless::spsc::Producer;
use heapless::Vec;

use stm32f4xx_hal::pac::{DMA1, TIM2};

const BUFFER_SIZE: usize = 8;

/*
 * Input using Timer 2, Input Channel 3.
 * Connected to PA2.
 * Can be captured by DMA1, Channel 3, Stream 1 which reacts on the TIM2_CH3 or TIM2_UP event.
 */
pub struct FluxReader {
    tim2: TIM2,
    dma1: Arc<Mutex<DMA1>>,
    current_buffer: &'static mut Vec<u32, BUFFER_SIZE>, // used by the CPU
    back_buffer: &'static mut Vec<u32, BUFFER_SIZE>,    //used by the DMA unit
    last_pulse_cnt: u32,
    prod: Producer<'static, u32, 512>,
}

impl FluxReader {
    fn dma_swapped_buffer_callback(&mut self) {
        //assert!(self.current_buffer.is_empty());

        // The back buffer with new data will now be moved to current
        mem::swap(&mut self.current_buffer, &mut self.back_buffer);

        //orange(true);
        //return;
        for i in self.current_buffer.iter() {
            let duration = i.wrapping_sub(self.last_pulse_cnt);
            //let duration = i - self.last_pulse_cnt;

            //self.prod.enqueue(duration).expect_err("Queue is full!");

            //assert!(duration < 6000);
            self.prod.enqueue(duration).unwrap();
            //self.prod.enqueue(duration);

            self.last_pulse_cnt = *i;
        }
        //orange(false);
    }

    pub fn dma1_stream1_irq(&mut self, cs: &CriticalSection) {
        //safeiprintln!("DMA {}", self.dma1.borrow(cs).lisr.read().bits());

        if self.dma1.borrow(cs).lisr.read().tcif1().is_complete() {
            //safeiprintln!("DMA1");

            self.dma_swapped_buffer_callback();

            self.dma1.borrow(cs).lifcr.write(|w| w.ctcif1().clear()); // Clear interrupt
        }

        if self.dma1.borrow(cs).lisr.read().teif1().is_error() {
            panic!("DMA Error");
            self.dma1.borrow(cs).lifcr.write(|w| w.cteif1().clear());
        }
    }

    pub fn stop_reception(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[1];

        assert!(dma_stream.cr.read().en().is_enabled() == true);
        assert!(self.tim2.cr1.read().cen().is_enabled() == true);

        dma_stream.cr.modify(|_, w| w.en().disabled()); // enable dma
        self.tim2.cr1.modify(|_, w| w.cen().clear_bit()); // enable timer
    }

    pub fn start_reception(&mut self, cs: &CriticalSection) {
        let dma_stream = &self.dma1.borrow(cs).st[1];

        assert!(dma_stream.cr.read().en().is_enabled() == false);
        assert!(self.tim2.cr1.read().cen().is_enabled() == false);

        self.back_buffer.resize(BUFFER_SIZE, 0).unwrap();
        self.current_buffer.resize(BUFFER_SIZE, 0).unwrap();

        #[rustfmt::skip] // keep the config readable!
            dma_stream.cr.write(|w| {
                w.chsel().bits(3)
                    .msize().bits32()
                    .psize().bits32()
                    .minc().incremented() //memory increment
                    .dir().peripheral_to_memory()
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
            dma_stream
                .par
                .write(|w| w.pa().bits(self.tim2.ccr3.as_ptr() as u32));
        }

        self.tim2.cnt.write(|w| w.cnt().bits(0)); // reset count to 0
        self.last_pulse_cnt = 0;

        dma_stream.cr.modify(|_, w| w.en().enabled()); // enable dma
        self.tim2.cr1.modify(|_, w| w.cen().set_bit()); // enable timer
    }

    pub fn new(
        tim2: TIM2,
        dma1: Arc<Mutex<DMA1>>,
        prod: Producer<'static, u32, 512>,
    ) -> FluxReader {
        //tim2.cr1.modify(|_, w| w.ckd().div4().dir().up()); // count up
        tim2.cr1.modify(|_, w| w.dir().up()); // count up

        tim2.ccmr2_input().write(|w| w.cc3s().ti3()); // select active input.
        tim2.ccer.write(|w| w.cc3e().set_bit()); // enable capture on channel 3
                                                 //tim2.dier.write(|w| w.cc3de().enabled().cc3ie().enabled()); // DMA request for channel 3
        tim2.dier.write(|w| w.cc3de().enabled()); // DMA request for channel 3

        // allocate static global safe buffers for double buffering DMA
        let first_buffer: &'static mut Vec<u32, BUFFER_SIZE> =
            cortex_m::singleton!(: Vec::<u32, BUFFER_SIZE> = Vec::<u32, BUFFER_SIZE>::new())
                .unwrap();
        let second_buffer: &'static mut Vec<u32, BUFFER_SIZE> =
            cortex_m::singleton!(: Vec::<u32, BUFFER_SIZE> = Vec::<u32, BUFFER_SIZE>::new())
                .unwrap();

        FluxReader {
            prod,
            dma1,
            tim2,
            current_buffer: first_buffer,
            back_buffer: second_buffer,
            last_pulse_cnt: 0,
        }
    }
}
