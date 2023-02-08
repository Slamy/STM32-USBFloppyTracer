use core::convert::Infallible;

use alloc::boxed::Box;
use stm32f4xx_hal::hal::digital::v2::{OutputPin, StatefulOutputPin};

enum MotorState {
    Off,
    On(u32),
}
pub enum HeadPosition {
    Unknown,
    Cylinder(u32),
}

pub struct FloppyDriveUnit {
    out_motor_enable: Box<dyn OutputPin<Error = Infallible> + Send>,
    out_drive_select: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
    motor_state: MotorState,
    head_position: Option<HeadPosition>,
}

impl FloppyDriveUnit {
    pub fn new(
        out_motor_enable: Box<dyn OutputPin<Error = Infallible> + Send>,
        out_drive_select: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
    ) -> Self {
        FloppyDriveUnit {
            out_motor_enable,
            out_drive_select,
            motor_state: MotorState::Off,
            head_position: Some(HeadPosition::Unknown),
        }
    }

    pub fn run(&mut self) {
        if let MotorState::On(count) = self.motor_state {
            if count > 0 {
                self.motor_state = MotorState::On(count - 1);
            } else {
                self.stop_motor();
            }
        }
    }

    pub fn spin_motor(&mut self) {
        self.out_motor_enable.set_low().unwrap();
        self.out_drive_select.set_low().unwrap();
        self.motor_state = MotorState::On(600);
    }

    pub fn disable_select_signal_if_possible(&mut self) {
        if matches!(self.motor_state, MotorState::Off) && self.head_position.is_some() {
            self.out_drive_select.set_high().unwrap();
        }
    }

    pub fn selection_signal_active(&self) -> bool {
        self.out_drive_select.is_set_low().unwrap()
    }

    pub fn stop_motor(&mut self) {
        self.out_motor_enable.set_high().unwrap();
        self.motor_state = MotorState::Off;
        self.disable_select_signal_if_possible();
    }

    pub fn is_spinning(&self) -> bool {
        matches!(self.motor_state, MotorState::On(_))
    }

    pub fn take_head_position_for_stepping(&mut self) -> HeadPosition {
        let taken = self.head_position.take().unwrap();
        self.out_drive_select.set_low().unwrap();
        taken
    }

    pub fn insert_current_head_position(&mut self, pos: HeadPosition) {
        let old = self.head_position.replace(pos);
        assert!(old.is_none(), "Program flow error");
        self.disable_select_signal_if_possible();
    }

    pub fn head_position_equals(&mut self, cylinder: u32) -> bool {
        if let Some(HeadPosition::Cylinder(c)) = self.head_position.as_ref() && *c==cylinder {
            true
        } else {
            false
        }
    }
}
