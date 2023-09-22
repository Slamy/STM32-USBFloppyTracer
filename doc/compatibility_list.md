# Copy protected images which have been checked

## Supported by this tool and tested on a real machine

Quality can vary between raw images. Writing and verification of .g64 and .ipf images is not guaranteed.
Writing of .stx images sometimes requires lots of patching as quality between images varies.
Therefore I try to keep a list of images which are expected to work with this software.

### Amiga

| Name                                                                        | MD5                              | Notes                                 | Copy Protection Method |
|-----------------------------------------------------------------------------|----------------------------------|---------------------------------------|------------------------|
| Apidya (Germany) (En) (Disk 1).ipf                                          | 3adf2ffa5fbf740515576c10f46e1a67 |                                       | Long Tracks (1.80 usec)|
| Apprentice.ipf                                                              | 3d5ca39d0fa07feb9d0099f684b2633f |                                       |                        |
| EnchantedLand.ipf                                                           | d907e262b6a3a72e0c690216bb9d0290 |                                       | Long Tracks (1.96 usec)|
| Gods_Disc1.ipf                                                              | 7b2a11eda49fc6841834e792dab53997 |                                       | Variable Density Track |
| James Pond II - Codename RoboCod (Europe).ipf                               | 7ef8e61c300717005e78a1b6494a84d4 |                                       | Long Tracks (1.80 usec)|
| Jim Power in Mutant Planet (Europe) (Disk 1).ipf                            | 78b2a03c31a30aadbcb269e75ae94853 |                                       | Long Tracks (1.89 usec)|
| Jumping Jack'Son (Europe).ipf                                               | b4106a4ae184f5547d87be0601c71c9e |                                       | Long Tracks (1.89 usec)|
| Rodland (Europe) (v1.32).ipf                                                | 5bf77241b8ce88a323010e82bf18f3e0 |                                       | Variable Density Track |
| Turrican2.ipf                                                               | 17abf9d8d5b2af451897f6db8c7f4868 | Might require write precompensation   | Long Tracks (1.81 usec)|
| Turrican III - Payment Day (Germany).ipf                                    | e471c215d5c58719aeec1172b6e2b0e5 |                                       | Long Tracks (1.80 usec)|
| Turrican.ipf                                                                | 654e52bec1555ab3802c21f6ea269e64 |                                       | Long Tracks (1.88 usec)|
| X-Out_1.ipf                                                                 | 1784c149245dfecde23223dc217604b0 | Sync on 0x8455. Nibble with X-Copy    | Custom Sync Word       |
| Z-Out (Europe).ipf                                                          | 0ff89947aede0817f443712d3689f503 | Can be copied with X-Copy             | No Copy Protection?    |
| Lemmings (Europe) (Amiga 500 Bundle - Cartoon Classics).ipf                 | d0d29f214ea57aef2bf1a8dfe508b8ba |                                       | Variable Density Track |
| P.P. Hammer and His Pneumatic Weapon (Europe) (Budget - Global Software).ipf| bd6477aa9a7ac1ff142812d85ed20143 | Can be copied with X-Copy             | No Copy Protection?    |
| Lotus Turbo Challenge 2 (Europe).ipf                                        | ed4321338a4544b6892383cfa2173241 | Also protected by codes in the manual | Long Tracks (1.89 usec)|

### Atari ST

IPF files are the preferred image type here as they actually contain the flux data of the disk.
STX files only contain the interpretation of the flux data by the WD1772 floppy disk controller and
therefore require lots of labor to actually reconstruct a disk from that.

| Name                                                      | MD5                              | Notes           | Copy Protection Method                 |
|-----------------------------------------------------------|----------------------------------|-----------------|----------------------------------------|
| Rick Dangerous.stx                                        | d365e49de69644e386ecb4dcba03509e |                 |                                        |
| Rodland.stx                                               | 80f6322934ca1c76bb04b5c4d6d25097 |                 | CopyLock - Rob Northen Computing       |
| Turrican (1990)(Rainbow Arts).stx                         | 4865957cd83562547a722c95e9a5421a |                 | Sector in Sector, No Flux Reversal Area|
| Turrican II (1991)(Rainbow Arts).stx                      | fb96a28ad633208a973e725ceb67c155 |                 | Long Tracks                            |
| Turrican II - The Final Fight (Europe) (Budget - Kixx).ipf| f18557040f7370b5c682456e668412ef |                 | Long Tracks (1.93 usec)                |
| X-Out (Europe) (Disk 1).ipf                               | 1fd85af060f96619ba7b9cc3d12ff119 |                 | Long Tracks (1.96 usec)                |
| Apprentice (Europe).ipf                                   | 324aa78a88b1e33e343998d58359d4a7 |                 |                                        |
| James Pond II - Codename RoboCod (Europe).ipf             | a6b91be93105d903e0634b69c2be86bc |                 |                                        |
| Thrust (Europe).ipf                                       | b76986a30c093cf22b718a3d7af771d6 |                 |                                        |

