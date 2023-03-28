use fltk::{
    app::{self, channel, set_callback, Scheme},
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
use fltk_theme::widget_schemes::aqua::frames::*;

use fltk_theme::colors::aqua::dark::*; // get all the dark aqua colors

use fltk_grid::Grid;

#[derive(Clone, Copy)]
enum Message {
    Reset,
    ChangeDuration,
    Tick,
}

use fltk::{enums::Event, prelude::*, *};

fn generate_track_table() -> Vec<Frame> {
    let mut track_labels = Vec::new();

    let mut grid = Grid::default_fill();
    grid.debug(false); // set to true to show cell outlines and numbers
    grid.set_layout(11, 11); // 5 rows, 5 columns

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
    let app = app::App::default().with_scheme(app::Scheme::Gleam);

    let mut wind = Window::default()
        .with_size(900, 450)
        .with_label("Tabs")
        .center_screen();

    let mut pack = Pack::new(15, 15, 150, 450 - 45, None);
    pack.set_spacing(9);

    let mut button_load = Button::default().with_size(0, 30).with_label("Load Image");

    button_load.set_callback(|f| {
        let mut nfc = dialog::NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
        nfc.show();
        println!("File {:?}", nfc.filename());
    });
    let button_discover = Button::default().with_size(0, 30).with_label("Discover");

    let mut button_write = Button::default()
        .with_size(0, 30)
        .with_label("Write to Disk");
    button_write.deactivate();

    let button_read = Button::default()
        .with_size(0, 30)
        .with_label("Read from Disk");

    let (sender, receiver) = channel::<Message>();

    pack.end();

    let mut loaded_image_path = Output::default().with_size(500, 30).right_of(&pack, 15);
    loaded_image_path.set_value("No image loaded");

    let side_0 = Pack::new(0, 0, 300, 300, "Side 0")
        .right_of(&pack, 10)
        .below_of(&loaded_image_path, 25);
    let mut track_labels_side0 = generate_track_table();
    side_0.end();

    let side_1 = Pack::default().with_size(300, 300).with_label("Side 1");
    let mut track_labels_side1 = generate_track_table();
    side_1.end();
    side_1.right_of(&side_0, 20);

    let mut track_label_ter = track_labels_side0.iter_mut();

    wind.make_resizable(false);
    wind.end();

    // Directly taken from https://fltk-rs.github.io/fltk-book/Drag-&-Drop.html
    wind.handle({
        let mut dnd = false;
        let mut released = false;
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
                    println!("{path}");
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

    //app.run().unwrap();
    while app.wait() {
        match receiver.recv() {
            Some(Message::Reset) => {}
            Some(Message::ChangeDuration) => {
                println!("Barf!");

                let x = track_label_ter.next().unwrap();

                x.set_color(Color::from_rgb(0, 255, 0));
                x.redraw();
            }
            Some(Message::Tick) => {}
            None => {}
        }
    }
}
