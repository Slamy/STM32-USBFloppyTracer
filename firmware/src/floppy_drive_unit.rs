//! Handling of all slow signals of a single floppy disk drive

use core::convert::Infallible;

use alloc::boxed::Box;
use stm32f4xx_hal::hal::digital::v2::{OutputPin, StatefulOutputPin};
use unwrap_infallible::UnwrapInfallible;

enum MotorState {
    Off,
    On(u32),
}

/// The drives head position is either unknown or known at a certain position
pub enum HeadPosition {
    /// The head might be anywhere. State after bootup
    Unknown,
    /// The head position is known. Usually the TRK00 signal is required for conformation
    Cylinder(u32),
}

/// Collects information for controlling a single floppy drive
pub struct FloppyDriveUnit {
    out_motor_enable: Box<dyn OutputPin<Error = Infallible> + Send>,
    out_drive_select: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
    motor_state: MotorState,
    head_position: Option<HeadPosition>,
}

impl FloppyDriveUnit {
    #[must_use]
    /// Construct an instance with required GPIO injected
    pub fn new(
        out_motor_enable: Box<dyn OutputPin<Error = Infallible> + Send>,
        out_drive_select: Box<dyn StatefulOutputPin<Error = Infallible> + Send>,
    ) -> Self {
        Self {
            out_motor_enable,
            out_drive_select,
            motor_state: MotorState::Off,
            head_position: Some(HeadPosition::Unknown),
        }
    }

    /// Expected to be called about every millisecond
    /// Stops the motor after some time
    pub fn run(&mut self) {
        if let MotorState::On(count) = self.motor_state {
            if count > 0 {
                self.motor_state = MotorState::On(count - 1);
            } else {
                self.stop_motor();
            }
        }
    }

    /// Activate the motor and selects the drive
    pub fn spin_motor(&mut self) {
        self.out_motor_enable.set_low().unwrap_infallible();
        self.out_drive_select.set_low().unwrap_infallible();
        self.motor_state = MotorState::On(600);
    }

    /// Deselects drive if no stepping operation is currently performed
    pub fn disable_select_signal_if_possible(&mut self) {
        if matches!(self.motor_state, MotorState::Off) && self.head_position.is_some() {
            self.out_drive_select.set_high().unwrap_infallible();
        }
    }

    #[must_use]
    /// Returns `true` if the drive is currently selected for operation
    pub fn selection_signal_active(&self) -> bool {
        self.out_drive_select.is_set_low().unwrap_infallible()
    }

    /// Stops the motor and deselects drive
    pub fn stop_motor(&mut self) {
        self.out_motor_enable.set_high().unwrap_infallible();
        self.motor_state = MotorState::Off;
        self.disable_select_signal_if_possible();
    }

    #[must_use]
    /// Returns `true` if the drive motor is active
    pub fn is_spinning(&self) -> bool {
        matches!(self.motor_state, MotorState::On(_))
    }

    /// Extracts the current head position for outside mechanisms
    pub fn take_head_position_for_stepping(&mut self) -> HeadPosition {
        let taken = self.head_position.take().expect("Program flow error");
        self.out_drive_select.set_low().unwrap_infallible();
        taken
    }

    /// Stores the current head position
    pub fn insert_current_head_position(&mut self, pos: HeadPosition) {
        let old = self.head_position.replace(pos);
        assert!(old.is_none(), "Program flow error");
        self.disable_select_signal_if_possible();
    }

    /// Returns `true` if the head is positioned on the provided `cylinder`
    pub fn head_position_equals(&mut self, cylinder: u32) -> bool {
        if let Some(HeadPosition::Cylinder(c)) = self.head_position.as_ref() && *c==cylinder {
            true
        } else {
            false
        }
    }
}
