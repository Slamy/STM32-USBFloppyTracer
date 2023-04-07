# STM32-USBFloppyTracer

Raw floppy writer and reader using the STM32F407. Supports Amiga and C64 Images. Even copy protected!
This is a remake of my [older project](https://github.com/Slamy/SlamySTM32Floppy) as I wanted to learn Rust and needed a project to do so.
Also I don't like my old code base any more and wanted to have a cleaner software architecture.

This project is not created to encourage software piracy. It should be seen as a way of reconstructing damaged disks to repair a collection.

## Features

* Writes and verifies raw tracks
* [Write Precompensation](doc/write_precompensation.md)
    * Configuration with cylinder precision
    * Semi-automatic calibration process
* Supported disk image formats for writing
    * .adf
    * .ipf
    * .d64
    * .g64
    * .dsk
    * .st
    * .stx (Highty experimental, only [patched images](doc/compatibility_list.md))
    * .img (Typical DOS disk)
* Supported disk image formats for reading
    * .adf
    * .st
    * .img
    * .d64
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


At the moment two boards are known to work.
The bigger STM32F4Discovery was used to start this project.

[Pinout diagram of STM32F4Discovery board with floppy signals](doc/pinout/discovery.png)

I've recently switched over to the smaller Diymore STM32F4 board.
The pin assigment hasn't changed to keep the software compatible.

[Pinout diagram of Diymore STM32F4 board with floppy signals](doc/pinout/diymore.png)


## Prerequisites for building this project

Submodules must be synchronized if not yet done.

    git submodule init
    git submodule sync
    git submodule update

This project requires an external library for parsing IPF images.
To build it, install CMake and a GNU Compiler toolchain.

### GNU Compiler Toolchain for Windows

One option which I've tested is [LLVM-MinGW](https://www.mingw-w64.org/downloads/#llvm-mingw)
For CMake just install it from the [website](https://cmake.org/download/).

### UDev Rules for linux

Without additional udev rules, only the root user has access to the USB device.
Install them like this, to allow normal users as well.

	sudo cp udev/99-usbfloppytracer.rules /etc/udev/rules.d/
	sudo udevadm control -R

### Rust

If rust is not installed yet, it is suggested to install it using [rustup](https://www.rust-lang.org/tools/install).
Don't install rust using the package manager as one might get only older versions of rust.

This project is not compatible with the currently stable version of rust. The nightly must be selected.
This is however only a matter of time until certain features reach the stable version.

    rustup default nightly

Ensure that your rust is up-to-date:

    rustup update nightly

If not yet performed, the target of the microcontroller must be added to the rust environment

    rustup target add thumbv7em-none-eabihf

Install cargo-embed as it is used for flashing:

    cargo install cargo-embed

## How to build and flash the firmware

    cd firmware
    cargo embed --release

## How to build and install the tool

    cargo build --release
    cargo install --path cli
    cargo install --path gui

## Why not use the Greaseweazle or the Kryoflux?

I had to ask myself this question during the start of this project in fall 2022. My [SlamySTM32Floppy](https://github.com/Slamy/SlamySTM32Floppy) was never changed since 2018. And even that year is wrong as the project matured during 2016 but wasn't directly uploaded at that time. It is now a long time ago and new players have entered the match. The [Greaseweazle](https://github.com/keirf/greaseweazle) seems to be a very affordable solution for most users and can be bought preassembled and ready to go for a low price.
Even older but maybe also sufficient is the [Kryoflux](https://kryoflux.com/). But that device is rather pricey for some people.

In the end, I just love floppy disks and wanted to use this project to learn Rust but also improve the software architecture of the old project. So I decided to give it another shot.

## Usage

Some help is provided by the tool itself:

    usbfloppytracer -h

### Writing images to disk

Assuming drive A is a 3.5" drive:

    usbfloppytracer -a image.adf
    usbfloppytracer -a image.ipf
    usbfloppytracer -a image.st
    usbfloppytracer -a image.stx
    usbfloppytracer -a image.dsk
    usbfloppytracer -a image.img # Expected to be an ISO / IBM image

Assuming drive B is a 5.25" drive:

    usbfloppytracer -b image.g64
    usbfloppytracer -b image.d64
    usbfloppytracer -b image.img # Expected to be an ISO / IBM image

It's possible to specify which tracks shall be written. The cylinders start
counting with 0 and the filter is inclusive.

    usbfloppytracer -a empty.adf -t8   # Write only cylinder 8 on both heads
    usbfloppytracer -a empty.adf -t8:0 # Write only cylinder 8 on head 0
    usbfloppytracer -a empty.adf -t8:1 # Write only cylinder 8 on head 1
    usbfloppytracer -a empty.adf -t-3  # Write cylinders 0 to 3 (4 cylinders)
    usbfloppytracer -a empty.adf -t70- # Write cylinders 70 to end of image

### Reading from disk to image

This tool can't be used to create copy protected masters for writing.
It should be noted that the number of tracks is not analyzed and can result in a shorter image
than expected. This can be especially a problem for non standard Atari ST disks.
If in doubt, read more tracks usual. Unformatted tracks will be discarded during reading process.
In case of the ISO format, the number of sectors per track is however checked.

    usbfloppytracer -r -a image.adf
    usbfloppytracer -r -a image.st
    usbfloppytracer -r -b image.d64
    usbfloppytracer -r -a image.img

It's possible to specify which tracks shall be read. The filter is again inclusive.

    usbfloppytracer -r -a image.st -t82 # Read the first 82 cylinders
    usbfloppytracer -r -a image.st -t-2 # Read cylinder 0 to 2 (3 cylinders)
    usbfloppytracer -r -a image.st -t2-3 # Read cylinder 2 to 3 (2 cylinders)

Inspect the disk for the format:

    cargo run --  -r -a discover
    cargo run --  -r -b discover

Just read whatever is there and decide the format for the user.
The name of the image will be the current time and date.
Amiga disks are written to .adf, ISO DD to .st, ISO HD to .img
and C64 disks are written to .d64 files.

    cargo run --  -r -a justread
    cargo run --  -r -b justread

### Write Precompensation

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
