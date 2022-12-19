use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead},
    time::Duration,
};

use rusb::{Context, DeviceHandle};
use util::Density;

use crate::RawImage;
use crate::{rawtrack::RawTrack, write_raw_track};

pub fn write_precompensation_calibration(
    usb_handles: &(DeviceHandle<Context>, u8, u8),
    mut image: RawImage,
) {
    println!("tracks len {}", image.tracks.len());
    println!("Disk Type {:?} {:?}", image.density, image.disk_type);

    // especially around 40 it is interesting as most drives activate the internal write precompensation
    // we want to filter especially that out here
    let cylinders_to_calibrate = vec![0, 10, 20, 30, 39, 40, 41, 42, 43, 44, 50, 60, 70, 75, 79];

    let maximum_write_precompensation = match (image.density, image.disk_type) {
        (Density::High, util::DiskType::Inch3_5) => 12,
        (Density::SingleDouble, util::DiskType::Inch3_5) => 22,
        (Density::SingleDouble, util::DiskType::Inch5_25) => 14,
        (_, _) => panic!("Unsupported for write precompensation!"),
    };

    let mut results: HashMap<usize, Vec<usize>> = HashMap::new();

    let process_answer = |results2: &mut HashMap<usize, Vec<usize>>, last: bool| {
        let timeout = Duration::from_secs(10);

        loop {
            let mut in_buf = [0u8; 64];

            let size = usb_handles
                .0
                .read_bulk(usb_handles.1, &mut in_buf, timeout)
                .unwrap();

            let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
            let response_split: Vec<&str> = response_text.split(" ").collect();

            match response_split[0] {
                "WrittenAndVerified" => {
                    println!(
                        "Verified write of track {} head {} - num_writes:{}, num_reads:{}, max_err:{} write_precomp:{}",
                        response_split[1],
                        response_split[2],
                        response_split[3],
                        response_split[4],
                        response_split[5],
                        response_split[6],
                    );

                    let track: usize = response_split[1].parse().unwrap();
                    let max_err: usize = response_split[5].parse().unwrap();

                    results2.get_mut(&track).unwrap().push(max_err);

                    if last {
                        break;
                    }
                }
                "GotCmd" => break, // Continue with next track!
                "Fail" => {
                    println!(
                        "Failed writing track {} head {} - num_writes:{}, num_reads:{}",
                        response_split[1], response_split[2], response_split[3], response_split[4],
                    );

                    let track: usize = response_split[1].parse().unwrap();
                    results2.get_mut(&track).unwrap().push(55);

                    if last {
                        break;
                    }
                }
                "WriteProtected" => panic!("Disk is write protected!"),
                _ => panic!("Unexpected answer from device: {}", response_text),
            }
        }
    };

    for forced_cylinder in cylinders_to_calibrate {
        let possible_track = image
            .tracks
            .iter_mut()
            .find(|f| f.cylinder == forced_cylinder);

        let mut track: &mut RawTrack = if possible_track.is_some() {
            possible_track.unwrap()
        } else {
            println!("Just use the last track...");
            image.tracks.last_mut().unwrap()
        };

        track.cylinder = forced_cylinder;
        results.insert(track.cylinder as usize, Vec::new());

        for write_precomp in (0..maximum_write_precompensation).step_by(1) {
            track.write_precompensation = write_precomp;
            write_raw_track(&usb_handles, track);

            process_answer(&mut results, false);
        }
    }
    // get last answer
    process_answer(&mut results, true);

    println!("{:?}", results);

    let mut csv_wtr = csv::Writer::from_path("wprecomp.csv").unwrap();

    // make header
    csv_wtr.write_field("").unwrap();
    for write_precomp in (0..maximum_write_precompensation).step_by(1) {
        csv_wtr.write_field(write_precomp.to_string()).unwrap();
    }
    csv_wtr.write_record(None::<&[u8]>).unwrap();

    // Data Rows
    let mut results: Vec<_> = results.iter().collect();
    results.sort_by_key(|f| f.0);

    for (track, entries) in results {
        csv_wtr.write_field(track.to_string()).unwrap();
        csv_wtr
            .write_record(entries.iter().map(|f| f.to_string()))
            .unwrap();
    }

    csv_wtr.flush().unwrap();
}

