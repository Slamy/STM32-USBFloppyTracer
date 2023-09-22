#![feature(let_chains)]
#![warn(clippy::panic)]
#![warn(clippy::expect_used)]
#![warn(clippy::indexing_slicing)]
#![warn(clippy::panic_in_result_fn)]
#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]

use anyhow::{bail, ensure, Context};
use chrono::Local;
use debugless_unwrap::DebuglessUnwrap;
use fltk::{
    app::{self, channel, Receiver, Sender},
    button::*,
    dialog::alert_default,
    frame::Frame,
    group::{Pack, PackType},
    image::{JpegImage, TiledImage},
    output::Output,
    prelude::{GroupExt, WidgetBase, WidgetExt, WindowExt},
    window::Window,
};
use fltk::{enums::*, prelude::*, *};
use rusb::DeviceHandle;
use std::sync::atomic::Ordering::Relaxed;
use std::{
    fs::File,
    io::Write,
    sync::{atomic::AtomicBool, Arc},
    thread::{self, JoinHandle},
};
use tool::{
    image_reader::parse_image,
    rawtrack::RawImage,
    track_parser::{read_first_track_discover_format, TrackPayload},
    usb_commands::{configure_device, read_raw_track, wait_for_answer, write_raw_track},
    usb_device::{clear_buffers, init_usb},
};
use util::{DriveSelectState, DRIVE_3_5_RPM, DRIVE_5_25_RPM};

struct Tools {
    usb_handles: (DeviceHandle<rusb::Context>, u8, u8),
    image: Option<RawImage>,
}

#[derive(Clone)]
enum Message {
    VerifiedTrack { cylinder: u32, head: u32 },
    FailedOnTrack { cylinder: u32, head: u32 },
    LoadFile(String),
    WriteToDisk,
    ReadFromDisk,
    Stop,
    Discover,
    ToolsReturned(Arc<Tools>),
    StatusMessage(String),
}

use fltk::enums::Event;
type FrameEventClosure = Box<dyn FnMut(&mut Frame, Event) -> bool>;

// Directly taken from https://fltk-rs.github.io/fltk-book/Drag-&-Drop.html
fn custom_handle(sender: &Sender<Message>) -> FrameEventClosure {
    let mut dnd = false;
    let mut released = false;
    let sender = sender.clone();
    Box::new(move |_, ev| match ev {
        Event::DndEnter => {
            dnd = true;
            true
        }
        Event::DndDrag => true,
        Event::DndRelease => {
            released = true;
            true
        }
        Event::Paste => {
            if dnd && released {
                let path = app::event_text();
                println!("Drag and Drop {}", path);
                let path = path.trim();
                let path = path.replace("file://", "");
                let path2 = std::path::PathBuf::from(&path);
                if path2.exists() {
                    println!("{}", path);
                    sender.send(Message::LoadFile(path));
                }
                dnd = false;
                released = false;
                true
            } else {
                false
            }
        }
        Event::DndLeave => {
            dnd = false;
            released = false;
            true
        }
        _ => false,
    })
}

fn generate_track_table(sender: &Sender<Message>) -> Vec<Frame> {
    let mut track_labels = Vec::new();

    let pack = Pack::default()
        .with_type(PackType::Horizontal)
        .with_size(0, 22);

    Frame::default().with_size(22, 22); // empty tile top left
    for x in 0..10 {
        let mut frame = Frame::default()
            .with_label(&x.to_string())
            .with_size(22, 22);
        frame.set_frame(FrameType::ThinDownFrame);
    }

    pack.end();

    for y in 0..9 {
        let pack = Pack::default()
            .with_type(PackType::Horizontal)
            .with_size(22 * 11, 22);

        let mut frame = Frame::default()
            .with_label(&y.to_string())
            .with_size(22, 22);
        frame.set_frame(FrameType::ThinDownFrame);

        for _ in 0..10 {
            let mut frame = Frame::default().with_size(22, 22);
            frame.set_frame(FrameType::ThinDownBox);
            frame.set_color(Color::from_rgb(0, 0, 0));
            frame.handle(custom_handle(sender));

            track_labels.push(frame);
        }
        pack.end();
    }

    track_labels
}

struct TrackLabels {
    frames: [Vec<Frame>; 2],
}

impl TrackLabels {
    fn all_black(&mut self) {
        for cell in self.frames.iter_mut().flatten() {
            cell.set_color(Color::from_rgb(0, 0, 0));
            cell.redraw();
        }
    }

