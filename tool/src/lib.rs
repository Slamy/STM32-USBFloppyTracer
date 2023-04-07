#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![feature(let_chains)]

macro_rules! program_flow_error {
    () => {
        format!("Program flow error at {}:{}", file!(), line!())
    };
}

macro_rules! index_out_of_bounds {
    () => {
        format!("Slice Index out of bounds at {}:{}", file!(), line!())
    };
}

macro_rules! ensure_index {
    ($a:ident [ $b:expr ]) => {
        *$a.get($b).with_context(|| index_out_of_bounds!())?
    };
}

macro_rules! ensure_index_mut {
    ($a:ident [ $b:expr ]) => {
        *$a.get_mut($b).with_context(|| index_out_of_bounds!())?
    };
}

pub mod image_reader;
pub mod track_parser;

pub mod rawtrack;
pub mod usb_commands;
pub mod usb_device;
pub mod write_precompensation;
