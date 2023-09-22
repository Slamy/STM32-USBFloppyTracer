use std::time::Duration;

use anyhow::{bail, ensure, Context};
use rusb::DeviceHandle;
use util::{Density, DriveSelectState};

use crate::rawtrack::RawTrack;

pub fn configure_device(
    handles: &(DeviceHandle<rusb::Context>, u8, u8),
    select_drive: DriveSelectState,
    density: Density,
    index_sim_frequency: u32,
) -> anyhow::Result<()> {
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
        .context(program_flow_error!())?
        .clone_from_slice(&u32::to_le_bytes(0x1234_0002));

    writer
        .next()
        .context(program_flow_error!())?
        .clone_from_slice(&u32::to_le_bytes(settings));

    writer
        .next()
        .context(program_flow_error!())?
        .clone_from_slice(&u32::to_le_bytes(index_sim_frequency));

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .context("Bulk Write failed - USB Problem?")?;

    Ok(())
}

pub fn measure_ticks_per_rotation(
    handles: &(DeviceHandle<rusb::Context>, u8, u8),
    select_drive: DriveSelectState,
) -> anyhow::Result<u32> {
    configure_device(handles, select_drive, Density::High, 0)?;

    let (handle, _, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    let mut command_buf = [0u8; 64];
    let mut writer = command_buf.chunks_mut(4);

    writer
        .next()
        .context(program_flow_error!())?
        .clone_from_slice(&u32::to_le_bytes(0x1234_0005));

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .context("Write Bulk Transfer failed - USB Problem?")?;

    let result = wait_for_answer(handles)?;

    if let UsbAnswer::RotationTicks { ticks } = result {
        Ok(ticks)
    } else {
        bail!("Unexpected response!");
    }
}

pub fn read_raw_track(
    handles: &(DeviceHandle<rusb::Context>, u8, u8),
    cylinder: u32,
    head: u32,
    wait_for_index: bool,
    duration_to_record: usize,
) -> anyhow::Result<Vec<u8>> {
    let (handle, endpoint_in, endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    println!("Read raw track from Cyl:{cylinder} Head:{head}");

    let mut command_buf = [0u8; 64];
    let mut writer = command_buf.chunks_mut(4);

    let wait_for_index = if wait_for_index { 1 << 9 } else { 0 };

    let header = vec![
        0x1234_0004,
        cylinder | (head << 8) | wait_for_index,
        duration_to_record as u32,
    ];

    for word in header {
        writer
            .next()
            .context(program_flow_error!())?
            .clone_from_slice(&u32::to_le_bytes(word));
    }

    handle
        .write_bulk(*endpoint_out, &command_buf, timeout)
        .context("Write Bulk Transfer failed - USB Problem?")?;

    let mut result = Vec::with_capacity(800 * 64); // TODO magic number

    loop {
        let mut in_buf = [0u8; 64];

        let size = handle
            .read_bulk(*endpoint_in, &mut in_buf, timeout)
            .context("Read Bulk failed - USB Problem?")?;

        if size == 64 {
            result.extend_from_slice(&in_buf);
        } else if size == 0 {
            // End sign
            break;
        } else {
            let response_text =
                std::str::from_utf8(&ensure_index!(in_buf[0..size])).context("UTF8 error")?;
            bail!("{}", response_text);
        }
    }

    if result.len() == 64 {
        println!("{result:?}");
    }
    Ok(result)
}

pub fn write_raw_track(
    handles: &(DeviceHandle<rusb::Context>, u8, u8),
    track: &RawTrack,
) -> anyhow::Result<()> {
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

    ensure!(track.head <= 1);
    ensure!(track.cylinder <= 0xff);
    ensure!(track.write_precompensation <= 0xff);

    let non_flux_reversal_mask = if track.has_non_flux_reversal_area {
        0x200
    } else {
        0
    };

    let header = vec![
        0x1234_0001,
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
            .context(program_flow_error!())?
            .clone_from_slice(&u32::to_le_bytes(i));
    }

    for density_entry in &track.densitymap {
        ensure!(density_entry.cell_size.0 < 512);

        writer
            .next()
            .context(program_flow_error!())?
            .clone_from_slice(&u32::to_le_bytes(
                ((density_entry.number_of_cellbytes as u32) << 9)
                    | density_entry.cell_size.0 as u32,
            ));
    }

    handle.write_bulk(*endpoint_out, &command_buf, timeout)?;

    for block in track.raw_data.chunks(64) {
        handle.write_bulk(*endpoint_out, block, timeout)?;
    }

    Ok(())
}

pub enum UsbAnswer {
    WrittenAndVerified {
        cylinder: u32,
        head: u32,
        writes: u32,
        reads: u32,
        max_err: u32,
        similarity_threshold: u32,
        match_after_pulses: u32,
        write_precomp: u32,
    },
    Fail {
        cylinder: u32,
        head: u32,
        writes: u32,
        reads: u32,
        error: String,
    },
    GotCmd,
    WriteProtected,
    RotationTicks {
        ticks: u32,
    },
}

pub fn wait_for_answer(
    handles: &(DeviceHandle<rusb::Context>, u8, u8),
) -> anyhow::Result<UsbAnswer> {
    let (handle, endpoint_in, _endpoint_out) = handles;
    let timeout = Duration::from_secs(10);

    // TODO copy pasta
    let mut in_buf = [0u8; 64];

    let size = handle.read_bulk(*endpoint_in, &mut in_buf, timeout)?;

    let response_text =
        std::str::from_utf8(&ensure_index!(in_buf[0..size])).context("UTF8 error")?;
    let response_split: Vec<&str> = response_text.split(' ').collect();

    Ok(match ensure_index!(response_split[0]) {
        "RotationTicks" => {
            let ticks = ensure_index!(response_split[1]).parse()?;

            UsbAnswer::RotationTicks { ticks }
        }
        "WrittenAndVerified" => {
            let cylinder = ensure_index!(response_split[1]).parse()?;
            let head = ensure_index!(response_split[2]).parse()?;
            let writes = ensure_index!(response_split[3]).parse()?;
            let reads = ensure_index!(response_split[4]).parse()?;
            let max_err = ensure_index!(response_split[5]).parse()?;
            let similarity_threshold: u32 = ensure_index!(response_split[6]).parse()?;
            let match_after_pulses: u32 = ensure_index!(response_split[7]).parse()?;
            let write_precomp: u32 = ensure_index!(response_split[8]).parse()?;

            UsbAnswer::WrittenAndVerified {
                cylinder,
                head,
                writes,
                reads,
                max_err,
                similarity_threshold,
                match_after_pulses,
                write_precomp,
            }
        }
        "GotCmd" => UsbAnswer::GotCmd,
        "Fail" => {
            let cylinder = ensure_index!(response_split[1]).parse()?;
            let head = ensure_index!(response_split[2]).parse()?;
            let writes = ensure_index!(response_split[3]).parse()?;
            let reads = ensure_index!(response_split[4]).parse()?;
            let error = ensure_index!(response_split[5]).into();
            UsbAnswer::Fail {
                cylinder,
                head,
                writes,
                reads,
                error,
            }
        }
        "WriteProtected" => UsbAnswer::WriteProtected,
        _ => bail!("Unexpected answer from device: {}", response_text),
    })
}