    fn set_color(&mut self, cylinder: u32, head: u32, color: Color) -> Option<()> {
        let cell = &mut self
            .frames
            .get_mut(head as usize)?
            .get_mut(cylinder as usize)?;
        cell.set_color(color);
        cell.redraw();
        Some(())
    }

    fn black_if_existing(&mut self, image: &RawImage) {
        for cell in self.frames.iter_mut().flatten() {
            cell.set_color(Color::from_rgb(128, 128, 128));
        }

        for track in &image.tracks {
            self.set_color(track.cylinder, track.head, Color::from_rgb(0, 0, 0));
        }

        for cell in self.frames.iter_mut().flatten() {
            cell.redraw();
        }
    }
}

struct UsbFloppyTracerWindow {
    button_load: Button,
    atomic_stop: Arc<AtomicBool>,
    button_discover: Button,
    button_read: Button,
    button_write: Button,
    button_stop: Button,
    radio_drive_a: RadioLightButton,
    radio_drive_b: RadioLightButton,
    checkbox_flippy_disk: CheckButton,
    receiver: Receiver<Message>,
    sender: Sender<Message>,
    maybe_image: Option<RawImage>,
    usb_handle: Option<(DeviceHandle<rusb::Context>, u8, u8)>,
    status_text: Output,
    tracklabels: TrackLabels,
    thread_handle: Option<JoinHandle<()>>,
    loaded_image_path: Output,
}
impl UsbFloppyTracerWindow {
    fn new() -> Self {
        let mut wind = Window::default()
            .with_size(750, 380)
            .with_label("USB Floppy Tracer")
            .center_screen();

        let mut frame = Frame::default_fill();

        let image = include_bytes!("../assets/lined-metal-background.jpg");
        if let Ok(image) = JpegImage::from_data(image) {
            let im2 = TiledImage::new(image, 0, 0);
            frame.set_image(Some(im2));
        }

        let mut pack = Pack::new(15, 15, 150, 0, None);
        pack.set_spacing(9);

        let mut button_load = Button::default().with_size(0, 30).with_label("Load Image");

        let (sender, receiver) = channel::<Message>();
        let atomic_stop = Arc::new(AtomicBool::new(false));

        button_load.set_callback({
            let sender = sender.clone();
            move |_| {
                let mut nfc =
                    dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
                nfc.show();
                let path = nfc.filename();
                if path.exists() {
                    if let Some(f) = path.to_str() {
                        sender.send(Message::LoadFile(f.to_owned()));
                    }
                }
            }
        });

        let mut button_discover = Button::default().with_size(0, 30).with_label("Discover");
        button_discover.emit(sender.clone(), Message::Discover);

        let mut button_write = Button::default()
            .with_size(0, 30)
            .with_label("Write to Disk");
        button_write.deactivate();
        button_write.emit(sender.clone(), Message::WriteToDisk);

        let mut button_read = Button::default()
            .with_size(0, 30)
            .with_label("Read from Disk");
        button_read.emit(sender.clone(), Message::ReadFromDisk);

        let mut button_stop = Button::default().with_size(0, 30).with_label("Stop");
        button_stop.deactivate();

        button_stop.emit(sender.clone(), Message::Stop);

        let pack2 = Pack::default()
            .with_type(PackType::Horizontal)
            .with_size(150, 30);

        let mut radio_drive_a = RadioLightButton::default()
            .with_label("Drive A")
            .with_size(150 / 2, 30);
        let radio_drive_b = RadioLightButton::default()
            .with_label("Drive B")
            .with_size(150 / 2, 30);
        radio_drive_a.set(true);
        pack2.end();

        let checkbox_flippy_disk = CheckButton::default()
            .with_label("Flippy Disk")
            .with_size(0, 25);

        pack.end();

        let cellsize = 22;

        let mut loaded_image_path = Output::default().with_size(500, 30).right_of(&pack, 15);
        loaded_image_path.set_value("No image loaded");

        let side_0 = Pack::new(0, 0, cellsize * 11, cellsize * 10, "Side 0")
            .right_of(&pack, 10)
            .below_of(&loaded_image_path, 25);
        let track_labels_side0 = generate_track_table(&sender);
        side_0.end();

        let side_1 = Pack::default()
            .with_size(cellsize * 11, cellsize * 10)
            .with_label("Side 1");
        let track_labels_side1 = generate_track_table(&sender);

        side_1.end();
        side_1.right_of(&side_0, cellsize);

        let tracklabels = TrackLabels {
            frames: [track_labels_side0, track_labels_side1],
        };

        let mut status_text = Output::default().with_size(500, 30).below_of(&side_0, 15);

        wind.make_resizable(false);
        wind.end();

        frame.handle(custom_handle(&sender));

        let maybe_image: Option<RawImage> = None;
        let thread_handle: Option<JoinHandle<_>> = None;
        let usb_handle = init_usb();

        if usb_handle.is_ok() {
            status_text.set_value("Systems ready!");
        } else {
            status_text.set_value(&format!(
                "Failed to initialize USB device: {:?}",
                usb_handle
            ));
        }

        wind.show();

        UsbFloppyTracerWindow {
            button_load,
            atomic_stop,
            button_discover,
            button_read,
            button_stop,
            radio_drive_a,
            radio_drive_b,
            receiver,
            sender,
            maybe_image,
            thread_handle,
            usb_handle: usb_handle.ok(),
            status_text,
            button_write,
            tracklabels,
            loaded_image_path,
            checkbox_flippy_disk,
        }
    }

