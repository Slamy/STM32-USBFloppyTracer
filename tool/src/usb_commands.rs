use std::{slice::Iter, time::Duration};

use rusb::{Context, DeviceHandle};
use util::{Density, DriveSelectState};

use crate::rawtrack::RawTrack;

pub fn configure_device(
    handles: &(DeviceHandle<Context>, u8, u8),
    select_drive: DriveSelectState,
    density: Density,
    index_sim_frequency: u32,
) {
    let (handle, _endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 3 * 4];

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

    writer
        .next()
        .unwrap()
        .clone_from_slice(&u32::to_le_bytes(index_sim_frequency));

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();
}

pub fn read_raw_track(
    handles: &(DeviceHandle<Context>, u8, u8),
    cylinder: u32,
    head: u32,
    wait_for_index: bool,
    duration_to_record: usize,
) -> Vec<u8> {
    let (handle, endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    println!("Read raw track from Cyl:{} Head:{}", cylinder, head);

    let mut command_buf = [0u8; 64];
    let mut writer = command_buf.chunks_mut(4);

    let wait_for_index = if wait_for_index { 1 << 9 } else { 0 };

    let header = vec![
        0x12340004,
        cylinder | (head << 8) | wait_for_index,
        duration_to_record as u32,
    ];

    for word in header {
        writer
            .next()
            .unwrap()
            .clone_from_slice(&u32::to_le_bytes(word));
    }

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();

    let mut result = Vec::with_capacity(800 * 64); // TODO magic number

    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .unwrap();

        if size == 64 {
            result.extend_from_slice(&in_buf);
        } else if size == 0 {
            // End sign
            break;
        } else {
            let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
            panic!("{}", response_text);
        }
    }

    if result.len() == 64 {
        println!("{:?}", result);
    }
    result
}

pub fn write_raw_track(handles: &(DeviceHandle<Context>, u8, u8), track: &RawTrack) {
    let (handle, _endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 64];

    let expected_size = track.raw_data.len();
    let mut remaining_blocks = expected_size / 64;
    if expected_size % 64 != 0 {
        remaining_blocks += 1;
    }

    println!(
        "Request write and verify of Cyl:{} Head:{} WritePrecomp:{}",
        track.cylinder, track.head, track.write_precompensation
    );

    let mut writer = command_buf.chunks_mut(4);

    assert!(track.head <= 1);
    assert!(track.cylinder <= 0xff);
    assert!(track.write_precompensation <= 0xff);

    let non_flux_reversal_mask = if track.has_non_flux_reversal_area {
        0x200
    } else {
        0
    };

    let header = vec![
        0x12340001,
        expected_size as u32,
        remaining_blocks as u32,
        // Fields 00000000 PPPPPPPP 000000NH CCCCCCCC
        track.cylinder
            | (track.head << 8)
            | non_flux_reversal_mask
            | (track.write_precompensation << 16),
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
            ((density_entry.number_of_cellbytes as u32) << 9) | density_entry.cell_size.0 as u32,
        ));
    }

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .unwrap();

    for block in track.raw_data.chunks(64) {
        handle.write_bulk(*endpoint_out, block, timeout).unwrap();
    }
}

pub fn wait_for_last_answer(handles: &(DeviceHandle<Context>, u8, u8), verify_track: &RawTrack) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    // TODO copy pasta
    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .unwrap();

        let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
        let response_split: Vec<&str> = response_text.split(' ').collect();

        match response_split[0] {
            "WrittenAndVerified" => {
                println!(
                    "Verified write of cylinder {} head {} - writes:{}, reads:{}, max_err:{} write_precomp:{}",
                    response_split[1],
                    response_split[2],
                    response_split[3],
                    response_split[4],
                    response_split[5],
                    response_split[6],
                );
                assert_eq!(verify_track.cylinder, response_split[1].parse().unwrap());
                assert_eq!(verify_track.head, response_split[2].parse().unwrap());
                break;
            }
            "GotCmd" => {} // Ignore
            "Fail" => panic!(
                "Failed writing cylinder {} head {} - writes:{}, reads:{}",
                response_split[1], response_split[2], response_split[3], response_split[4],
            ),
            _ => panic!("Unexpected answer from device: {}", response_text),
        }
    }
}

pub fn wait_for_answer(
    handles: &(DeviceHandle<Context>, u8, u8),
    verify_iterator: &mut Iter<RawTrack>,
) {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    // TODO copy pasta
    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .unwrap();

        let response_text = std::str::from_utf8(&in_buf[0..size]).unwrap();
        let response_split: Vec<&str> = response_text.split(' ').collect();

        match response_split[0] {
            "WrittenAndVerified" => {
                println!(
                    "Verified write of cylinder {} head {} - writes:{}, reads:{}, max_err:{} write_precomp:{}",
                    response_split[1],
                    response_split[2],
                    response_split[3],
                    response_split[4],
                    response_split[5],
                    response_split[6],
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
                "Failed writing track {} head {} - num_writes:{}, num_reads:{} error:{}",
                response_split[1],
                response_split[2],
                response_split[3],
                response_split[4],
                response_split[5],
            ),
            "WriteProtected" => panic!("Disk is write protected!"),
            _ => panic!("Unexpected answer from device: {}", response_text),
        }
    }
}
