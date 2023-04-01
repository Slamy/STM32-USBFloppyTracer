#![feature(let_chains)]

use std::{
    process::exit,
    sync::{atomic::AtomicBool, Arc},
    thread::{self, JoinHandle},
};

use anyhow::{bail, ensure};
use debugless_unwrap::DebuglessUnwrap;
use fltk::{
    app::{self, channel, Sender},
    button::*,
    frame::Frame,
    group::{Pack, PackType},
    image::{JpegImage, TiledImage},
    output::Output,
    prelude::{GroupExt, WidgetBase, WidgetExt, WindowExt},
    window::Window,
};

use fltk::{enums::*, prelude::*, *};
use fltk_grid::Grid;

// get all the dark aqua colors
use rusb::{Context, DeviceHandle};
use std::sync::atomic::Ordering::Relaxed;
use util::DriveSelectState;

use tool::{
    image_reader::parse_image,
    rawtrack::RawImage,
    track_parser::read_first_track_discover_format,
    usb_commands::{configure_device, wait_for_answer, write_raw_track},
    usb_device::{clear_buffers, init_usb},
};

struct Tools {
    usb_handles: (DeviceHandle<Context>, u8, u8),
    image: Option<RawImage>,
}
#[derive(Clone)]
enum Message {
    VerifiedTrack { cylinder: u32, head: u32 },
    FailedOnTrack { cylinder: u32, head: u32 },
    LoadFile(String),
    StartWrite,
    Stop,
    Discover,
    ToolsReturned(Arc<Tools>),
    StatusMessage(String),
}

use fltk::enums::Event;

fn generate_track_table() -> Vec<Frame> {
    let mut track_labels = Vec::new();

    let mut grid = Grid::default_fill();
    //app::set_frame_color(Color::from_rgb(255, 0, 0));

    grid.debug(false); // set to true to show cell outlines and numbers
    grid.set_layout(10, 11); // 5 rows, 5 columns

    for y in 0..9 {
        let mut frame = Frame::default().with_label(&y.to_string());
        frame.set_frame(FrameType::ThinDownFrame);
        grid.insert(&mut frame, y + 1, 0); // widget, row, col
    }

    for x in 0..10 {
        let mut frame = Frame::default().with_label(&x.to_string());
        frame.set_frame(FrameType::ThinDownFrame);
        grid.insert(&mut frame, 0, x + 1); // widget, row, col
    }

    for y in 0..9 {
        for x in 0..10 {
            let mut frame = Frame::default();
            frame.set_frame(FrameType::ThinDownBox);
            frame.set_color(Color::from_rgb(0, 0, 0));
            track_labels.push(frame);

            grid.insert(track_labels.last_mut().unwrap(), y + 1, x + 1); // widget, row, col
        }
    }

    track_labels
}

