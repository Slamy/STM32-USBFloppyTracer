#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![feature(let_else)]

use crate::image_g64::parse_g64_image;
use crate::image_ipf::parse_ipf_image;
use crate::usb_commands::{wait_for_answer, wait_for_last_answer, write_raw_track};
use image_adf::parse_adf_image;
use image_d64::parse_d64_image;
use image_iso::parse_iso_image;
use pretty_hex::{HexConfig, PrettyHex};
use rawtrack::RawImage;
use rusb::{Context, DeviceHandle};
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::process::exit;
use std::{ffi::OsStr, path::Path};
use usb_commands::configure_device;
use usb_device::{clear_buffers, init_usb};
use util::DriveSelectState;
use write_precompensation::{write_precompensation_calibration, WritePrecompDb};

pub mod image_adf;
pub mod image_d64;
pub mod image_g64;
pub mod image_ipf;
pub mod image_iso;
pub mod rawtrack;
pub mod usb_commands;
pub mod usb_device;
pub mod write_precompensation;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, about, long_about = None)]
struct Args {
    /// Path to disk image
    filepath: String,

    /// Write raw track data to file. No USB communication
    #[arg(short)]
    debug_text_file: Option<String>,

    /// Use drive A
    #[arg(short, default_value_t = false)]
    a_drive: bool,

    /// Use drive B
    #[arg(short, default_value_t = false)]
    b_drive: bool,

    /// Use provided image to test write precompensation values
    #[arg(short, default_value_t = false)]
    wprecomp_calib: bool,

    /// Simulate index signal for flipped 5.25" disks
    #[arg(short, long, default_value_t = false)]
    flippy: bool,
}

fn parse_image(path: &str) -> RawImage {
    let extension = Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .expect("Unknown file extension!");

    match extension {
        "ipf" => parse_ipf_image(path),
        "adf" => parse_adf_image(path),
        "d64" => parse_d64_image(path),
        "g64" => parse_g64_image(path),
        "st" => parse_iso_image(path),
        _ => panic!("{} is an unknown file extension!", extension),
    }
}

fn write_and_verify_image(usb_handles: &(DeviceHandle<Context>, u8, u8), image: RawImage) {
    let mut write_iterator = image.tracks.iter();
    let mut verify_iterator = image.tracks.iter();

    while let Some(write_track) = write_iterator.next() {
        write_raw_track(&usb_handles, write_track);
        wait_for_answer(&usb_handles, &mut verify_iterator);
    }

    println!("All tracks written. Wait for remaining verifications!");

    while let Some(verify_track) = verify_iterator.next() {
        wait_for_last_answer(&usb_handles, &verify_track);
    }

    println!("--- Disk Image written and verified! ---")
}

fn write_debug_text_file(path: &str, image: RawImage) {
    let f = File::create(path).expect("Unable to create file");
    let mut f = BufWriter::new(f);

    let cfg = HexConfig {
        title: true,
        ascii: false,
        width: 16,
        group: 0,
        chunk: 1,
        ..HexConfig::default()
    };

    let mut context = md5::Context::new();

    for track in image.tracks.iter() {
        context.consume(u32::to_le_bytes(track.cylinder));
        context.consume(u32::to_le_bytes(track.head));
        track.densitymap.iter().for_each(|g| {
            context.consume(i32::to_le_bytes(g.cell_size.0 as i32));
            context.consume(usize::to_le_bytes(g.number_of_cells));
        });
        context.consume(&track.raw_data);

        f.write_all(
            format!(
                "Cylinder {} Head {} Encoding {:?}\n",
                track.cylinder, track.head, track.encoding
            )
            .as_bytes(),
        )
        .unwrap();

        track.densitymap.iter().for_each(|g| {
            f.write_all(
                format!(
                    "For {} cells use density {}\n",
                    g.number_of_cells, g.cell_size.0
                )
                .as_bytes(),
            )
            .unwrap();
        });

        f.write_all(format!("{:?}\n", track.raw_data.hex_conf(cfg)).as_bytes())
            .unwrap();
    }

    let md5_hash = context.compute();
    let md5_hashstr = format!("{:x}", md5_hash);
    println!("MD5 for unit test: {}", md5_hashstr);
}