### C64

It seems that C64 images are rarely delivered as G64 file. Instead we usually get a NIB file which shall be converted first.

#### NIB Files used for conversion

| Name                                         | MD5                              | Notes                                       |
|----------------------------------------------|----------------------------------|---------------------------------------------|
| Katakis (Side 1).nib                         | 63fcfea043054882cfc31ae43fd0a5f9 | nibconv -r                                  |
| turrican_2_s1\[rainbow_arts_1991\]\(r2).nib  | 2940f1d9672061f5da2b9a10699526ee | Doesn't even work in emulator. Broken image?|
| Turrican (Europe) (Side 1).nib               | 7a0ea1dd18294659d6df10eb1e441084 | Doesn't even work in emulator. Broken image?|
| Turrican (Europe) (Alt 1) (Side 1).nib       | 8a5b1032ed0f02118e0b1dafeba74931 | nibconv -r                                  |
| Turrican (Europe) (Alt 1) (Side 2).nib       | dbcd6884bc3123e3e791d4f14e8f3a3d | nibconv -r                                  |
| x-out_s1\[rainbow_arts_1989\]\(r2).nib       | c8bc58739ecd9c8dd8509cea784d01bb | nibconv -r                                  |


#### G64 files resulting from NIB conversion

| Name                                         | MD5                              | Notes                              | Copy Protection Method                   |
|----------------------------------------------|----------------------------------|------------------------------------|------------------------------------------|
| Katakis (Side 1).g64                         | 53c47c575d057181a1911e6653229324 | Green Level 2. Broken protection\* | Rainbow Arts (RADWAR) - Timing Exact Sync|
| x-out_s1\[rainbow_arts_1989\]\(r2).g64       | 9785b035823c8f366a92d98bcf91544d |                                    | Weak Bits                                |
| Turrican (Europe) (Alt 1) (Side 1).g64       | 79edb43946e428ba8000f21681a825dd |                                    |                                          |
| Turrican (Europe) (Alt 1) (Side 2).g64       | fa58c3d902af0b6f5027fcb560fababd |                                    |                                          |

#### G64 files premastered

| Name                                                   | MD5                              | Notes                                                           | Copy Protection Method                   |
|--------------------------------------------------------|----------------------------------|-----------------------------------------------------------------|------------------------------------------|
| turrican_ii_s1\[rainbow_arts_1991\]\(!).g64            | 43d928ad9c0791e6fa0b0e73a50757fe |                                                                 |                                          |
| turrican_ii_s2\[rainbow_arts_1991\]\(!).g64            | bd88b076129238f688f88757a6bfa4b5 |                                                                 |                                          |
| turrican_s1\[rainbow_arts_1990\]\(pal)\(!).g64         | 3904bf094cd24e1c32dcf0588aeb53ec |                                                                 |                                          |
| turrican_s2\[rainbow_arts_1990\]\(pal)\(!).g64         | 15e3c8ec7f40b85b8cd3de3fd5cc692d |                                                                 |                                          |
| nebulus\[hewson_1987\]\(pal).g64                       | 0290df644e609e0ecb50ca7ae868c396 | The loader is weird.                                            |                                          |
| x-out_s1\[rainbow_arts_1989\]\(!).g64                  | 6780f6fab0e8e69a804921bcc8834382 |                                                                 |                                          |
| cybernoid_ii\[hewson_1988\].g64                        | 01585840dbe6962361eabdb8b2d34025 |                                                                 |                                          |
| katakis_s1\[rainbow_arts_1988\]\(r1)(!).g64            | 406d29151e7001f6bfc7d95b7ade799d | Green Level 2. Broken protection\*                              | Rainbow Arts (RADWAR) - Timing Exact Sync|
| katakis_s1\[rainbow_arts_1988\]\(r1)(alt).g64          | d2aa92ccf3531fc995e771be91a45241 | White Level 2. Broken protection\*. Corrupt highscore table.\*\*| Rainbow Arts (RADWAR) - Timing Exact Sync|
| thrust\[firebird_1986\]\(pal).g64                      | 53f9553277c4c526c509b88af56a837f |                                                                 |                                          |
| Great_Giana_Sisters_The.g64                            | c2334233136c523b9ec62beb8bea1e00 | Broken protection\*                                             | Rainbow Arts (RADWAR) - Timing Exact Sync|
| mr_robot_factory\[datamost_1983\].g64                  | 2ead4cc911d984120659c876e8d2ea1d | Very slow loading                                               |                                          |
| mayhem_in_monsterland_s1\[apex_1993\]\(pal).g64        | a3c10eb5c590ba8bd5c9bb8e0a08226c | Additional code sheet required.                                 |                                          |
| mayhem_in_monsterland_s2\[apex_1993\]\(pal)(!).g64     | 321194b397e8f6d7ae8d60841dc5d9ac |                                                                 |                                          |
| impossible_mission\[epyx_1984\]\(pal\)(!).g64          | 5d97d83a7caf5ad2ddbace6c2504b436 | Bumps head while loading!                                       |                                          |
| r-type_s1\[electric_dreams_1988]\(pal\)(!).g64         | db2a2d351f5701746c7af0806c8de389 | Bumps head while loading!                                       |                                          |
| r-type_s2\[electric_dreams_1988]\(pal\)(!).g64         | 259f2c7cd4bfefe32e6ca1b66df6a983 |                                                                 |                                          |
| lode_runner\[br0derbund_1983\]\(00)(black_label)(!).g64| f10fb40754b73a57c2e38957215e76ce | Bumps head while loading!                                       |                                          |
| boulder_dash\[first_star_1984\]\(pal)(!).g64           | b8350b9d1cc76a33e99218b446695a97 |                                                                 |                                          |
| armalyte_s1\[thalamus_1988\]\(pal)(!).g64              | 8dd4bfe82f254b2dbf5fd4998e406e12 |                                                                 |                                          |
| armalyte_s2\[thalamus_1988\]\(pal)(!).g64              | 725858316042378290db0c41842f94e9 |                                                                 |                                          |