    fn take_usb_handle(&mut self) -> anyhow::Result<(DeviceHandle<rusb::Context>, u8, u8)> {
        if self.usb_handle.is_none() {
            self.usb_handle = Some(init_usb()?);
        }
        self.usb_handle
            .take()
            .context("USB Device still not available!")
    }

    fn handle(&mut self) -> anyhow::Result<()> {
        let selected_drive = if self.radio_drive_a.is_set() {
            DriveSelectState::A
        } else {
            DriveSelectState::B
        };

        // TODO better documentation here
        let index_sim_frequency = if self.checkbox_flippy_disk.is_checked() {
            (14 * 1000) * 1000
        } else {
            0
        };

        match self.receiver.recv() {
            Some(Message::StatusMessage(text)) => self.status_text.set_value(&text),
            Some(Message::ToolsReturned(tools)) => {
                let tools = Arc::try_unwrap(tools).debugless_unwrap();
                self.maybe_image = tools.image;
                self.usb_handle = Some(tools.usb_handles);

                if self.maybe_image.is_some() {
                    self.button_write.activate();
                }
                self.button_read.activate();
                self.button_load.activate();
                self.button_discover.activate();
                self.radio_drive_a.activate();
                self.radio_drive_b.activate();

                self.button_stop.deactivate();
            }

            Some(Message::Stop) => {
                self.atomic_stop.store(true, Relaxed);
                self.button_stop.deactivate();
            }
            Some(Message::Discover) => {
                let taken_usb_handle = self.take_usb_handle()?;
                let taken_image = self.maybe_image.take();
                let sender = self.sender.clone();

                self.status_text.set_value("Checking...");

                self.button_write.deactivate();
                self.button_read.deactivate();
                self.button_load.deactivate();
                self.button_discover.deactivate();
                self.radio_drive_a.deactivate();
                self.radio_drive_b.deactivate();

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                let thread_handle = thread::spawn(move || {
                    let result = read_first_track_discover_format(
                        &taken_usb_handle,
                        selected_drive,
                        index_sim_frequency,
                    );

                    let status_string = match result {
                        Ok((_possible_parser, possible_formats)) => {
                            if possible_formats.is_empty() {
                                "No known format detected".into()
                            } else {
                                format!("Format is probably {:?}", possible_formats)
                            }
                        }
                        Err(x) => x.to_string(),
                    };
                    sender.send(Message::StatusMessage(status_string));

                    sender.send(Message::ToolsReturned(Arc::new(Tools {
                        usb_handles: taken_usb_handle,
                        image: taken_image,
                    })));
                });

                self.thread_handle = Some(thread_handle);
            }
            Some(Message::ReadFromDisk) => {
                let taken_image = self.maybe_image.take();
                let taken_usb_handle = self.take_usb_handle()?;

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                let sender = self.sender.clone();

                self.button_stop.activate();

                self.button_write.deactivate();
                self.button_read.deactivate();
                self.button_load.deactivate();
                self.button_discover.deactivate();
                self.radio_drive_a.deactivate();
                self.radio_drive_b.deactivate();

                self.atomic_stop.store(false, Relaxed);
                let atomic_stop = self.atomic_stop.clone();

                if let Some(handle) = self.thread_handle.take() {
                    handle.join().ok();
                }

                self.tracklabels.all_black();

                self.status_text.set_value("Reading...");

                self.thread_handle = Some(thread::spawn(move || {
                    let result = read_tracks_to_diskimage(
                        &taken_usb_handle,
                        selected_drive,
                        sender.clone(),
                        atomic_stop,
                        index_sim_frequency,
                    );

                    let status_string = match result {
                        Ok(()) => "Disk read to image!".into(),
                        Err(x) => x.to_string(),
                    };

                    sender.send(Message::StatusMessage(status_string));

                    sender.send(Message::ToolsReturned(Arc::new(Tools {
                        usb_handles: taken_usb_handle,
                        image: taken_image,
                    })));
                }));
            }
            Some(Message::WriteToDisk) => {
                let taken_image = self.maybe_image.take().context("No image loaded!")?;
                let taken_usb_handle = self.take_usb_handle()?;

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                configure_device(
                    &taken_usb_handle,
                    selected_drive,
                    taken_image.density,
                    index_sim_frequency,
                )?;
                let sender = self.sender.clone();

                self.button_stop.activate();

                self.button_write.deactivate();
                self.button_read.deactivate();
                self.button_load.deactivate();
                self.button_discover.deactivate();
                self.radio_drive_a.deactivate();
                self.radio_drive_b.deactivate();

                self.atomic_stop.store(false, Relaxed);
                let atomic_stop = self.atomic_stop.clone();

                if let Some(handle) = self.thread_handle.take() {
                    handle.join().ok();
                }

                self.tracklabels.black_if_existing(&taken_image);

                self.status_text.set_value("Writing...");

                self.thread_handle = Some(thread::spawn(move || {
                    let result = write_and_verify_image(
                        &taken_usb_handle,
                        &taken_image,
                        sender.clone(),
                        atomic_stop,
                    );

                    let status_string = match result {
                        Ok(()) => "Image written!".into(),
                        Err(x) => x.to_string(),
                    };

                    sender.send(Message::StatusMessage(status_string));

                    sender.send(Message::ToolsReturned(Arc::new(Tools {
                        usb_handles: taken_usb_handle,
                        image: Some(taken_image),
                    })));
                }));
            }
            Some(Message::LoadFile(filepath)) => match parse_image(&filepath).and_then(|x| {
                let rpm = match x.disk_type {
                    util::DiskType::Inch3_5 => DRIVE_3_5_RPM,
                    util::DiskType::Inch5_25 => DRIVE_5_25_RPM,
                };

                for track in &x.tracks {
                    track.assert_fits_into_rotation(rpm)?;
                    track.check_writability()?;
                }
                Ok(x)
            }) {
                Ok(i) => {
                    self.tracklabels.black_if_existing(&i);
                    self.maybe_image = Some(i);
                    self.loaded_image_path.set_value(&filepath);
                    self.button_write.activate();
                }
                Err(s) => {
                    println!("{:?}", s);

                    self.status_text.set_value(&s.to_string())
                }
            },
            Some(Message::FailedOnTrack { cylinder, head }) => {
                self.tracklabels
                    .set_color(cylinder, head, Color::from_rgb(255, 0, 0));
            }
            Some(Message::VerifiedTrack { cylinder, head }) => {
                self.tracklabels
                    .set_color(cylinder, head, Color::from_rgb(0, 255, 0));
            }

            None => {}
        }

        Ok(())
    }
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Gleam);

    let mut window = UsbFloppyTracerWindow::new();
    while app.wait() {
        if let Err(e) = window.handle() {
            alert_default(&e.to_string());
        }
    }
}

