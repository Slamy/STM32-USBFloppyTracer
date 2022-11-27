use core::cell::RefCell;

use alloc::{collections::VecDeque, vec, vec::Vec};
use heapless::spsc::Consumer;
use util::{
    mfm::{MfmEncoder2, MfmResult2},
    Bit,
};

use crate::safeiprintln;

#[derive(PartialEq, Debug, Clone, Copy)]
enum SyncType {
    Header,
    Data,
}

enum State {
    Idle,
    WaitingForSync(SyncType),
    CheckingSyncMarkType(SyncType),
    ReadingData(SyncType),
    WritingData,
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub(crate) struct CHSSector {
    pub cylinder: u8,
    pub head: u8,
    pub sector: u8,
    pub size: u8,
}

#[derive(PartialEq, Debug)]
pub(crate) enum Order {
    Read(CHSSector),
    Write(CHSSector, Vec<u8>),
    Verify(CHSSector),
}

#[derive(PartialEq, Debug)]
pub(crate) enum Response {
    Read(CHSSector, Vec<u8>),
    Write(CHSSector, bool),
    Verify(CHSSector, bool),
}

pub(crate) struct IsoSectorHandler<'a> {
    sector_buffer: Vec<u8>,
    state: State,
    crc: crc16::State<crc16::CCITT_FALSE>,
    order_queue: Consumer<'static, Order, 6>,
    result_queue: &'a RefCell<VecDeque<Response>>,
}

impl IsoSectorHandler<'_> {
    pub fn new<'a>(
        order_queue: Consumer<'static, Order, 6>,
        result_queue: &'a RefCell<VecDeque<Response>>,
    ) -> IsoSectorHandler<'a> {
        IsoSectorHandler {
            sector_buffer: Vec::with_capacity(512 + 24),
            state: State::Idle,
            crc: crc16::State::<crc16::CCITT_FALSE>::new(),
            order_queue,
            result_queue,
        }
    }

    pub fn produce<T>(&mut self, _mfme: &MfmEncoder2<T>)
    where
        T: FnMut(Bit),
    {
        //mfme.feed(inval);
    }

    pub fn feed(&mut self, data: MfmResult2) {
        self.state = match self.state {
            State::Idle => {
                if self.order_queue.ready() {
                    State::WaitingForSync(SyncType::Header)
                } else {
                    State::Idle
                }
            }
            State::WaitingForSync(in_sector) => {
                if data == MfmResult2::SyncWord {
                    self.sector_buffer.clear();
                    self.crc = crc16::State::<crc16::CCITT_FALSE>::new();
                    self.crc.update(&vec![0xa1, 0xa1, 0xa1]);

                    State::CheckingSyncMarkType(in_sector)
                } else {
                    State::WaitingForSync(in_sector)
                }
            }

            State::CheckingSyncMarkType(in_sector) => {
                if let MfmResult2::Got(y) = data {
                    self.crc.update(&y.to_ne_bytes());

                    match (y, in_sector) {
                        (0xfe, _) => State::ReadingData(SyncType::Header),
                        (0xfb, SyncType::Data) => State::ReadingData(SyncType::Data),
                        (0xf8, SyncType::Data) => State::ReadingData(SyncType::Data),
                        _ => State::WaitingForSync(SyncType::Header),
                    }
                } else {
                    State::CheckingSyncMarkType(in_sector)
                }
            }

            State::ReadingData(in_sector) => {
                if let MfmResult2::Got(y) = data {
                    self.sector_buffer.push(y);
                    self.crc.update(&y.to_ne_bytes());

                    if in_sector == SyncType::Data {
                        let expected_len = 512 + 2;

                        if self.sector_buffer.len() >= expected_len {
                            if self.crc.get() == 0 {
                                // TODO
                                //safeiprintln!("Jo!");

                                let chs;
                                if let Order::Read(wanted_sector) = self.order_queue.peek().unwrap()
                                {
                                    chs = *wanted_sector;
                                } else {
                                    panic!()
                                }

                                let mut x = core::mem::take(&mut self.sector_buffer);
                                x.pop().unwrap(); // remove CRC
                                x.pop().unwrap();

                                let r = Response::Read(chs, x);

                                self.result_queue.borrow_mut().push_back(r);
                                self.order_queue.dequeue().unwrap();
                                State::Idle
                            } else {
                                State::WaitingForSync(SyncType::Header)
                            }
                            //safeiprintln!("{:?} {}", self.sector_buffer, self.crc.get());
                        } else {
                            State::ReadingData(in_sector)
                        }
                    } else {
                        let expected_len = 6;

                        if self.sector_buffer.len() >= expected_len {
                            safeiprintln!("{:?} {}", self.sector_buffer, self.crc.get());

                            let this_sector = CHSSector {
                                cylinder: self.sector_buffer[0],
                                head: self.sector_buffer[1],
                                sector: self.sector_buffer[2],
                                size: self.sector_buffer[3],
                            };

                            if let Order::Read(wanted_sector) = self.order_queue.peek().unwrap() {
                                State::WaitingForSync(if this_sector == *wanted_sector {
                                    SyncType::Data
                                } else {
                                    SyncType::Header
                                })
                            } else {
                                panic!()
                            }
                        } else {
                            State::ReadingData(in_sector)
                        }
                    }
                } else {
                    State::ReadingData(in_sector)
                }
            }
            State::WritingData => todo!(),
        };
    }
}
