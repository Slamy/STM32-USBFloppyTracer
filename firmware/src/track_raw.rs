use core::{cell::RefCell, cmp::max, future::Future, task::Poll};

use alloc::{collections::VecDeque, vec::Vec};
use cassette::futures::poll_fn;
use heapless::spsc::{Consumer, Producer};

use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator, PulseDuration, RawCellData, Track,
};

use crate::{
    interrupts::{
        self, async_select_and_wait_for_track, async_wait_for_index, async_wait_for_transmit,
        FLUX_READER, START_RECEIVE_ON_INDEX, START_TRANSMIT_ON_INDEX,
    },
    orange, rprintln,
    usb::UsbHandler,
};

pub struct RawTrackHandler {
    pub read_cons: Consumer<'static, u32, 512>,
    pub write_prod_cell: RefCell<Producer<'static, u32, 128>>,
    pub track_data_to_write: Option<RawCellData>,
}

#[derive(Debug)]
pub enum RawTrackError {
    NoIndexPulse,
    NoIncomingData,
    NoCrossCorrelation,
    DataNotEqual,
}

impl RawTrackHandler {
    fn async_read_flux(&mut self) -> impl Future<Output = Option<i32>> + '_ {
        poll_fn(move |_| {
            if let Some(pulse_duration) = self.read_cons.dequeue() {
                Poll::Ready(Some(pulse_duration as i32))
            } else {
                let motor_is_spinning = cortex_m::interrupt::free(|cs| {
                    interrupts::FLOPPY_CONTROL
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .is_spinning()
                });

                if motor_is_spinning {
                    Poll::Pending
                } else {
                    rprintln!("async_read_flux timeout!");
                    Poll::Ready(None)
                }
            }
        })
    }

    pub async fn write_and_verify(
        &mut self,
        track: Track,
        first_significance_offset: usize,
        write_precompensation: PulseDuration,
    ) -> Result<(u8, u8, PulseDuration, PulseDuration), (u8, u8)> {
        async_select_and_wait_for_track(track).await;

        let mut write_operations = 0;
        let mut verify_operations = 0;

        for _ in 0..5 {
            rprintln!(
                "Write track at cyl:{} head:{} sigoff:{}",
                track.cylinder.0,
                track.head.0,
                first_significance_offset
            );
            write_operations += 1;
            if matches!(self.write_track(write_precompensation).await, Err(())) {
                return Err((write_operations, verify_operations));
            }

            for read_try in 0..3 {
                verify_operations += 1;

                let verify_result = self.verify_track(first_significance_offset).await;
                match verify_result {
                    Ok(max_err) => {
                        return Ok((
                            write_operations,
                            verify_operations,
                            max_err,
                            write_precompensation,
                        ));
                    }
                    Err(RawTrackError::DataNotEqual) => {
                        // We shall do nothing. Maybe it was a fluke?
                        // Just read again...
                    }
                    Err(RawTrackError::NoCrossCorrelation) if read_try == 0 => {
                        // This happens sometimes. Nothing to worry about.
                        // This usually occurs with longer tracks as the read head
                        // must recalibrate.
                        // Just read again...
                    }
                    Err(RawTrackError::NoCrossCorrelation) => {
                        // Ok now this is bad.
                        // Abort reading and write again. This won't get any better.
                        // This can occur if the write process overwrites the start of the track.
                        // A fluctuation in the rotation speed causes this.
                        break;
                    }
                    Err(RawTrackError::NoIncomingData) => {
                        // Abort. Drive not responding
                        return Err((write_operations, verify_operations));
                    }
                    Err(RawTrackError::NoIndexPulse) => {
                        // Abort. Drive not responding
                        return Err((write_operations, verify_operations));
                    }
                }
            }
        }
        Err((write_operations, verify_operations))
    }

    async fn write_track(&mut self, write_precompensation: PulseDuration) -> Result<(), ()> {
        // keep it spinning!
        cortex_m::interrupt::free(|cs| {
            interrupts::FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .clear_buffers();

            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // prefill output buffer
        let track_data_to_write = self.track_data_to_write.take().unwrap();
        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        let mut write_prod_fpg = FluxPulseGenerator::new(
            |f| {
                self.write_prod_cell
                    .borrow_mut()
                    .enqueue(f.0 as u32)
                    .unwrap()
            },
            part.cell_size.0 as u32,
        );

        write_prod_fpg.precompensation = write_precompensation.0 as u32;

        if *track_data_to_write.borrow_has_non_flux_reversal_area() {
            write_prod_fpg.enable_non_flux_reversal_generator = true;
        } else {
            write_prod_fpg.enable_weak_bit_generator = true;
        }

        let mut track_data_iter = part.cells.iter();

        // prefill buffer with first data
        while self.write_prod_cell.borrow().len() < 70 {
            let mfm_byte = *track_data_iter.next().unwrap();
            to_bit_stream(mfm_byte, |bit| write_prod_fpg.feed(bit));
        }

        // start transmit on index pulse
        cortex_m::interrupt::free(|cs| {
            START_TRANSMIT_ON_INDEX.borrow(cs).set(true);
        });

        if let Err(_) = async_wait_for_transmit().await {
            rprintln!("Transmit timeout? Drive not responsing.");
            return Err(());
        }

        // continue until whole track is written.
        // TODO copy pasta
        while let Some(mfm_byte) = track_data_iter.next() {
            assert!(self.write_prod_cell.borrow().len() > 20); // check for underflow

            while self.write_prod_cell.borrow().len() > 70 {
                cassette::yield_now().await;
            }
            to_bit_stream(*mfm_byte, |bit| write_prod_fpg.feed(bit));
        }

        while let Some(part) = parts.next() {
            let mut track_data_iter = part.cells.iter();

            write_prod_fpg.cell_duration = part.cell_size.0 as u32;
            while let Some(mfm_byte) = track_data_iter.next() {
                assert!(self.write_prod_cell.borrow().len() > 20); // check for underflow

                while self.write_prod_cell.borrow().len() > 70 {
                    cassette::yield_now().await;
                }
                to_bit_stream(*mfm_byte, |bit| write_prod_fpg.feed(bit));
            }
        }

        write_prod_fpg.flush();

        self.track_data_to_write = Some(track_data_to_write);
        Ok(())
    }

    pub async fn read_track(
        &mut self,
        track: Track,
        duration_to_record: u32,
        wait_for_index: bool,
        usb_handler: &mut UsbHandler<'_>,
    ) -> Result<(), RawTrackError> {
        // keep the motor spinning
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        while let Some(_) = self.read_cons.dequeue() {}

        async_select_and_wait_for_track(track).await;

        if wait_for_index {
            // Throw away all data in the queue before we read real data
            while let Some(_) = self.read_cons.dequeue() {}

            // start reception of track on next index pulse
            cortex_m::interrupt::free(|cs| {
                START_RECEIVE_ON_INDEX.borrow(cs).set(true);
            });
            if let Err(_) = async_wait_for_index().await {
                return Err(RawTrackError::NoIndexPulse);
            };
        } else {
            cortex_m::interrupt::free(|cs| {
                FLUX_READER
                    .borrow(cs)
                    .borrow_mut()
                    .as_mut()
                    .unwrap()
                    .start_reception(cs);
            });
        }
        orange(true);
        let mut collect_buffer: Vec<u8> = Vec::with_capacity(64);
        let mut usb_frames_transferred = 0;
        let mut usb_frames_collected = 0;

        let mut buffers: VecDeque<Vec<u8>> = VecDeque::new();
        let mut max_slack = 0;

        let mut timeout = 0;
        let mut duration_yet_recorded = 0;
        let mut required_duration_was_recorded = false;

        // Throw away the first 2 pulses.
        // For yet unknown reasons the first two are garbage.
        // TODO Are they coming from the DMA?
        if self.async_read_flux().await.is_none() {
            cortex_m::interrupt::free(|cs| {
                let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                let y2 = fr1.as_mut().unwrap();
                y2.stop_reception(cs);
            });
            return Err(RawTrackError::NoIncomingData);
        }
        self.async_read_flux().await;

        while usb_frames_transferred < usb_frames_collected
            || required_duration_was_recorded == false
        {
            // Some data to send?
            if let Some(front) = buffers.front() {
                if let Ok(size) = usb_handler.write(&front) {
                    assert_eq!(size, 64);

                    max_slack = max_slack.max(buffers.len());
                    buffers.pop_front();
                    usb_frames_transferred += 1;
                }
                usb_handler.handle();
            }

            if let Some(pulse) = self.read_cons.dequeue() {
                timeout = 0;
                duration_yet_recorded += pulse;
                // TODO magic number
                let mut reduced_pulse = pulse >> 3;

                if pulse & 0b100 != 0 {
                    //round up
                    reduced_pulse += 1;
                }

                if reduced_pulse > 0xff {
                    reduced_pulse = 0xff;
                }

                collect_buffer.push(reduced_pulse as u8);

                if collect_buffer.len() == 64 {
                    let new_buffer: Vec<u8> = Vec::with_capacity(64);
                    let old_buffer = core::mem::replace(&mut collect_buffer, new_buffer);
                    buffers.push_back(old_buffer);

                    usb_frames_collected += 1;

                    if duration_yet_recorded >= duration_to_record {
                        required_duration_was_recorded = true;
                        cortex_m::interrupt::free(|cs| {
                            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                            let y2 = fr1.as_mut().unwrap();
                            y2.stop_reception(cs);
                        });
                        orange(false);
                        // Throw away remaining data
                        while let Some(_) = self.read_cons.dequeue() {}
                    }
                }
            } else {
                timeout += 1;
                // TODO magic number
                if timeout == 0x800_000 {
                    cortex_m::interrupt::free(|cs| {
                        let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                        let y2 = fr1.as_mut().unwrap();
                        y2.stop_reception(cs);
                    });
                    // Throw away remaining data
                    while let Some(_) = self.read_cons.dequeue() {}
                    return Err(RawTrackError::NoIncomingData);
                }
            }
        }

        // Send empty end package
        loop {
            if let Ok(size) = usb_handler.write(&[0; 0]) {
                assert_eq!(size, 0);
                break;
            }
            usb_handler.handle();
        }

        rprintln!(
            "{} {} Collected {} {} blocks! {}   {} {}",
            track.cylinder.0,
            track.head.0,
            usb_frames_transferred,
            usb_frames_collected,
            max_slack,
            duration_yet_recorded,
            duration_to_record
        );

        Ok(())
    }

    async fn verify_track(
        &mut self,
        first_significance_offset: usize,
    ) -> Result<PulseDuration, RawTrackError> {
        // Size of sliding window, containing the significant data we use, trying
        // to match the data we read back against the groundtruth data we thought
        // to have written before
        const COMPARE_WINDOW_SIZE: usize = 20;

        // We record this amount of pulses to slide the COMPARE_WINDOW on
        // to perfom cross correlation
        const READ_DATA_WINDOW_SIZE: usize = 90;

        // keep the motor spinning
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // Throw away all data in the queue before we read real data
        while let Some(_) = self.read_cons.dequeue() {}

        // grab the ownership inside this async function to avoid issues with borrowing
        let track_data_to_write = self.track_data_to_write.take().unwrap();

        // we might have multiple different cell densities. grab the first one
        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        // How similar should the data be against the reference?
        // The minimum similarity is half of the bit cell. But we are better than that!
        // 35% should be ok!
        let similarity_treshold = part.cell_size.0 as i32 * 35 / 100;

        // prepare compare data around the first significant position to compare the data we read back to
        let flux_data_to_write_queue: RefCell<VecDeque<PulseDuration>> = RefCell::new(
            VecDeque::with_capacity(first_significance_offset + COMPARE_WINDOW_SIZE * 2),
        );
        let mut flux_data_to_write_fpg = FluxPulseGenerator::new(
            |f| flux_data_to_write_queue.borrow_mut().push_back(f),
            part.cell_size.0 as u32,
        );

        if *track_data_to_write.borrow_has_non_flux_reversal_area() {
            // It is important to have the non flux reversal generator disabled here.
            // We will be reading an area of nothing after all!
            flux_data_to_write_fpg.enable_non_flux_reversal_generator = false;
        } else {
            flux_data_to_write_fpg.enable_weak_bit_generator = true;
        }

        let mut track_data_to_write_iter = part.cells.iter();
        while flux_data_to_write_queue.borrow().len()
            < first_significance_offset + COMPARE_WINDOW_SIZE
        {
            to_bit_stream(
                *track_data_to_write_iter.next().unwrap_or_else(|| {
                    panic!("Not filled {}", flux_data_to_write_queue.borrow().len())
                }),
                |bit| flux_data_to_write_fpg.feed(bit),
            )
        }

        // now drain the generated data until we are left with only the compare window
        // for this, we usually need at least have more data than half of the compare window.
        if first_significance_offset > COMPARE_WINDOW_SIZE / 2 {
            flux_data_to_write_queue
                .borrow_mut()
                .drain(0..first_significance_offset - COMPARE_WINDOW_SIZE / 2);
        } else {
            // For Apydia we have to do a different approach.
            // It is very risky to do this as we reduce the time to settle on incoming data
            // But if there is no other option, we must hope that the index pulse is exact...
            flux_data_to_write_queue.borrow_mut().drain(0..2);
        }

        // reserve some memory for reading flux data from disk
        let mut read_mfm_flux_data_queue: VecDeque<PulseDuration> = VecDeque::with_capacity(1000);

        // start reception of track on next index pulse
        cortex_m::interrupt::free(|cs| {
            START_RECEIVE_ON_INDEX.borrow(cs).set(true);
        });

        if let Err(_) = async_wait_for_index().await {
            self.track_data_to_write = Some(track_data_to_write);
            return Err(RawTrackError::NoIndexPulse);
        };

        // throw away first pulses before the point of significance
        // as we can't verify those pulses.
        if first_significance_offset > 10 {
            let mut pulses_to_throw_away = first_significance_offset - 10;
            while pulses_to_throw_away > 0 {
                let pulse = self.async_read_flux().await;
                if pulse.is_none() {
                    rprintln!("Timeout1");
                    cortex_m::interrupt::free(|cs| {
                        let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                        let y2 = fr1.as_mut().unwrap();
                        y2.stop_reception(cs);
                    });
                    self.track_data_to_write = Some(track_data_to_write);
                    return Err(RawTrackError::NoIncomingData);
                };

                pulses_to_throw_away -= 1;
            }
        }

        // now record something slightly larger than the "significant window"
        while read_mfm_flux_data_queue.len() < READ_DATA_WINDOW_SIZE {
            if let Some(pulse) = self.async_read_flux().await {
                read_mfm_flux_data_queue.push_back(PulseDuration(pulse))
            } else {
                rprintln!("Timeout2");

                cortex_m::interrupt::free(|cs| {
                    let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                    let y2 = fr1.as_mut().unwrap();
                    y2.stop_reception(cs);
                });
                self.track_data_to_write = Some(track_data_to_write);
                return Err(RawTrackError::NoIncomingData);
            };
        }

        let mut equal = false; // set to true if correlation is found
        let mut match_after_pulses = 0;
        // now move the reference significant window over the already read data and compare it.
        // there should be one position where it matches!
        for read_window_index in 0..READ_DATA_WINDOW_SIZE {
            if read_mfm_flux_data_queue.len() < COMPARE_WINDOW_SIZE {
                rprintln!("Unable to cross correlate!");

                cortex_m::interrupt::free(|cs| {
                    let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                    let y2 = fr1.as_mut().unwrap();
                    y2.stop_reception(cs);
                });
                self.track_data_to_write = Some(track_data_to_write);
                return Err(RawTrackError::NoCrossCorrelation);
            }
            equal = read_mfm_flux_data_queue
                .range(0..COMPARE_WINDOW_SIZE)
                .zip(flux_data_to_write_queue.borrow().iter())
                .all(|(x, y)| y.similar(x, similarity_treshold));

            if equal {
                match_after_pulses = read_window_index;
                break;
            }

            read_mfm_flux_data_queue.pop_front();
        }

        assert!(equal); // program flow check

        // We are now synchronized and shall compare upcoming data
        let mut maximum_diff = 0;
        let mut successful_compares = 0;

        loop {
            // read more data from the incoming flux stream and
            // put it into our compare buffer
            if read_mfm_flux_data_queue.len() < 30 {
                if let Some(pulse) = self.read_cons.dequeue() {
                    read_mfm_flux_data_queue.push_back(PulseDuration(pulse as i32))
                }
            }

            // generate more pulse data from the groundtruth data
            if flux_data_to_write_queue.borrow().len() < 30 {
                if let Some(val) = track_data_to_write_iter.next() {
                    to_bit_stream(*val, |bit| flux_data_to_write_fpg.feed(bit))
                } else {
                    if let Some(part) = parts.next() {
                        flux_data_to_write_fpg.cell_duration = part.cell_size.0 as u32;

                        track_data_to_write_iter = part.cells.iter();
                    } else {
                        flux_data_to_write_fpg.flush();
                    }
                }
            }

            // we have no more data to compare too.
            if flux_data_to_write_queue.borrow().is_empty() {
                break; // Yay! All is verified.
            }

            if !read_mfm_flux_data_queue.is_empty() && !flux_data_to_write_queue.borrow().is_empty()
            {
                let reference = flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
                let readback = read_mfm_flux_data_queue.pop_front().unwrap();

                if reference.0 > part.cell_size.0 * 10 {
                    // Non Flux Reversal Detected. Some cleanup needed.
                    // TODO Is this really the best approach to fix this?
                    // It is also pretty random. Sometimes it doesn't work at all.
                    flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
                } else if !reference.similar(&readback, similarity_treshold) {
                    rprintln!(
                        "{} != {}, successful_compares until compare fail: {}",
                        reference.0,
                        readback.0,
                        successful_compares
                    );

                    cortex_m::interrupt::free(|cs| {
                        let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                        let y2 = fr1.as_mut().unwrap();
                        y2.stop_reception(cs);
                    });
                    self.track_data_to_write = Some(track_data_to_write);

                    return Err(RawTrackError::DataNotEqual);
                } else {
                    maximum_diff = max(maximum_diff, (reference.0).abs_diff(readback.0));
                }
                successful_compares += 1;
            }
        }

        // stop the reception to avoid DMA overflows
        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.stop_reception(cs);
        });

        rprintln!(
            "Verified {} pulses, Max error {} / {}, window match offset {}",
            successful_compares,
            maximum_diff,
            similarity_treshold,
            match_after_pulses
        );
        Ok(PulseDuration(maximum_diff as i32))
    }
}
