//! Handling of all slow signals on the floppy bus

use core::{convert::Infallible, future::Future, pin::Pin};

use alloc::boxed::Box;
use cassette::Cassette;
use rtt_target::rprintln;
use stm32f4xx_hal::{
    gpio::PinState,
    hal::digital::v2::{InputPin, OutputPin},
};
use unwrap_infallible::UnwrapInfallible;
use util::{Density, DriveSelectState, Track};

use crate::{
    floppy_drive_unit::{FloppyDriveUnit, HeadPosition},
    floppy_stepper::FloppyStepperSignals,
};

type FutureHeadPosition =
    Cassette<Pin<Box<dyn Future<Output = (FloppyStepperSignals, HeadPosition)> + Send>>>;

/// Manages all slow signals of the floppy bus
/// Examples are the selection of the head or the timing of the stepper motor
pub struct FloppyControl {
    out_head_select: Box<dyn OutputPin<Error = Infallible> + Send>,
    out_density_select: Box<dyn OutputPin<Error = Infallible> + Send>,
    in_write_protect: Box<dyn InputPin<Error = Infallible> + Send>,
    floppy_step_signals: Option<FloppyStepperSignals>,
    floppy_step_progress: Option<FutureHeadPosition>,
    drive_a: FloppyDriveUnit,
    drive_b: FloppyDriveUnit,
    drive_select: DriveSelectState,
}

impl FloppyControl {
    #[must_use]
    /// Construct an instance with required GPIO injected
    pub fn new(
        drive_a: FloppyDriveUnit,
        drive_b: FloppyDriveUnit,
        stepper: FloppyStepperSignals,
        out_head_select: Box<dyn OutputPin<Error = Infallible> + Send>,
        out_density_select: Box<dyn OutputPin<Error = Infallible> + Send>,
        in_write_protect: Box<dyn InputPin<Error = Infallible> + Send>,
    ) -> Self {
        Self {
            drive_a,
            drive_b,
            floppy_step_signals: Some(stepper),
            floppy_step_progress: None,
            drive_select: DriveSelectState::None,
            out_head_select,
            out_density_select,
            in_write_protect,
        }
    }

    /// Selects High our double densitiy
    pub fn select_density(&mut self, dens: Density) {
        match dens {
            Density::High => {
                self.out_density_select.set_high().unwrap_infallible();
                rprintln!("High Density selected!");
            }
            Density::SingleDouble => {
                self.out_density_select.set_low().unwrap_infallible();
                rprintln!("Double Density selected!");
            }
        }
    }

    /// Returns `true` in case the disk can't be written because of the write protection
    pub fn write_protection_is_active(&mut self) -> bool {
        assert!(self
            .selected_drive_unit()
            .expect("Drive not selected")
            .selection_signal_active());
        self.in_write_protect.is_low().unwrap_infallible()
    }

    /// Activate the motor of the currently selected drive
    pub fn spin_motor(&mut self) {
        if let Some(f) = self.selected_drive_unit().as_mut() {
            f.spin_motor()
        }
    }

    #[must_use]
    /// Returns `true` if the currently selected drives motor is active
    pub fn is_spinning(&self) -> bool {
        self.selected_drive_unit_ref()
            .as_ref()
            .map_or(false, |f| f.is_spinning())
    }

    /// Returns the currently selected drive or `None`
    pub fn selected_drive_unit(&mut self) -> Option<&mut FloppyDriveUnit> {
        match self.drive_select {
            DriveSelectState::None => None,
            DriveSelectState::A => Some(&mut self.drive_a),
            DriveSelectState::B => Some(&mut self.drive_b),
        }
    }

    #[must_use]
    /// Returns the currently selected drive or `None`
    pub fn selected_drive_unit_ref(&self) -> Option<&FloppyDriveUnit> {
        match self.drive_select {
            DriveSelectState::None => None,
            DriveSelectState::A => Some(&self.drive_a),
            DriveSelectState::B => Some(&self.drive_b),
        }
    }

    /// Stops the motor of the currently selected drive
    pub fn stop_motor(&mut self) {
        if let Some(f) = self.selected_drive_unit() {
            f.stop_motor()
        }
    }

    /// Select a disk drive
    pub fn select_drive(&mut self, state: DriveSelectState) {
        self.drive_select = state;
    }

    /// Starts an asynchronous process to step to the provided cylinder and also select the head
    /// to provide access to the wanted `track`.
    /// Use `reached_selected_cylinder` to check for completion
    pub fn select_track(&mut self, track: Track) {
        let selected_drive = self.selected_drive_unit().expect("Drive not selected!");

        let wanted_cylinder = u32::from(track.cylinder.0);
        if !selected_drive.head_position_equals(wanted_cylinder) {
            let current_head_position = selected_drive.take_head_position_for_stepping();
            let func = Box::pin(
                self.floppy_step_signals
                    .take()
                    .expect("Program flow error")
                    .step_to_cylinder(current_head_position, u32::from(track.cylinder.0)),
            );

            self.floppy_step_progress = Some(Cassette::new(func));
        }

        self.out_head_select
            .set_state(if track.head.0 == 0 {
                PinState::High
            } else {
                PinState::Low
            })
            .unwrap_infallible();
    }

    #[must_use]
    /// Returns `true` if the stepping process has finished
    pub fn reached_selected_cylinder(&self) -> bool {
        self.floppy_step_progress.is_none()
    }

    /// Expected to be called about every millisecond
    /// Handles motor activation and stepping.
    pub fn run(&mut self) {
        self.drive_a.run();
        self.drive_b.run();

        if let Some(cm) = self.floppy_step_progress.as_mut() {
            if let Some(result) = cm.poll_on() {
                let old = self.floppy_step_signals.replace(result.0);
                assert!(old.is_none(), "Program flow error");
                self.selected_drive_unit()
                    .expect("Drive not selected")
                    .insert_current_head_position(result.1);

                self.floppy_step_progress = None;
            }
        }
    }
}