fn read_tracks_to_diskimage(
    usb_handles: &(DeviceHandle<rusb::Context>, u8, u8),
    select_drive: DriveSelectState,
    sender: Sender<Message>,
    atomic_stop: Arc<AtomicBool>,
    index_sim_frequency: u32,
) -> Result<(), anyhow::Error> {
    let (possible_track_parser, possible_formats) =
        read_first_track_discover_format(usb_handles, select_drive, index_sim_frequency)?;

    let mut track_parser = possible_track_parser.context("Unable to detect floppy format!")?;
    println!("Format is probably '{:?}'", possible_formats);

    let now = Local::now();
    let time_str = now.format("%Y%m%d_%H%M%S");
    let filepath = format!("{}.{}", time_str, track_parser.default_file_extension());

    println!("Resulting image will be {filepath}");

    let track_filter = track_parser.default_trackfilter();
    let duration_to_record = track_parser.duration_to_record();
    configure_device(
        usb_handles,
        select_drive,
        track_parser.track_density(),
        index_sim_frequency,
    )?;

    let mut cylinder_begin = track_filter.cyl_start.unwrap_or(0);
    let mut cylinder_end = track_filter
        .cyl_end
        .context("Please specify the last cylinder to read!")?;

    if cylinder_begin == cylinder_end {
        cylinder_begin = 0;
    } else {
        cylinder_end += 1;
    }

    let heads = match track_filter.head {
        Some(0) => 0..1,
        Some(1) => 1..2,
        None => 0..2,
        _ => bail!("Program flow error!"),
    };

    println!("Reading cylinders {cylinder_begin} to {cylinder_end}");
    let mut outfile = File::create(filepath)?;

    for cylinder in (cylinder_begin..cylinder_end).step_by(track_parser.step_size()) {
        for head in heads.clone() {
            track_parser.expect_track(cylinder, head);

            let mut possible_track: Option<TrackPayload> = None;

            for _ in 0..5 {
                if atomic_stop.load(Relaxed) {
                    bail!("Stopped before finishing the operation");
                }

                let raw_data =
                    read_raw_track(usb_handles, cylinder, head, false, duration_to_record)?;
                let track = track_parser.parse_raw_track(&raw_data).ok();

                if track.is_some() {
                    possible_track = track;
                    break;
                }

                println!("Reading of track {cylinder} {head} not successful. Try again...");

                sender.send(Message::FailedOnTrack { cylinder, head });
            }

            let track =
                possible_track.context(format!("Unable to read track {} {}", cylinder, head))?;
            ensure!(cylinder == track.cylinder);
            ensure!(head == track.head);

            sender.send(Message::VerifiedTrack { cylinder, head });

            outfile.write_all(&track.payload)?;
        }
    }

    Ok(())
}