// vector of tuples of cellsize, track, wprecomp
#[derive(PartialEq, PartialOrd, Eq, Ord, Debug)]
struct Sample {
    cellsize: u32,
    cylinder: u32,
    wprecomp: u32,
}

pub struct WritePrecompDb {
    samples: Vec<Sample>,
}

impl WritePrecompDb {
    pub fn new() -> Option<Self> {
        let mut samples = Vec::new();

        let x = home::home_dir()
            .unwrap()
            .join(".usbfloppytracer/wprecomp.cfg");

        println!("Reading config from {:?}", x);
        let file = File::open(x)
            .or_else(|f| {
                println!("Write precompensation not used... {}", f.to_string());
                Err(f)
            })
            .ok()?;

        let lines = io::BufReader::new(file).lines();

        for line in lines {
            if let Ok(line) = line {
                let number_parts: Vec<u32> = line
                    .split_ascii_whitespace()
                    .filter_map(|d| d.parse().ok())
                    .collect();

                if number_parts.len() == 3 {
                    let cellsize = number_parts[0];
                    let cylinder = number_parts[1];
                    let wprecomp = number_parts[2];

                    samples.push(Sample {
                        cellsize,
                        cylinder,
                        wprecomp,
                    });
                }
            }
        }

        samples.sort();

        Some(WritePrecompDb { samples })
    }

    fn lerp_left(&self, cellsize: u32, cylinder: u32) -> Option<(f32, u32)> {
        let left_top_sample = self
            .samples
            .iter()
            .filter(|f| f.cellsize <= cellsize && f.cylinder <= cylinder)
            .last()?;

        let Some(left_bottom_sample) = self
            .samples
            .iter()
            .filter(|f| f.cellsize == left_top_sample.cellsize && f.cylinder >= cylinder)
            .next()
            else {
                return Some((left_top_sample.wprecomp as f32, left_top_sample.cellsize));
            };

        if left_bottom_sample.cylinder == left_top_sample.cylinder {
            return Some((left_top_sample.wprecomp as f32, left_top_sample.cellsize));
        }

        let left_track_factor = (cylinder - left_top_sample.cylinder) as f32
            / (left_bottom_sample.cylinder - left_top_sample.cylinder) as f32;

        let left_result = (1.0 - left_track_factor) * left_top_sample.wprecomp as f32
            + left_track_factor * left_bottom_sample.wprecomp as f32;

        Some((left_result, left_top_sample.cellsize))
    }

    fn lerp_right(&self, cellsize: u32, cylinder: u32) -> (f32, u32) {
        let Some(right_bottom_sample) = self
            .samples
            .iter()
            .filter(|f| f.cellsize >= cellsize && f.cylinder >= cylinder)
            .next()
            else {
                let last_sample = self.samples.last().unwrap();
                return (last_sample.wprecomp as f32, last_sample.cellsize);
            };

        let right_top_sample = self
            .samples
            .iter()
            .filter(|f| f.cellsize == right_bottom_sample.cellsize && f.cylinder <= cylinder)
            .last()
            .unwrap();

        if right_bottom_sample.cylinder == right_top_sample.cylinder {
            return (right_top_sample.wprecomp as f32, right_top_sample.cellsize);
        }

        let right_track_factor = (cylinder - right_top_sample.cylinder) as f32
            / (right_bottom_sample.cylinder - right_top_sample.cylinder) as f32;
        let right_result = (1.0 - right_track_factor) * right_top_sample.wprecomp as f32
            + right_track_factor * right_bottom_sample.wprecomp as f32;
        (right_result, right_bottom_sample.cellsize)
    }

    pub fn calculate_write_precompensation(&self, cellsize: u32, cylinder: u32) -> Option<u32> {
        // cell sizes are left to right, so the x axis
        // cylinders are top to bottom, so the y axis
        let (left_result, left_cellsize) = self.lerp_left(cellsize, cylinder)?;
        let (right_result, right_cellsize) = self.lerp_right(cellsize, cylinder);

        if left_cellsize == right_cellsize {
            return Some(left_result.round() as u32);
        }

        let cellsize_factor =
            (cellsize - left_cellsize) as f32 / (right_cellsize - left_cellsize) as f32;

        Some(
            ((1.0 - cellsize_factor) * left_result + cellsize_factor * right_result).round() as u32,
        )
    }
}
