#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![feature(let_else)]

use image_adf::parse_adf_image;
use image_d64::parse_d64_image;
use rawtrack::RawTrack;
use rusb::{Context, DeviceHandle};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::exit;
use std::slice::Iter;
use std::time::Duration;
use usb::init_usb;
use util::{Density, DriveSelectState, Encoding};

use crate::image_g64::parse_g64_image;
use crate::image_ipf::parse_ipf_image;

pub mod image_adf;
pub mod image_d64;
pub mod image_g64;
pub mod image_ipf;
pub mod rawtrack;
pub mod usb;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, about, long_about = None)]
struct Args {
    /// Path to disk image
    filepath: String,

    /// Use drive A
    #[arg(short, default_value_t = false)]
    a_drive: bool,

    /// Use drive B
    #[arg(short, default_value_t = false)]
    b_drive: bool,

    /// Simulate index signal for flipped 5 1/4" disks
    #[arg(short, long, default_value_t = false)]
    flippy: bool,
}

fn configure_device(
    handles: &(DeviceHandle<Context>, u8, u8),
    select_drive: DriveSelectState,
    density: Density,
) {
    let (handle, _endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 8];

    let mut writer = command_buf.chunks_mut(4);

    let mut settings = 0;

    if matches!(select_drive, DriveSelectState::B) {
        settings |= 1;
    }

    if matches!(density, Density::High) {
        settings |= 2;
    }

    writer
        .next()
        .unwrap()
        .clone_from_slice(&u32::to_le_bytes(0x12340002));

    writer
        .next()
        .unwrap()
        .clone_from_slice(&u32::to_le_bytes(settings));

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();
}

fn step_to_track(handles: &(DeviceHandle<Context>, u8, u8), cylinder: u32) {
    let (handle, _endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 8];

    let mut writer = command_buf.chunks_mut(4);

    writer
        .next()
        .unwrap()
        .clone_from_slice(&u32::to_le_bytes(0x12340003));

    writer
        .next()
        .unwrap()
        .clone_from_slice(&u32::to_le_bytes(cylinder));

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();
}

fn write_raw_track(handles: &(DeviceHandle<Context>, u8, u8), track: &RawTrack) {
    let (handle, _endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 64];

    let expected_size = track.raw_data.len();
    let mut remaining_blocks = expected_size / 64;
    if expected_size % 64 != 0 {
        remaining_blocks += 1;
    }

    println!(
        "Request write and verify of Cyl:{} Head:{}",
        track.cylinder, track.head
    );

    let mut writer = command_buf.chunks_mut(4);

    let header = vec![
        0x12340001,
        expected_size as u32,
        remaining_blocks as u32,
        track.cylinder | track.head << 16,
        track.first_significane_offset.unwrap() as u32,
        track.densitymap.len() as u32,
    ];

    for i in header {
        writer
            .next()
            .unwrap()
            .clone_from_slice(&u32::to_le_bytes(i));
    }

    for density_entry in track.densitymap.iter() {
        assert!(density_entry.cell_size.0 < 512);

        writer.next().unwrap().clone_from_slice(&u32::to_le_bytes(
            ((density_entry.number_of_cells as u32) << 9) | density_entry.cell_size.0 as u32,
        ));
    }

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();

    for block in track.raw_data.chunks(64) {
        handle.write_bulk(*endpoint_out, block, timeout).unwrap();
    }
}

fn wait_for_last_answer(handles: &(DeviceHandle<Context>, u8, u8), verify_track: &RawTrack) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .unwrap();

        let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
        let response_split: Vec<&str> = response_text.split(" ").collect();

        match response_split[0] {
            "WrittenAndVerified" => {
                println!(
                    "Verified write of track {} head {} - num_writes:{}, num_reads:{}",
                    response_split[1], response_split[2], response_split[3], response_split[4]
                );
                assert_eq!(verify_track.cylinder, response_split[1].parse().unwrap());
                assert_eq!(verify_track.head, response_split[2].parse().unwrap());
                break;
            }
            "Fail" => panic!(
                "Failed writing track {} head {} - num_writes:{}, num_reads:{}",
                response_split[1], response_split[2], response_split[3], response_split[4],
            ),
            _ => panic!("Unexpected answer from device: {}", response_text),
        }
    }
}

fn clear_buffers(handles: &(DeviceHandle<Context>, u8, u8)) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_millis(10);
    let mut in_buf = [0u8; 64];

    loop {
        let Ok(size) = handle.read_bulk(*endpoint_in, &mut in_buf, timeout) else {
            return;
        };
        println!("Cleared residual USB buffer of size {}", size);
    }
}

fn wait_for_answer(
    handles: &(DeviceHandle<Context>, u8, u8),
    verify_iterator: &mut Iter<RawTrack>,
) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .unwrap();

        let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
        let response_split: Vec<&str> = response_text.split(" ").collect();

        match response_split[0] {
            "WrittenAndVerified" => {
                println!(
                    "Verified write of track {} head {} - num_writes:{}, num_reads:{}",
                    response_split[1], response_split[2], response_split[3], response_split[4]
                );
                let expected_to_verify = verify_iterator.next().unwrap();
                assert_eq!(
                    expected_to_verify.cylinder,
                    response_split[1].parse().unwrap()
                );
                assert_eq!(expected_to_verify.head, response_split[2].parse().unwrap());
            }
            "GotCmd" => break, // Continue with next track!
            "Fail" => panic!(
                "Failed writing track {} head {} - num_writes:{}, num_reads:{}",
                response_split[1], response_split[2], response_split[3], response_split[4],
            ),
            "WriteProtected" => panic!("Disk is write protected!"),
            _ => panic!("Unexpected answer from device: {}", response_text),
        }
    }
}

fn parse_image(path: &str) -> Vec<RawTrack> {
    let extension = Path::new(path).extension().and_then(OsStr::to_str).unwrap();

    match extension {
        "ipf" => parse_ipf_image(path),
        "adf" => parse_adf_image(path),
        "d64" => parse_d64_image(path),
        "g64" => parse_g64_image(path),
        _ => panic!("{} is an unknown file extension!", extension),
    }
}

fn main2() {
    let usb_handles = init_usb().unwrap_or_else(|| {
        println!("Unable to initialize the USB device!");
        exit(1);
    });

    clear_buffers(&usb_handles);

    let paths = fs::read_dir("./images").unwrap();

    for path in paths {
        let p = path.unwrap().path();
        let mut tracks = parse_image(p.to_str().unwrap());

        for track in tracks.iter() {
            track.check_writability();
        }

        for track in tracks.iter_mut() {
            track.get_significance_offset();
        }

        if matches!(tracks[0].encoding, Encoding::GCR) {
            configure_device(&usb_handles, DriveSelectState::B, Density::SingleDouble);
        } else {
            configure_device(&usb_handles, DriveSelectState::A, Density::SingleDouble);
        }

        let mut write_iterator = tracks.iter();
        let mut verify_iterator = tracks.iter();

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
}

fn main() {
    let cli = Args::parse();

    let mut tracks = parse_image(&cli.filepath);

    for track in tracks.iter() {
        track.check_writability();
    }

    for track in tracks.iter_mut() {
        track.get_significance_offset();
    }

    let usb_handles = init_usb().unwrap_or_else(|| {
        println!("Unable to initialize the USB device!");
        exit(1);
    });

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

    let density = Density::SingleDouble; // TODO must be changeable!

    configure_device(&usb_handles, select_drive, density);

    let mut write_iterator = tracks.iter();
    let mut verify_iterator = tracks.iter();

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
