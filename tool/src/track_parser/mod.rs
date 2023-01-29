use std::{ffi::OsStr, fs::File, io::Write, path::Path};

use rusb::{Context, DeviceHandle};

use crate::{
    rawtrack::TrackFilter,
    track_parser::{amiga::AmigaTrackParser, c64::C64TrackParser, iso::IsoTrackParser},
    usb_commands::read_raw_track,
};

pub mod amiga;
pub mod c64;
pub mod iso;

pub struct TrackPayload {
    pub cylinder: u32,
    pub head: u32,
    pub payload: Vec<u8>,
}

pub struct CollectedSector {
    index: u32,
    payload: Vec<u8>,
}

pub trait TrackParser {
    fn parse_raw_track(&mut self, track: &[u8]) -> anyhow::Result<TrackPayload>;
    fn expect_track(&mut self, cylinder: u32, head: u32);
    fn step_size(&self) -> usize;
}

fn concatenate_sectors(
    mut collected_sectors: Vec<CollectedSector>,
    cylinder: u32,
    head: u32,
) -> TrackPayload {
    // Put the sectors in the right order before concatenating their data together
    collected_sectors.sort_by_key(|f| f.index);

    let mut track_data = Vec::with_capacity(collected_sectors.len() * 512);

    collected_sectors
        .iter_mut()
        .for_each(|f| track_data.append(&mut f.payload));

    TrackPayload {
        cylinder,
        head,
        payload: track_data,
    }
}

pub fn read_tracks_to_diskimage(
    usb_handles: &(DeviceHandle<Context>, u8, u8),
    track_filter: &TrackFilter,
    filepath: &str,
) {
    let file_extension = Path::new(filepath)
        .extension()
        .and_then(OsStr::to_str)
        .expect("No file extension!");

    let mut track_parse: Box<dyn TrackParser> = match file_extension {
        "adf" => Box::new(AmigaTrackParser::new(util::Density::SingleDouble)),
        "d64" => Box::new(C64TrackParser::new()),
        "st" => Box::new(IsoTrackParser::new(9)),
        "img" => Box::new(IsoTrackParser::new(15)),
        _ => panic!("{} is an unknown file extension!", file_extension),
    };

    let mut cylinder_begin = track_filter.cyl_start.unwrap_or(0);
    let mut cylinder_end = track_filter
        .cyl_end
        .expect("Please specify the last cylinder to read!");

    if cylinder_begin == cylinder_end {
        cylinder_begin = 0;
    } else {
        cylinder_end += 1;
    }

    let heads = match track_filter.head {
        Some(0) => 0..1,
        Some(1) => 1..2,
        None => 0..2,
        _ => panic!("Program flow error!"),
    };

    println!("Reading cylinders {} to {}", cylinder_begin, cylinder_end);
    let mut outfile = File::create(filepath).expect("Unable to create file");

    for cylinder in (cylinder_begin..cylinder_end).step_by(track_parse.step_size()) {
        for head in heads.clone() {
            track_parse.expect_track(cylinder, head);

            let mut possible_track: Option<TrackPayload> = None;

            for _ in 0..5 {
                let raw_data = read_raw_track(&usb_handles, cylinder, head, false);
                let track = track_parse.parse_raw_track(&raw_data).ok();

                if track.is_some() {
                    possible_track = track;
                    break;
                }

                println!(
                    "Reading of track {} {} not successful. Try again...",
                    cylinder, head
                )
            }

            let track =
                possible_track.expect(&format!("Unable to read track {} {}", cylinder, head));

            assert_eq!(cylinder, track.cylinder);
            assert_eq!(head, track.head);

            outfile.write_all(&track.payload).unwrap();
        }
    }
}