fn main() {
    // connect to USB
    let usb_handles = init_usb().unwrap_or_else(|| {
        println!("Unable to initialize the USB device!");
        exit(1);
    });

    let app = app::App::default().with_scheme(app::Scheme::Gleam);

    let mut wind = Window::default()
        .with_size(900, 450)
        .with_label("USB Floppy Tracer")
        .center_screen();

    let image = JpegImage::load("/home/andre/Downloads/lined-metal-background.jpg").unwrap();
    let im2 = TiledImage::new(image, 0, 0);
    let mut frame = Frame::default_fill();
    frame.set_image(Some(im2));

    let mut pack = Pack::new(15, 15, 150, 450 - 45, None);
    pack.set_spacing(9);

    let mut button_load = Button::default().with_size(0, 30).with_label("Load Image");

    let (sender, receiver) = channel::<Message>();
    let atomic_stop = Arc::new(AtomicBool::new(false));
    //let (thread_control, thread_receiver) = channel::<Message>();

    button_load.set_callback({
        let sender = sender.clone();
        move |_| {
            let mut nfc = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            nfc.show();
            let path = nfc.filename();
            if path.exists() {
                sender.send(Message::LoadFile(path.to_str().unwrap().to_owned()));
            }
        }
    });

    let mut button_discover = Button::default().with_size(0, 30).with_label("Discover");
    button_discover.emit(sender.clone(), Message::Discover);

    let mut button_write = Button::default()
        .with_size(0, 30)
        .with_label("Write to Disk");
    button_write.deactivate();
    button_write.emit(sender.clone(), Message::StartWrite);

    let mut button_read = Button::default()
        .with_size(0, 30)
        .with_label("Read from Disk");

    let mut button_stop = Button::default().with_size(0, 30).with_label("Stop");
    button_stop.deactivate();

    button_stop.emit(sender.clone(), Message::Stop);

    let pack2 = Pack::default()
        .with_type(PackType::Horizontal)
        .with_size(150, 30);

    //pack2.set_type(PackType::Vertical);
    //pack2.set_spacing(9);
    let mut radio_drive_a = RadioLightButton::default()
        .with_label("Drive A")
        .with_size(150 / 2, 30);
    let _radio_drive_b = RadioLightButton::default()
        .with_label("Drive B")
        .with_size(150 / 2, 30);
    radio_drive_a.set(true);
    pack2.end();

    let mut frame = Frame::default().with_size(10, 50);
    frame.set_frame(FrameType::EmbossedBox);
    frame.set_color(Color::from_rgb(0, 0,180));
    frame.set_label_color(Color::from_rgb(0, 230, 0));
    //app::set_background2_color(230, 0, 0);
    //app::set_background_color(0, 250, 0);
    pack.end();

    let cellsize = 22;

    let mut loaded_image_path = Output::default().with_size(500, 30).right_of(&pack, 15);
    loaded_image_path.set_value("No image loaded");

    let mut side_0 = Pack::new(0, 0, cellsize * 11, cellsize * 10, "Side 0")
        .right_of(&pack, 10)
        .below_of(&loaded_image_path, 25);
    //side_0.set_color(Color::from_rgb(0, 255, 0));
    let mut track_labels_side0 = generate_track_table();
    side_0.end();

    let side_1 = Pack::default()
        .with_size(cellsize * 11, cellsize * 10)
        .with_label("Side 1");
    let mut track_labels_side1 = generate_track_table();
    side_1.end();
    side_1.right_of(&side_0, 20);

    let mut status_text = Output::default().with_size(500, 30).below_of(&side_0, 15);

    wind.make_resizable(false);
    wind.end();

    // Directly taken from https://fltk-rs.github.io/fltk-book/Drag-&-Drop.html
    wind.handle({
        let mut dnd = false;
        let mut released = false;
        let sender = sender.clone();
        move |_, ev| match ev {
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
        }
    });

    wind.show();

    let mut maybe_image: Option<RawImage> = None;
    let mut thread_handle: Option<JoinHandle<_>> = None;
    let mut usb_handle = Some(usb_handles);

    //app.run().unwrap();
    while app.wait() {
        let selected_drive = if radio_drive_a.is_set() {
            DriveSelectState::A
        } else {
            DriveSelectState::B
        };

        /*
        let thread_finished = thread_handle.as_ref().map_or(false, |f| f.is_finished());

        if thread_finished {
            let x = thread_handle.take().unwrap();
            let y = x.join();
            println!("Joined");
        } */

        match receiver.recv() {
            Some(Message::StatusMessage(text)) => status_text.set_value(&text),
            Some(Message::ToolsReturned(tools)) => {
                let tools = Arc::try_unwrap(tools).debugless_unwrap();
                maybe_image = tools.image;
                usb_handle = Some(tools.usb_handles);

                if maybe_image.is_some() {
                    button_write.activate();
                }
                button_read.activate();
                button_load.activate();
                button_discover.activate();

                button_stop.deactivate();
            }

            Some(Message::Stop) => {
                atomic_stop.store(true, Relaxed);
                button_stop.deactivate();
            }
            Some(Message::Discover) => {
                status_text.set_value("Checking...");

                button_write.deactivate();
                button_read.deactivate();
                button_load.deactivate();
                button_discover.deactivate();

                let taken_usb_handle = usb_handle.take().unwrap();
                let taken_image = maybe_image.take();
                let sender = sender.clone();

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                thread_handle = Some(thread::spawn(move || {
                    let result =
                        read_first_track_discover_format(&taken_usb_handle, selected_drive);

                    let status_string = match result {
                        Ok((_possible_parser, possible_formats)) => {
                            format!("{:?}", possible_formats)
                        }
                        Err(x) => x.to_string(),
                    };
                    sender.send(Message::StatusMessage(status_string));

                    sender.send(Message::ToolsReturned(Arc::new(Tools {
                        usb_handles: taken_usb_handle,
                        image: taken_image,
                    })));
                }));
            }
            Some(Message::StartWrite) => {
                let taken_image = maybe_image.take().unwrap();
                let taken_usb_handle = usb_handle.take().unwrap();

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                configure_device(&taken_usb_handle, selected_drive, taken_image.density, 0);
                let sender = sender.clone();

                button_stop.activate();

                button_write.deactivate();
                button_read.deactivate();
                button_load.deactivate();
                button_discover.deactivate();

                atomic_stop.store(false, Relaxed);
                let atomic_stop = atomic_stop.clone();

                if let Some(handle) = thread_handle.take() {
                    handle.join().unwrap();
                }

                for cell in track_labels_side0.iter_mut() {
                    cell.set_color(Color::from_rgb(0, 0, 0));
                    cell.redraw();
                }

                for cell in track_labels_side1.iter_mut() {
                    cell.set_color(Color::from_rgb(0, 0, 0));
                    cell.redraw();
                }

                status_text.set_value("Writing...");

                thread_handle = Some(thread::spawn(move || {
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
            Some(Message::LoadFile(filepath)) => match parse_image(&filepath) {
                Ok(i) => {
                    maybe_image = Some(i);
                    loaded_image_path.set_value(&filepath);
                    button_write.activate();
                }
                Err(s) => status_text.set_value(&s.to_string()),
            },
            Some(Message::FailedOnTrack { cylinder, head }) => {
                let cell = if head == 1 {
                    &mut track_labels_side1
                } else {
                    &mut track_labels_side0
                }
                .get_mut(cylinder as usize)
                .unwrap();

                cell.set_color(Color::from_rgb(255, 0, 0));
                //cell.set_label(&1.to_string());
                cell.redraw();
            }
            Some(Message::VerifiedTrack { cylinder, head }) => {
                let cell = if head == 1 {
                    &mut track_labels_side1
                } else {
                    &mut track_labels_side0
                }
                .get_mut(cylinder as usize)
                .unwrap();

                cell.set_color(Color::from_rgb(0, 255, 0));
                //cell.set_label(&1.to_string());
                cell.redraw();
            }

            None => {}
        }
    }
}

fn write_and_verify_image(
    usb_handles: &(DeviceHandle<Context>, u8, u8),
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
                write_raw_track(usb_handles, write_track);
                last_written_track = Some(write_track);
            } else {
                println!("All tracks written. Wait for remaining verifications!");
            }
        }

        loop {
            match wait_for_answer(usb_handles) {
                tool::usb_commands::UsbAnswer::WrittenAndVerified {
                    cylinder,
                    head,
                    writes: _,
                    reads: _,
                    max_err: _,
                    write_precomp: _,
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
            }
        }
    }
}
