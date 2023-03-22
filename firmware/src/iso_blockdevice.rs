use core::cell::RefCell;
use core::future::Future;
use core::task::Poll;

use crate::interrupts::{
    self, async_select_and_wait_for_track, flux_reader_stop_reception, FLUX_READER,
};
use crate::scsi_class::BlockDevice;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use async_trait::async_trait;
use cassette::futures::poll_fn;
use heapless::spsc::{Consumer, Producer};
use rtt_target::rprintln;
use usb_device::Result;
use util::fluxpulse::FluxPulseToCells;
use util::mfm::{MfmDecoder, MfmWord, ISO_DAM, ISO_IDAM, ISO_SYNC_BYTE};
use util::{Cylinder, Head, PulseDuration, Track};

struct CacheEntry {
    lba: u32,
    data: Vec<u8>,
}

pub struct CollectedSector {
    index: u32,
    payload: Vec<u8>,
}

pub struct IsoBlockDevice {
    cache: VecDeque<CacheEntry>,
    read_cons: Consumer<'static, u32, 512>,
    write_prod_cell: RefCell<Producer<'static, u32, 128>>,
}

const CYLINDERS: u32 = 80;
const HEADS: u32 = 2;
const SECTORS_PER_TRACK: u32 = 18;
const SECTORS_PER_CYLINDER: u32 = SECTORS_PER_TRACK * HEADS;
const SECTORS_PER_DISK: u32 = CYLINDERS * SECTORS_PER_CYLINDER;

impl IsoBlockDevice {
    pub fn new(
        read_cons: Consumer<'static, u32, 512>,
        write_prod_cell: RefCell<Producer<'static, u32, 128>>,
    ) -> Self {
        IsoBlockDevice {
            cache: VecDeque::new(),
            read_cons,
            write_prod_cell,
        }
    }

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

    async fn read_lba(&mut self, lba: u32) {
        let cylinder = lba / SECTORS_PER_CYLINDER;
        let cylinder_offset = lba % SECTORS_PER_CYLINDER;
        let head = cylinder_offset / SECTORS_PER_TRACK;
        let _sector = cylinder_offset % SECTORS_PER_TRACK;

        let lba_first_sector = cylinder * SECTORS_PER_CYLINDER + head * SECTORS_PER_TRACK;

        let collected_sectors = self.read_track(cylinder, head).await;

        for i in collected_sectors {
            let lba = lba_first_sector + i.index;
            self.cache.push_back(CacheEntry {
                lba,
                data: i.payload,
            });

            if self.cache.len() > 180 {
                self.cache.pop_front();
            }
        }
    }

