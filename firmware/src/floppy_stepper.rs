//! Algorithms for correctly timed stepping of the head

use core::convert::Infallible;

use alloc::boxed::Box;
use stm32f4xx_hal::{
    gpio::PinState,
    hal::digital::v2::{InputPin, OutputPin, StatefulOutputPin},
};
use unwrap_infallible::UnwrapInfallible;

use crate::floppy_drive_unit::HeadPosition;

/// Collects all required GPIOs for head stepping related activities
pub struct FloppyStepperSignals {
    out_step_direction: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
    out_step_perform: Box<dyn OutputPin<Error = Infallible> + Send>,
    in_track_00: Box<dyn InputPin<Error = Infallible> + Send>,
}

async fn wait(steps: usize) {
    for _ in 0..steps {
        cassette::yield_now().await;
    }
}

#[derive(Clone, Copy, Debug)]
enum StepDirection {
    Inward,
    Outward,
}

const DURATION_CHANGE_SETTLE_TIME: usize = 10;
const HEAD_SETTLE_TIME: usize = 10;

async fn wait_for_head_to_settle() {
    wait(HEAD_SETTLE_TIME).await;
}

impl FloppyStepperSignals {
    #[must_use]
    /// Constructs with injected dependencies
    pub fn new(
        out_step_direction: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
        out_step_perform: Box<dyn OutputPin<Error = Infallible> + Send>,
        in_track_00: Box<dyn InputPin<Error = Infallible> + Send>,
    ) -> Self {
        Self {
            out_step_direction,
            out_step_perform,
            in_track_00,
        }
    }

    async fn set_direction(&mut self, direction: StepDirection) {
        let state = match direction {
            StepDirection::Inward => PinState::Low,
            StepDirection::Outward => PinState::High,
        };
        self.out_step_direction.set_state(state).unwrap_infallible();
        wait(DURATION_CHANGE_SETTLE_TIME).await;
    }

    async fn perform_step(&mut self) {
        self.out_step_perform.set_low().unwrap_infallible();
        cassette::yield_now().await;
        self.out_step_perform.set_high().unwrap_infallible();
        cassette::yield_now().await;
    }

    /// Asynchronous function for stepping the head to a provided cylinder.
    /// If the current position of the Head is not known, the function
    /// will first step outside until the first cyclinder is detected.
    /// Will return with the current head position upon arrival
    pub async fn step_to_cylinder(
        mut self,
        current_position: HeadPosition,
        wanted_cylinder: u32,
    ) -> (Self, HeadPosition) {
        let current_pos = match current_position {
            HeadPosition::Unknown => {
                // We need to get to track 0 before we know our position
                self.set_direction(StepDirection::Outward).await;

                for _ in 0..90 {
                    self.perform_step().await;

                    if self.in_track_00.is_low().unwrap_infallible() {
                        break;
                    }
                }
                wait_for_head_to_settle().await;
                if self.in_track_00.is_high().unwrap_infallible() {
                    return (self, HeadPosition::Unknown);
                };
                0 // Head position is now known as cylinder 0
            }
            HeadPosition::Cylinder(pos) => pos,
        };

        if current_pos == wanted_cylinder {
            return (self, HeadPosition::Cylinder(current_pos));
        }

        self.set_direction(if current_pos < wanted_cylinder {
            StepDirection::Inward
        } else {
            StepDirection::Outward
        })
        .await;

        let steps_to_perform = current_pos.abs_diff(wanted_cylinder);

        for _ in 0..steps_to_perform {
            self.perform_step().await;
        }
        wait_for_head_to_settle().await;

        (self, HeadPosition::Cylinder(wanted_cylinder))
    }
}
