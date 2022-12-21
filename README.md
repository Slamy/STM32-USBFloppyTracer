# STM32-USBFloppyTracer

Raw Floppy Writer using STM32F4Discovery board. Supports Amiga and C64 Images. Even copy protected!
This is a remake of my [older project](https://github.com/Slamy/SlamySTM32Floppy) as I wanted to learn Rust and needed a project to do so.
Also I don't like my old code base any more and wanted to have a cleaner software architecture.

This project is not created to encourage software piracy. It should be seen as a way of reconstructing damaged disks to repair a collection.

## Features

* Writes and verifies raw tracks
* Write Precompensation
    * Configuration for single tracks possible
    * Semi-automatic calibration process
* Supported disk image formats
    * .adf
    * .ipf
    * .d64
    * .g64
    * .st
    * .stx (Experimental)

## Compatibility and Differences to old project

This project is 100% pin compatible with the [older version](https://github.com/Slamy/SlamySTM32Floppy).
There is no need to resolder the old setup.

### Improvements to the old project:

* USB transfer and Floppy writing is done in parallel. Faster process.
* Every track now verified after being written. Even raw images with copy protections.
* The project is rewritten in Rust.

### Features still missing from the old project:

* Reading of disk images
* Flippy disks (Index hole simulation)

## How to connect the STM32F4Discovery board to the disk drive

![Pinout diagram of STM32F4Discovery board with floppy signals](doc/pinout.png)

## How to build and flash the firmware

	cd firmware
	cargo embed --release

## How to build and install the tool

	cargo build --release
	cargo install --path tool/

## Why not use the Greaseweazle or the Kryoflux?

I had to ask myself this question during the start of this project in fall 2022. My [SlamySTM32Floppy](https://github.com/Slamy/SlamySTM32Floppy) was never changed since 2018. And even that year is wrong as the project matured during 2016 but wasn't directly uploaded at that time. It is now a long time ago and new players have entered the match. The [Greaseweazle](https://github.com/keirf/greaseweazle) seems to be a very affordable solution for most users and can be bought preassembled and ready to go for a low price.
Even older but maybe also sufficient is the [Kryoflux](https://kryoflux.com/). But that device is rather pricey for some people.

In the end, I just love floppy disks and wanted to use this project to learn Rust but also improve the software architecture of the old project. So I decided to give it another shot.

## Usage

Help is provided by the tool itself

    usbfloppytracer -h

Assuming drive A is a 3.5" drive: Writing of Amiga images

    usbfloppytracer -a Turrican.adf
    usbfloppytracer -a Turrican.ipf

Assuming drive B is a 5.25" drive: Writing of C64 images

    usbfloppytracer -b Katakis_s1.g64
    usbfloppytracer -b Katakis_s1.d64

For proper write precompensation, another [document](doc/write_precompensation.md) was added to explain the process.

## Copy protected images which have been checked and are supported by this tool

Quality can vary between raw images. Writing and verification of .g64 and .ipf images is not guaranteed.
Writing of .stx images sometimes requires lots of patching as quality between images varies.
Therefore I try to keep a list of images which are expected to work with this software.

| Name                                             | MD5                              | Notes                                     | Copy Protection Method                 |
|--------------------------------------------------|----------------------------------|-------------------------------------------|----------------------------------------|
| Apidya (Germany) (En) (Disk 1).ipf               | 3adf2ffa5fbf740515576c10f46e1a67 |                                           | Long Tracks                            |
| EnchantedLand.ipf                                | d907e262b6a3a72e0c690216bb9d0290 |                                           |                                        |
| Gods_Disc1.ipf                                   | 7b2a11eda49fc6841834e792dab53997 |                                           |                                        |
| Jim Power in Mutant Planet (Europe) (Disk 1).ipf | 78b2a03c31a30aadbcb269e75ae94853 |                                           |                                        |
| Jumping Jack'Son (Europe).ipf                    | b4106a4ae184f5547d87be0601c71c9e |                                           |                                        |
| Katakis (Side 1).g64                             | 53c47c575d057181a1911e6653229324 | Created with nibconv from .nib image      | Rainbow Arts (RADWAR)                  |
| Katakis (Side 1).nib                             | 63fcfea043054882cfc31ae43fd0a5f9 | ./nibconv -r katakis_s1.nib katakis_s1.g64| Rainbow Arts (RADWAR)                  |
| Rodland (Europe) (v1.32).ipf                     | 5bf77241b8ce88a323010e82bf18f3e0 |                                           | Rob Northen copylock?                  |
| Turrican2.ipf                                    | 17abf9d8d5b2af451897f6db8c7f4868 | Might require write precompensation       | Long Tracks                            |
| Turrican III - Payment Day (Germany).ipf         | e471c215d5c58719aeec1172b6e2b0e5 |                                           | Long Tracks                            |
| Turrican.ipf                                     | 654e52bec1555ab3802c21f6ea269e64 |                                           | Long Tracks                            |
| X-Out_1.ipf                                      | 1784c149245dfecde23223dc217604b0 |                                           | Long Tracks                            |
| Z-Out (Europe).ipf                               | 0ff89947aede0817f443712d3689f503 |                                           | Long Tracks                            |
| Turrican II (1991)(Rainbow Arts).stx             | fb96a28ad633208a973e725ceb67c155 |                                           | Long Tracks (worse than first game?)   |
| Turrican (1990)(Rainbow Arts).stx                | 4865957cd83562547a722c95e9a5421a |                                           | Sector in Sector, No Flux Reversal Area|


## Copy protected images which have been known for NOT working with this tool

This list doesn't mean that these images won't be supported in the future.
It is mostly a TODO list for me and a hint for others who are struggling reconstructing this particular disk.


| Name                                             | MD5                              | Notes                                   | Copy Protection Method|
|--------------------------------------------------|----------------------------------|-----------------------------------------|-----------------------|
|                                                  |                                  |                                         |                       |

