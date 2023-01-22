# STM32-USBFloppyTracer

Raw Floppy Writer using the STM32F407. Supports Amiga and C64 Images. Even copy protected!
This is a remake of my [older project](https://github.com/Slamy/SlamySTM32Floppy) as I wanted to learn Rust and needed a project to do so.
Also I don't like my old code base any more and wanted to have a cleaner software architecture.

This project is not created to encourage software piracy. It should be seen as a way of reconstructing damaged disks to repair a collection.

## Features

* Writes and verifies raw tracks
* [Write Precompensation](doc/write_precompensation.md)
    * Configuration with cylinder precision
    * Semi-automatic calibration process
* Supported disk image formats
    * .adf
    * .ipf
    * .d64
    * .g64
    * .st
    * .stx (Experimental)
* [Flippy Disk Index Simulation](doc/flippy_index.md)
* Supported protections
    * Long Tracks
    * Variable Density (CopyLock - Rob Northen Computing)
    * Weak / Fuzzy bits (Still experimental)
    * Sector in Sector (currently requires patching of STX file)
    * Non Flux Reversal Area (experimental)
* Not yet supported protections
    * Unformatted Area
    * Specific to STX
    	* In Sector Variable density
    	* Hidden data in Gap
    	* Data Tracks

## Compatibility and Differences to old project

This project is 100% pin compatible with the [older version](https://github.com/Slamy/SlamySTM32Floppy).
There is no need to resolder the old setup.

### Improvements to the old project:

* USB transfer and Floppy writing is done in parallel. Faster process.
* Every track now verified after being written. Even raw images with copy protections.
* The project is rewritten in Rust.

### Features still missing from the old project:

* Reading of disk images

## How to connect the STM32F407 board to the disk drive

At the moment two boards are known to work.
The bigger STM32F4Discovery was used to start this project.

[Pinout diagram of STM32F4Discovery board with floppy signals](doc/pinout/discovery.png)

I've recently switched over to the smaller Diymore STM32F4 board.
The pin assigment hasn't changed to keep the software compatible.

[Pinout diagram of Diymore STM32F4 board with floppy signals](doc/pinout/diymore.png)

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

## List of images which have been tested with this project

[Compatible and incompatible disk images](doc/compatibility_list.md)

## Information sources

This project wouldn't have been possible without the information I collected from various sources.

* [Infos about floppy signals](https://retrocmp.de/fdd/general/floppy-bus.htm)
* [Documentation of Turrican copy protection of the Atari ST version](https://github.com/sarnau/AtariSTCopyProtections/blob/master/protection_turrican.md)
* [Atari Floppy Disk Copy Protection - By Jean Louis-Guérin (DrCoolZic)](http://dmweb.free.fr/files/Atari-Copy-Protection-V1.4.pdf)
* [Lots of Info about MFM Encoding and Floppy Drives](http://info-coach.fr/atari/hardware/FD-Hard.php)
* [Technical Details of the ISO Floppy Format](http://info-coach.fr/atari/software/FD-Soft.php)
* [Amiga Floppy Format](http://lclevy.free.fr/adflib/adf_info.html)
* [G64 disk image documentation](http://www.unusedino.de/ec64/technical/formats/g64.html)
* [Api Documentation for IPF reading using libcapsimage](http://www.softpres.org/_media/files:ipfdoc102a.zip?id=download&cache=cache)
* [Pasti file format](http://info-coach.fr/atari/documents/_mydoc/Pasti-documentation.pdf)
* [Inspiration for write precompensation handling](https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/track.py#L41)
* [GCR Encoding of the 1541 floppy drive](http://www.baltissen.org/newhtm/1541c.htm)
