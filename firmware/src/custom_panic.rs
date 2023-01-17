use core::panic::PanicInfo;
use rtt_target::rprintln;
use stm32f4xx_hal::{pac, prelude::*};

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cortex_m::interrupt::disable();

    let dp = unsafe { pac::Peripherals::steal() };

    let gpioa = dp.GPIOA.split();
    let gpiob = dp.GPIOB.split();

    gpioa
        .pa8
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    gpioa
        .pa15
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    gpiob
        .pb0
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);
    gpiob
        .pb1
        .into_push_pull_output_in_state(stm32f4xx_hal::gpio::PinState::High);

    rprintln!("{}", info);

    loop {
        // add some side effect to prevent this from turning into a UDF instruction
        // see rust-lang/rust#28728 for details
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
