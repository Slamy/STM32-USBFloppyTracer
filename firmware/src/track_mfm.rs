use core::{borrow::BorrowMut, cell::RefCell, pin::Pin, slice::Iter};
use ouroboros::self_referencing;

use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use heapless::spsc::Consumer;
use util::{
    mfm::{MfmEncoder2, MfmResult2},
    Bit,
};

#[derive(PartialEq, Debug)]
pub(crate) enum Order {
    Write(u8, Vec<u8>),
    Verify(u8),
}

#[derive(PartialEq, Debug)]
pub(crate) enum Response {
    Write(u8, bool),
    Verify(u8, bool),
}

#[derive(Debug)]
enum State {
    Idle,
    WaitingForIndexToWrite,
    WritingTrack,
    WaitingForIndexToVerify,
    VerifyingTrack,
}

#[self_referencing]
struct ReadingVec {
    data: Vec<u8>,
    #[borrows(data)]
    #[covariant]
    pos: Iter<'this, u8>,
}

pub(crate) struct MfmTrackHandler {
    state: State,
    pub order_queue: VecDeque<Order>,
    pub result_queue: VecDeque<Response>,
    track_data: Option<ReadingVec>,
}

impl MfmTrackHandler {
    pub fn new() -> MfmTrackHandler {
        MfmTrackHandler {
            state: State::Idle,
            order_queue: VecDeque::new(),
            result_queue: VecDeque::new(),
            track_data: None,
        }
    }

    pub fn index(&mut self) {
        match self.state {
            State::WaitingForIndexToWrite => self.state = State::WritingTrack,
            State::WaitingForIndexToVerify => self.state = State::VerifyingTrack,
            _ => {}
        }
    }

    pub fn run<'a, T>(&'a mut self, mut out: T)
    where
        T: FnMut(u8),
    {
        self.state = match self.state {
            State::Idle => {
                let x = self.order_queue.borrow_mut().pop_front();

                if let Some(Order::Write(y, x)) = x {
                    self.track_data = Some(
                        ReadingVecBuilder {
                            data: x,
                            pos_builder: |int_data| int_data.iter(),
                        }
                        .build(),
                    );

                    State::WaitingForIndexToWrite
                } else {
                    State::Idle
                }
            }
            State::WaitingForIndexToWrite => {
                /* Do nothing */
                State::WaitingForIndexToWrite
            }

            State::WritingTrack => {
                self.track_data.as_mut().unwrap().with_pos_mut(|f| {
                    //let x = self.track_data.as_ref().unwrap().pos.unwrap().next();
                    if let Some(y) = f.next() {
                        out(*y);
                        State::WritingTrack
                    } else {
                        State::VerifyingTrack
                    }
                })
            }
            State::WaitingForIndexToVerify => {
                /* Do nothing */
                State::WaitingForIndexToVerify
            }

            State::VerifyingTrack => todo!(),
        }
        /*
        if self.state == State::WritingTrack {
            if let Some(x) = self.order_queue.borrow_mut().pop_front() {
                mfme.feed(inval)
            }
        }
        */
    }

    pub fn feed(&mut self, data: MfmResult2) {}
}
