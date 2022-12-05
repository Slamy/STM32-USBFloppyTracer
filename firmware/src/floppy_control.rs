use stm32f4xx_hal::gpio::{Output, Pin, PinState};
use util::{Density, DriveSelectState, Track};

use crate::safeiprintln;
#[derive(Clone, Copy, Debug)]
enum StepState {
    Idle,
    SettingDirection(u8),
    Stepping,
    SettlingHead(u8),
}

enum MotorState {
    Off,
    On(u32),
}

pub struct FloppyControl {
    out_motor_enable_a: Pin<'A', 8, Output>,
    out_drive_select_b: Pin<'A', 15, Output>,
    out_drive_select_a: Pin<'B', 0, Output>,
    out_motor_enable_b: Pin<'B', 1, Output>,
    out_step_direction: Pin<'B', 2, Output>,
    out_step_perform: Pin<'B', 4, Output>,
    in_track_00: Pin<'B', 7>,
    out_head_select: Pin<'B', 11, Output>,
    out_density_select: Pin<'B', 13, Output>,

    current_cylinder: Option<i32>,
    wanted_cylinder: i32,
    step_state: StepState,
    motor_state: MotorState,
    drive_select: DriveSelectState,
}

impl FloppyControl {
    pub fn new(
        out_motor_enable_a: Pin<'A', 8, Output>,
        out_drive_select_b: Pin<'A', 15, Output>,
        out_drive_select_a: Pin<'B', 0, Output>,
        out_motor_enable_b: Pin<'B', 1, Output>,
        out_step_direction: Pin<'B', 2, Output>,
        out_step_perform: Pin<'B', 4, Output>,
        in_track_00: Pin<'B', 7>,
        out_head_select: Pin<'B', 11, Output>,
        out_density_select: Pin<'B', 13, Output>,
    ) -> FloppyControl {
        FloppyControl {
            out_motor_enable_a,
            out_drive_select_b,
            out_drive_select_a,
            out_motor_enable_b,
            out_step_direction,
            out_step_perform,
            in_track_00,
            out_head_select,
            out_density_select,
            current_cylinder: Some(0),
            wanted_cylinder: 0,
            step_state: StepState::Idle,
            motor_state: MotorState::Off,
            drive_select: DriveSelectState::None,
        }
    }

    pub fn select_density(&mut self, dens: Density) {
        match dens {
            Density::High => {
                self.out_density_select.set_high();
                safeiprintln!("High Density selected!")
            }
            Density::SingleDouble => {
                self.out_density_select.set_low();
                safeiprintln!("Double Density selected!")
            }
        }
    }

    pub fn spin_motor(&mut self) {
        match self.drive_select {
            DriveSelectState::None => {}
            DriveSelectState::A => {
                self.out_motor_enable_a.set_low();
                self.out_drive_select_a.set_low();
            }
            DriveSelectState::B => {
                self.out_drive_select_b.set_low();
                self.out_motor_enable_b.set_low();
            }
        }
        self.motor_state = MotorState::On(800);
    }

    pub fn is_spinning(&self) -> bool {
        matches!(self.motor_state, MotorState::On(_))
    }

    pub fn stop_motor(&mut self) {
        match self.drive_select {
            DriveSelectState::None => {}
            DriveSelectState::A => {
                self.out_motor_enable_a.set_high();
            }
            DriveSelectState::B => {
                self.out_motor_enable_b.set_high();
            }
        }
        self.motor_state = MotorState::Off;
    }

    pub fn select_drive(&mut self, state: DriveSelectState) {
        match state {
            DriveSelectState::None => {
                // stop everything.
                self.out_drive_select_a.set_high();
                self.out_motor_enable_a.set_high();

                self.out_drive_select_b.set_high();
                self.out_motor_enable_b.set_high();
            }
            DriveSelectState::A => {
                // stop all drive B activities
                self.out_drive_select_b.set_high();
                self.out_motor_enable_b.set_high();

                self.out_drive_select_a.set_low();
                safeiprintln!("Drive A selected!")
            }
            DriveSelectState::B => {
                // stop all drive A activites
                self.out_drive_select_a.set_high();
                self.out_motor_enable_a.set_high();

                self.out_drive_select_b.set_low();
                safeiprintln!("Drive B selected!")
            }
        }

        self.drive_select = state;

        self.out_step_direction.set_high();
        self.out_step_perform.set_high();
        self.out_head_select.set_high();

        // cylinder is unknown. require track 00 first.
        self.current_cylinder = None;
    }

    pub fn select_track(&mut self, track: Track) {
        self.wanted_cylinder = track.cylinder.0 as i32;
        self.out_head_select.set_state(if track.head.0 == 0 {
            PinState::High
        } else {
            PinState::Low
        });
    }

    pub fn get_current_cylinder(&self) -> i32 {
        self.current_cylinder.unwrap_or(-1)
    }

    pub fn reached_selected_cylinder(&self) -> bool {
        matches!(self.step_state, StepState::Idle)
            && self.wanted_cylinder == self.current_cylinder.unwrap_or(-1)
    }

    fn step_machine(&mut self) {
        self.step_state = match self.step_state {
            StepState::Idle => {
                if self.in_track_00.is_low() {
                    self.current_cylinder = Some(0);
                }

                if let Some(current_cylinder) = self.current_cylinder {
                    if current_cylinder < self.wanted_cylinder
                        && self.out_step_direction.is_set_high()
                    {
                        // direction is wrong. set direction and give it time to settle
                        self.out_step_direction.set_low();
                        StepState::SettingDirection(10)
                    } else if current_cylinder > self.wanted_cylinder
                        && self.out_step_direction.is_set_low()
                    {
                        // direction is wrong. set direction and give it time to settle
                        self.out_step_direction.set_high();
                        StepState::SettingDirection(10)
                    } else if current_cylinder != self.wanted_cylinder {
                        self.out_step_perform.set_low();

                        if current_cylinder < self.wanted_cylinder {
                            *self.current_cylinder.as_mut().unwrap() += 1;
                        } else {
                            *self.current_cylinder.as_mut().unwrap() -= 1;
                        }
                        StepState::Stepping
                    } else {
                        StepState::Idle
                    }
                } else {
                    // the current cylinder is not known. set the direction to outside and step
                    self.out_step_direction.set_high();
                    self.out_step_perform.set_low();
                    StepState::Stepping
                }
            }
            StepState::SettingDirection(cnt) => {
                if cnt > 0 {
                    StepState::SettingDirection(cnt - 1)
                } else {
                    StepState::Idle
                }
            }

            StepState::SettlingHead(cnt) => {
                if cnt > 0 {
                    StepState::SettlingHead(cnt - 1)
                } else {
                    StepState::Idle
                }
            }

            StepState::Stepping => {
                self.out_step_perform.set_high();

                // Is this the cylinder which we want? Then allow the head to settle before doing anything else.
                if let Some(current_cylinder) = self.current_cylinder && current_cylinder==self.wanted_cylinder {
            StepState::SettlingHead(10)
        }
        else
        {
            StepState::Idle
        }
            }
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

        self.step_machine();
    }
}
