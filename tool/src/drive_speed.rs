use anyhow::{bail, Context};
use io::BufRead;
use std::{
    fs::File,
    io::{self, Write},
};
use util::{DriveSelectState, DRIVE_5_25_TIMER_TICKS_PER_ROTATION};

pub fn read_stored_timer_ticks_per_rotation(drive: DriveSelectState) -> anyhow::Result<u32> {
    let filename = match drive {
        DriveSelectState::None => bail!("Should never happen"),
        DriveSelectState::A => "speed_a.cfg",
        DriveSelectState::B => "speed_b.cfg",
    };

    let config_path = home::home_dir()
        .context("Home Directoy not available")?
        .join(".usbfloppytracer/")
        .join(filename);

    println!("Reading drive speed from {config_path:?}");

    let file = File::open(config_path).map_err(|f| {
        println!("Custom drive speed not found. Use default...");
        f
    })?;

    let mut lines = io::BufReader::new(file).lines();
    let first_line = lines.next().context("No first line?")??;
    let number: u32 = first_line.parse()?;
    println!("Using custom ticks per rotation: {}", number);

    Ok(number)
}

pub fn get_timer_ticks_per_rotation(drive: DriveSelectState) -> u32 {
    read_stored_timer_ticks_per_rotation(drive).unwrap_or(DRIVE_5_25_TIMER_TICKS_PER_ROTATION)
}

pub fn store_timer_ticks_per_rotation(drive: DriveSelectState, ticks: u32) -> anyhow::Result<()> {
    let filename = match drive {
        DriveSelectState::None => bail!("Should never happen"),
        DriveSelectState::A => "speed_a.cfg",
        DriveSelectState::B => "speed_b.cfg",
    };

    let config_path = home::home_dir()
        .context("Home Directoy not available")?
        .join(".usbfloppytracer/")
        .join(filename);

    let mut file = File::create(&config_path).map_err(|f| {
        println!("Unable to create file: {f}");
        f
    })?;

    file.write_all(format!("{ticks}\n").as_bytes())?;

    println!("Drive speed is stored in {config_path:?}");

    Ok(())
}
