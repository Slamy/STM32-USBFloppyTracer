set -e

# 80 * 2 * 15 with FAT12
dd if=/dev/zero of=empty_hd_5_25.img bs=2400 count=512
/sbin/mkfs.msdos empty_hd_5_25.img

# 80 * 2 * 18 with FAT12
dd if=/dev/zero of=empty_hd_3_5.img bs=2880 count=512
/sbin/mkfs.msdos empty_hd_3_5.img

# Only one size allowed
dd if=/dev/zero of=empty.d64 bs=174848 count=1

# 80 * 2 * 11
dd if=/dev/zero of=empty.adf bs=1760 count=512

# 80 * 2 * 9
dd if=/dev/zero of=empty.st bs=1440 count=512
