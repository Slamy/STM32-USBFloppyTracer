#![feature(let_chains)]

use core::time;
use std::{
    os::unix::thread::JoinHandleExt,
    process::exit,
    rc::Rc,
    sync::{atomic::AtomicBool, Arc},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{bail, ensure};
use debugless_unwrap::DebuglessUnwrap;
use fltk::{
    app::{self, channel, set_callback, Scheme, Sender},
    button::*,
    frame::Frame,
    group::{Flex, Group, Pack, Tabs},
    input::Input,
    menu::{Choice, MenuButton},
    output::Output,
    prelude::{GroupExt, MenuExt, WidgetBase, WidgetExt, WindowExt},
    window::Window,
};
use fltk_table::{SmartTable, TableOpts};
use fltk_theme::{widget_themes, ThemeType, WidgetTheme};
use fltk_theme::{SchemeType, WidgetScheme};

use fltk::{enums::*, prelude::*, *};
use fltk_grid::Grid;
use fltk_theme::colors::aqua::dark::*;
use fltk_theme::widget_schemes::aqua::frames::*; // get all the dark aqua colors
use rusb::{Context, DeviceHandle};
use std::sync::atomic::Ordering::Relaxed;
use util::{DriveSelectState, DRIVE_3_5_RPM, DRIVE_5_25_RPM};

use tool::{
    image_reader::parse_image,
    rawtrack::RawImage,
    usb_commands::{configure_device, wait_for_answer, write_raw_track},
    usb_device::{clear_buffers, init_usb},
};

struct Tools {
    usb_handles: (DeviceHandle<Context>, u8, u8),
    image: RawImage,
}
#[derive(Clone)]
enum Message {
    VerifiedTrack { cylinder: u32, head: u32 },
    FailedOnTrack { cylinder: u32, head: u32 },
    Tick,
    LoadFile(String),
    StartWrite,
    Stop,
    ToolsReturned(Arc<Tools>),
}

use fltk::{enums::Event, prelude::*, *};

fn generate_track_table() -> Vec<Frame> {
    let mut track_labels = Vec::new();

    let mut grid = Grid::default_fill();
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
        .with_label("Tabs")
        .center_screen();

    let mut pack = Pack::new(15, 15, 150, 450 - 45, None);
    pack.set_spacing(9);

    let mut button_load = Button::default().with_size(0, 30).with_label("Load Image");

    let (sender, receiver) = channel::<Message>();
    let atomic_stop = Arc::new(AtomicBool::new(false));
    //let (thread_control, thread_receiver) = channel::<Message>();

    button_load.set_callback({
        let sender = sender.clone();
        move |f| {
            let mut nfc = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            nfc.show();
            let path = nfc.filename();
            //println!("File {:?}", nfc.filename());
            sender.send(Message::LoadFile(path.to_str().unwrap().to_owned()));
        }
    });

    let button_discover = Button::default().with_size(0, 30).with_label("Discover");
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

    pack.end();

    let cellsize = 22;

    let mut loaded_image_path = Output::default().with_size(500, 30).right_of(&pack, 15);
    loaded_image_path.set_value("No image loaded");

    let side_0 = Pack::new(0, 0, cellsize * 11, cellsize * 10, "Side 0")
        .right_of(&pack, 10)
        .below_of(&loaded_image_path, 25);
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

    let mut image: Option<RawImage> = None;
    let mut thread_handle: Option<JoinHandle<_>> = None;
    let mut usb_handle = Some(usb_handles);

    //app.run().unwrap();
    while app.wait() {
        match receiver.recv() {
            Some(Message::ToolsReturned(tools)) => {
                let tools = Arc::try_unwrap(tools).debugless_unwrap();
                image = Some(tools.image);
                usb_handle = Some(tools.usb_handles);

                if image.is_some() {
                    button_write.activate();
                }
            }

            Some(Message::Stop) => {
                atomic_stop.store(true, Relaxed);
                button_stop.deactivate();
            }
            Some(Message::StartWrite) => {
                let taken_image = image.take().unwrap();
                let taken_usb_handle = usb_handle.take().unwrap();

                // it might be sometimes possible during an abort, that the endpoint
                // still contains data. Must be removed before proceeding
                clear_buffers(&taken_usb_handle);

                configure_device(
                    &taken_usb_handle,
                    DriveSelectState::A,
                    taken_image.density,
                    0,
                );
                let sender = sender.clone();
                button_write.deactivate();
                button_stop.activate();
                button_read.deactivate();

                atomic_stop.store(false, Relaxed);
                let atomic_stop = atomic_stop.clone();

                if let Some(handle) = thread_handle.take() {
                    let _ = handle.join().unwrap();
                }

                for cell in track_labels_side0.iter_mut() {
                    cell.set_color(Color::from_rgb(0, 0, 0));
                    cell.redraw();
                }

                for cell in track_labels_side1.iter_mut() {
                    cell.set_color(Color::from_rgb(0, 0, 0));
                    cell.redraw();
                }

                thread_handle = Some(thread::spawn(move || {
                    write_and_verify_image(
                        &taken_usb_handle,
                        &taken_image,
                        sender.clone(),
                        atomic_stop,
                    );

                    sender.send(Message::ToolsReturned(Arc::new(Tools {
                        usb_handles: taken_usb_handle,
                        image: taken_image,
                    })));
                }));
            }
            Some(Message::LoadFile(filepath)) => {
                image = Some(parse_image(&filepath));
                loaded_image_path.set_value(&filepath);
                button_write.activate();
            }
            Some(Message::Tick) => {}
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
        if atomic_stop.load(Relaxed) == false {
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
                    writes,
                    reads,
                    max_err,
                    write_precomp,
                } => {
                    sender.send(Message::VerifiedTrack { cylinder, head });

                    if let Some(track) = expected_to_verify {
                        ensure!(track.cylinder == cylinder);
                        ensure!(track.head == head);

                        if let Some(last_written_track) = last_written_track && atomic_stop.load(Relaxed) == true && last_written_track.cylinder == track.cylinder && last_written_track.head == track.head{
                            println!("Stopped!");
                        return Ok(());
                            
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
                } => bail!(
                    "Failed writing track {} head {} - num_writes:{}, num_reads:{} error:{}",
                    cylinder,
                    head,
                    writes,
                    reads,
                    error,
                ),
                tool::usb_commands::UsbAnswer::GotCmd => {
                    println!("Got cmd");
                    break;
                }
                tool::usb_commands::UsbAnswer::WriteProtected => bail!("Disk is write protected!"),
            }
        }
    }
}