fn write_and_verify_image(
    usb_handles: &(DeviceHandle<rusb::Context>, u8, u8),
    image: &RawImage,
    sender: Sender<Message>,
    atomic_stop: Arc<AtomicBool>,
) -> Result<(), anyhow::Error> {
    let mut write_iterator = image.tracks.iter();
    let mut verify_iterator = image.tracks.iter();

    let mut expected_to_verify = verify_iterator.next();

    let mut last_written_track = None;
    loop {
        if !atomic_stop.load(Relaxed) {
            if let Some(write_track) = write_iterator.next() {
                write_raw_track(usb_handles, write_track)?;
                last_written_track = Some(write_track);
            } else {
                println!("All tracks written. Wait for remaining verifications!");
            }
        }

        loop {
            match wait_for_answer(usb_handles)? {
                tool::usb_commands::UsbAnswer::WrittenAndVerified {
                    cylinder,
                    head,
                    writes: _,
                    reads: _,
                    max_err: _,
                    write_precomp: _,
                    similarity_threshold: _,
                    match_after_pulses: _,
                } => {
                    sender.send(Message::VerifiedTrack { cylinder, head });

                    if let Some(track) = expected_to_verify {
                        ensure!(track.cylinder == cylinder);
                        ensure!(track.head == head);

                        if let Some(last_written_track) = last_written_track && atomic_stop.load(Relaxed) && last_written_track.cylinder == track.cylinder && last_written_track.head == track.head{
                            bail!("Stopped before finishing the operation");
                        }
                    }
                    expected_to_verify = verify_iterator.next();
                    if expected_to_verify.is_none() {
                        println!("--- Disk Image written and verified! ---");
                        return Ok(());
                    }
                }
                tool::usb_commands::UsbAnswer::Fail {
                    cylinder,
                    head,
                    writes,
                    reads,
                    error,
                } => {
                    sender.send(Message::FailedOnTrack { cylinder, head });

                    bail!(
                        "Failed writing track {} head {} - num_writes:{}, num_reads:{} error:{}",
                        cylinder,
                        head,
                        writes,
                        reads,
                        error,
                    )
                }
                tool::usb_commands::UsbAnswer::GotCmd => {
                    break;
                }
                tool::usb_commands::UsbAnswer::WriteProtected => bail!("Disk is write protected!"),
                tool::usb_commands::UsbAnswer::RotationTicks { ticks: _ } => {
                    bail!("Unexpected answer!")
                }
            }
        }
    }
}