    async fn read_track(&mut self, cylinder: u32, head: u32) -> Vec<CollectedSector> {
        rprintln!("Read track {} {}", cylinder, head);
        // keep the motor spinning
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // empty queue before starting to work
        while self.read_cons.dequeue().is_some() {}

        async_select_and_wait_for_track(Track {
            cylinder: Cylinder(cylinder as u8),
            head: Head(head as u8),
        })
        .await;

        let mfm_words: RefCell<VecDeque<MfmWord>> = RefCell::new(VecDeque::with_capacity(200));
        let mut mfmd = MfmDecoder::new(|f| mfm_words.borrow_mut().push_back(f));
        let mut pulseparser = FluxPulseToCells::new(|val| mfmd.feed(val), 84);
        let mut sector_header = Vec::with_capacity(20);
        let mut collected_sectors: Vec<CollectedSector> = Vec::new();

        cortex_m::interrupt::free(|cs| {
            FLUX_READER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .start_reception(cs);
        });

        // TODO This is still a problem. the first 2 values are too high. clean up a bit
        self.async_read_flux().await.unwrap();
        self.async_read_flux().await.unwrap();
        self.async_read_flux().await.unwrap();

        let mut awaiting_dam = 0;

        loop {
            // Wait for IDAM
            let duration = self.async_read_flux().await.unwrap();

            pulseparser.feed(PulseDuration(duration));

            while let Some(word) = {
                // TODO this is very weird
                // for some reason i need an extra scope to release the mutable borrow.
                // this shouldn't be... Is this a Rust bug?
                let x = mfm_words.borrow_mut().pop_front();
                x
            } {
                awaiting_dam -= 1;

                if matches!(word, MfmWord::SyncWord) {
                    // check if we have received an IDAM

                    while mfm_words.borrow().len() < 10 {
                        if let Some(pulse) = self.read_cons.dequeue() {
                            pulseparser.feed(PulseDuration(pulse as i32));
                        }
                    }

                    let address_mark_type = mfm_words.borrow_mut().pop_front().unwrap();

                    match address_mark_type {
                        MfmWord::Enc(ISO_IDAM) => {
                            sector_header.clear();

                            for _ in 0..6 {
                                if let Some(MfmWord::Enc(val)) = mfm_words.borrow_mut().pop_front()
                                {
                                    sector_header.push(val);
                                }
                            }
                            let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
                            crc.update(&[ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_IDAM]);
                            crc.update(&sector_header);
                            let crc16 = crc.get();

                            if crc16 == 0 {
                                assert_eq!(sector_header[0] as u32, cylinder);
                                assert_eq!(sector_header[1] as u32, head);

                                if collected_sectors
                                    .iter()
                                    .any(|f| f.index == u32::from(sector_header[2]) - 1)
                                {
                                    rprintln!("Already got sector header {:?}", sector_header);
                                } else {
                                    // Activate DAM reading for the next 43 data bytes
                                    // rprintln!("Awaiting sector header {:?}", sector_header);

                                    awaiting_dam = 43;
                                }
                            } else {
                                rprintln!("Invalid Header CRC {:?}", sector_header);
                            }
                        }

                        MfmWord::Enc(ISO_DAM) if awaiting_dam > 0 => {
                            // Some(MfmWord::Enc(ISO_DAM)) => {
                            let sector_size = 128 << sector_header[3];
                            let mut sector_data = Vec::with_capacity(sector_size + 2);

                            let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
                            crc.update(&[ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_DAM]);

                            for _ in 0..sector_size + 2 {
                                while mfm_words.borrow().len() < 5 {
                                    if let Some(pulse) = self.read_cons.dequeue() {
                                        pulseparser.feed(PulseDuration(pulse as i32));
                                    }
                                }

                                if mfm_words.borrow().len() > 70 {
                                    cassette::yield_now().await;
                                }

                                if let Some(x) = mfm_words.borrow_mut().pop_front() {
                                    if let MfmWord::Enc(val) = x {
                                        sector_data.push(val);
                                        crc.update(&[val]);
                                    } else {
                                        panic!("{:?}", x);
                                    }
                                }
                            }

                            let crc16 = crc.get();

                            if crc16 == 0 {
                                //let lba = lba_first_sector + sector_header[2] as u32 - 1;
                                assert_eq!(sector_header[3] as u32, 2); // 512 byte sectors for now

                                /*
                                rprintln!(
                                    "Got sector data {:?} {}",
                                    sector_header,
                                    //lba,
                                    awaiting_dam
                                );
                                */
                                sector_data.resize(sector_size, 0); // remove CRC at the end
                                collected_sectors.push(CollectedSector {
                                    index: (sector_header[2] as u32) - 1,
                                    payload: sector_data,
                                });

                                if collected_sectors.len() == SECTORS_PER_TRACK as usize {
                                    flux_reader_stop_reception();
                                    // Throw away remaining data
                                    while self.read_cons.dequeue().is_some() {}

                                    return collected_sectors;
                                }
                            } else {
                                rprintln!("Invalid Header CRC {:?}", sector_header);
                            }
                        }
                        MfmWord::Enc(ISO_DAM) => {
                            //rprintln!("Unexpected sector data!");
                        }
                        _ => {}
                    }

                    //rprintln!("{:x?}", sector_header);
                }
            }
        }
    }
}

#[async_trait(?Send)]
impl BlockDevice for IsoBlockDevice {
    fn medium_present(&self) -> bool {
        true
    }

    fn max_lba(&self) -> u32 {
        SECTORS_PER_DISK - 1
    }

    async fn read_block(&mut self, lba: u32) -> Option<Vec<u8>> {
        // check the cache
        if let Some(block) = self.cache.iter().find(|f| f.lba == lba) {
            //rprintln!("Read {} from cache", lba);
            return Some(block.data.clone()); // TODO avoid clone
        }

        //rprintln!("Read {} from disk", lba);
        self.read_lba(lba).await;

        // check the cache again
        if let Some(block) = self.cache.iter().find(|f| f.lba == lba) {
            //rprintln!("Got {} now from cache", lba);
            return Some(block.data.clone()); // TODO avoid clone
        }

        panic!();
        //None
    }

    async fn write_block(&mut self, _lba: u32, _block: &[u8]) -> Result<()> {
        rprintln!("Write");
        // Ok(());
        todo!();
    }
}
