use stm32f4xx_hal::pac::TIM5;

pub struct IndexSignal {
    tim5: TIM5,
}

impl IndexSignal {
    #[must_use]
    pub fn new(tim5: TIM5) -> Self {
        tim5.cr1.modify(|_, w| w.dir().up());
        tim5.cnt.write(|w| w.cnt().bits(0)); // reset count to 0
        tim5.arr.write(|w| w.arr().bits(14 * 1000 * 1000)); // 6 Hz == 360 RPM
        tim5.ccr2().write(|w| w.ccr().bits(200_000)); // output compare value, have something like 3ms
        tim5.ccmr1_output().modify(|_, w| w.oc2m().force_inactive());
        tim5.ccer.write(|w| w.cc2e().set_bit().cc2p().set_bit()); //activate channel 2 output with inverted polarity

        Self { tim5 }
    }

    pub fn configure_index_sim(&self, frequency: u32) {
        self.tim5.cnt.write(|w| w.cnt().bits(0)); // reset count to 0

        if frequency > 0 {
            self.tim5.arr.write(|w| w.arr().bits(frequency)); // 6 Hz == 360 RPM
            self.tim5.ccmr1_output().modify(|_, w| w.oc2m().pwm_mode1());
            self.tim5.cr1.modify(|_, w| w.cen().set_bit()); // enable timer
        } else {
            self.tim5
                .ccmr1_output()
                .modify(|_, w| w.oc2m().force_inactive());
            self.tim5.cr1.modify(|_, w| w.cen().clear_bit()); // disable timer
        }
    }

    pub fn measure_index_period(&self) {
        // Disable manipulation of index sim pin
        self.tim5
            .ccmr1_output()
            .modify(|_, w| w.oc2m().force_inactive());

        self.tim5.cnt.write(|w| w.cnt().bits(0)); // reset count to 0
        self.tim5.arr.write(|w| w.arr().bits(0xffff_ffff)); // Don't reload
        self.tim5.cr1.modify(|_, w| w.cen().set_bit()); // enable timer
    }

    pub fn get_measured_index_period(&self) -> u32 {
        let result = self.tim5.cnt.read().cnt().bits();
        self.tim5.cr1.modify(|_, w| w.cen().clear_bit()); // disable timer
        result
    }
}