fn main() {
    let cli = Args::parse();

    let wprecomp_db = WritePrecompDb::new();

    // before the make contact to the USB device, we shall read the image first
    // to be sure that it is writeable.
    let mut image = parse_image(&cli.filepath);

    for track in image.tracks.iter() {
        track.check_writability();
    }

    let mut already_warned_about_wprecomp_fail = false;
    for track in image.tracks.iter_mut() {
        track.get_significance_offset();

        // only alter the write precompensation if no calibration is performed!
        if let Some(wprecomp_db) = &wprecomp_db && !cli.wprecomp_calib {
            track.write_precompensation = wprecomp_db.calculate_write_precompensation(
                track.densitymap[0].cell_size.0 as u32,
                track.cylinder,
            ).unwrap_or_else(||{
                if !already_warned_about_wprecomp_fail{
                    already_warned_about_wprecomp_fail=true;
                    println!("Unable to calculate write precompensation for cylinder {} and density {}",track.cylinder,track.densitymap[0].cell_size.0 );
                }
                0
            });
        }
    }

    if let Some(debug_text_file) = cli.debug_text_file {
        write_debug_text_file(&debug_text_file, image);
        exit(0);
    }

    // connect to USB
    let usb_handles = init_usb().unwrap_or_else(|| {
        println!("Unable to initialize the USB device!");
        exit(1);
    });

    // it might be sometimes possible during an abort, that the endpoint
    // still contains data. Must be removed before proceeding
    clear_buffers(&usb_handles);

    if cli.a_drive && cli.b_drive {
        panic!("Specify either drive A or B. NOT BOTH!");
    }

    let select_drive = if cli.a_drive {
        DriveSelectState::A
    } else if cli.b_drive {
        DriveSelectState::B
    } else {
        panic!("No drive selected! Please specifiy with -a or -b");
    };

    configure_device(&usb_handles, select_drive, image.density);

    if cli.wprecomp_calib {
        write_precompensation_calibration(&usb_handles, image);
    } else {
        write_and_verify_image(&usb_handles, image);
    }
}

fn md5_sum_of_file(path: &str) -> String {
    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = f.read(whole_file_buffer.as_mut()).unwrap();
    assert_eq!(bytes_read, metadata.len() as usize);
    let file_hash = md5::compute(&whole_file_buffer);
    let file_hashstr = format!("{:x}", file_hash);
    file_hashstr
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        "../images/turrican.adf",
        "6677ce6cea38dc66be40e9211576a149",
        "b9167a41464460a0b4ebd8ddccd38f74"
    )]
    #[case(
        "../images/Turrican.ipf",
        "654e52bec1555ab3802c21f6ea269e64",
        "d5b13e4fc464924321f92a5e76ac855a"
    )]
    #[case(
        "../images/Turrican2.ipf",
        "17abf9d8d5b2af451897f6db8c7f4868",
        "d952bb8fa136be25906f8f0ebd2b9ef8"
    )]
    #[case(
        "../images/Katakis_(CPX).d64",
        "a1a64b89c44d9c778b2677b0027e015e",
        "ace751801193ce5d8ff689c2e1eac003"
    )]
    #[case(
        "../images/Katakis (Side 1).g64",
        "53c47c575d057181a1911e6653229324",
        "3b031710072ec07d39120c9d57f8ff50"
    )]
    fn known_image_regression_test(
        #[case] filepath: &str,
        #[case] expected_file_md5: &str,
        #[case] expected_md5: &str,
    ) {
        // before we start, we must be sure that this is really the file we want to process
        assert_eq!(md5_sum_of_file(filepath), expected_file_md5);

        let mut image = parse_image(filepath);

        let mut context = md5::Context::new();

        for track in image.tracks.iter_mut() {
            context.consume(u32::to_le_bytes(track.cylinder));
            context.consume(u32::to_le_bytes(track.head));
            track.densitymap.iter().for_each(|g| {
                context.consume(i32::to_le_bytes(g.cell_size.0 as i32));
                context.consume(usize::to_le_bytes(g.number_of_cells));
            });
            context.consume(&track.raw_data);
        }

        let md5_hash = context.compute();
        let md5_hashstr = format!("{:x}", md5_hash);
        println!("{}", md5_hashstr);
        assert_eq!(md5_hashstr, expected_md5);
    }
}