\* Some disks have a broken protection track. These are patched by the tool if found.

\*\* Katakis Side 1 with a md5sum of d2aa92ccf3531fc995e771be91a45241 has a damaged highscore table with scores impossible to beat.
The score table is stored in cylinder 4.
When comparing to the version with the green level (md5sum of 406d29151e7001f6bfc7d95b7ade799d),
the only difference in cylinder 4 is the high score table.
I conclude that the table can be reset/fixed by overwriting cylinder 4 with another working copy.

To get the white level 2 version with a working high score table, we write the broken one first and overwrite a single cylinder from the other:

	usbfloppytracer -b 'katakis_s1[rainbow_arts_1988](r1)(alt).g64'
	usbfloppytracer -b 'katakis_s1[rainbow_arts_1988](r1)(!).g64' -t4

After doing so, we are granted with a white level 2 version of Katakis and an empty highscore table.

### Amstrad CPC

| Name                                                                            | MD5                              | Notes                          | Copy Protection Method                 |
|---------------------------------------------------------------------------------|----------------------------------|--------------------------------|----------------------------------------|
| Turrican (UK) (Face A) (1990) (CPM) (UK retail version) \[Original\].dsk        | c64c93cf9abf0a35aa451cc9150ef4a0 |                                | Hexagon Disk Protection - 1989 - Type 3|
| Turrican 2 (UK) (Face A) (1991) (UK retail version) \[Original\].dsk            | a7b2af46f0f31e86444bbf4a7feee670 |                                |                                        |
| Solomons Key + Road Runner (UK) (Face A) (1989) \[Original\] \[COMPILATION\].dsk| 31c1ca3ff99e9908eb02f17c2cd4b881 | This disk contains Solomons Key| None                                   |
| Gryzor (UK) (1987) (CPM) \[Original\].dsk                                       | bf70f0893459a1b25a917f953c8dc6ee |                                | Erased sectors                         |
| Spindizzy (UK) (1986) \[Original\].dsk                                          | e8eaf5c64ab6125a651099db34eea75c |                                | None                                   |
| Cybernoid (UK) (1988) (CPM) \[Original\].dsk                                    | da98036cd7f1d8d051651d85f1f33e3f | Start in CPM mode              | None                                   |

## Not working with this tool

This list doesn't mean that these images won't be supported in the future.
It is mostly a TODO list for me and a hint for others who are struggling reconstructing this particular disk.


| Name                                       | MD5                             | Notes                                   | Copy Protection Method                                 |
|--------------------------------------------|---------------------------------|-----------------------------------------|--------------------------------------------------------|
| Batman (2 disk) A.stx                      | a35e2a6c32dd77fefb76cc81d83db56d| Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV). Data Tracks (DTT)|
| enchanted_land.stx                         | 823066c507d10d6f69109788660eadc7| Doesn't load                            | Data in Gap? (HDG)                                     |
| nebulus.stx                                | c94ccfcccfa1fba31cc913ad7b8dcc2f| Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV)                   |
|                                            |                                 |                                         |                                                        |


## Images which refuse to work

These images seem to have some sort of problem.
Help or information about these would be required to get these to work.
It is possible that the image itself is broken.

| Name                                       | MD5                              | Notes                                                                 | Copy Protection Method   |
|--------------------------------------------|----------------------------------|-----------------------------------------------------------------------|--------------------------|
| rodland\[sales_curve_1989\]\(pal).g64      | 18b0f9cd2f223af76afefed2365bacb3 | Loads very slowly. Crashes on first level even in emulator. Don't use.|                          |

