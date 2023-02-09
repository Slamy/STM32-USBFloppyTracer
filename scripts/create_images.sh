# 80 * 2 * 15
dd if=/dev/zero of=empty_hd_5_25.img bs=2400 count=512

# 80 * 2 * 18
dd if=/dev/zero of=empty_hd_3_5.img bs=2880 count=512

# Only one size allowd
dd if=/dev/zero of=empty.d64 bs=174848 count=1

# 80 * 2 * 11
dd if=/dev/zero of=empty.adf bs=1760 count=512

# 80 * 2 * 9
dd if=/dev/zero of=empty.st bs=1440 count=512
