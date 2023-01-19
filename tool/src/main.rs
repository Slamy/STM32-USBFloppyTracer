#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![feature(let_else)]

use crate::usb_commands::{wait_for_answer, wait_for_last_answer, write_raw_track};
use image_reader::parse_image;
use pretty_hex::{HexConfig, PrettyHex};
use rawtrack::{RawImage, TrackFilter};
use rusb::{Context, DeviceHandle};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::process::exit;
use track_parser::read_tracks_to_diskimage;
use usb_commands::configure_device;
use usb_device::{clear_buffers, init_usb};
use util::{DriveSelectState, DRIVE_3_5_RPM, DRIVE_5_25_RPM};
use write_precompensation::{write_precompensation_calibration, WritePrecompDb};

pub mod image_reader;
pub mod track_parser;

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

    /// Read instead of write
    #[arg(short, default_value_t = false)]
    read: bool,

    /// Write raw track data to file. No USB communication
    #[arg(short)]
    debug_text_file: Option<String>,

    /// Only write some tracks: eg. range 2-4 or single track 8
    #[arg(short)]
    track_filter: Option<String>,

    /// Use drive A
    #[arg(short, default_value_t = false)]
    a_drive: bool,

    /// Use drive B
    #[arg(short, default_value_t = false)]
    b_drive: bool,

    /// Use provided image to test write precompensation values
    #[arg(short, default_value_t = false)]
    wprecomp_calib: bool,

    /// Simulate index signal for flipped 5.25" disks with provided timing offset
    #[arg(short, long)]
    flippy: Option<u32>,
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
            context.consume(i32::to_le_bytes(g.cell_size.0));
            context.consume(usize::to_le_bytes(g.number_of_cellbytes));
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

        if track.has_non_flux_reversal_area {
            f.write_all("Has Non Flux Reversal Area\n".as_bytes())
                .unwrap();
        }
        track.densitymap.iter().for_each(|g| {
            f.write_all(
                format!(
                    "For {} cells use density {}\n",
                    g.number_of_cellbytes, g.cell_size.0
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

    let image = if cli.read == false {
        let wprecomp_db = WritePrecompDb::new();

        // before the make contact to the USB device, we shall read the image first
        // to be sure that it is writeable.
        let mut image = parse_image(&cli.filepath);
        let rpm = match image.disk_type {
            util::DiskType::Inch3_5 => DRIVE_3_5_RPM,
            util::DiskType::Inch5_25 => DRIVE_5_25_RPM,
        };

        if let Some(filter) = cli.track_filter.as_ref() {
            let filter = TrackFilter::new(&filter);
            image.filter_tracks(filter);
        }

        for track in image.tracks.iter_mut() {
            track.get_significance_offset();
        }

        if let Some(debug_text_file) = cli.debug_text_file {
            write_debug_text_file(&debug_text_file, image);
            exit(0);
        }

        for track in image.tracks.iter() {
            track.assert_fits_into_rotation(rpm);
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
        Some(image)
    } else {
        None
    };

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

    let index_sim_frequency = if let Some(flippy_param) = cli.flippy {
        (14 * 1000 - flippy_param) * 1000
    } else {
        0
    };

    if cli.read {
        configure_device(
            &usb_handles,
            select_drive,
            util::Density::SingleDouble,
            index_sim_frequency,
        );

        let track_filter = cli
            .track_filter
            .as_ref()
            .expect("Please specify tracks used for reading!");

        let track_filter = TrackFilter::new(track_filter);

        read_tracks_to_diskimage(&usb_handles, &track_filter, &cli.filepath);
    } else {
        let image = image.unwrap();

        configure_device(
            &usb_handles,
            select_drive,
            image.density,
            index_sim_frequency,
        );

        if cli.wprecomp_calib {
            write_precompensation_calibration(&usb_handles, image);
        } else {
            write_and_verify_image(&usb_handles, image);
        }
    }
}
