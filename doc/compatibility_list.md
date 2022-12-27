# Copy protected images which have been checked

## Supported by this tool and tested on a real machine

Quality can vary between raw images. Writing and verification of .g64 and .ipf images is not guaranteed.
Writing of .stx images sometimes requires lots of patching as quality between images varies.
Therefore I try to keep a list of images which are expected to work with this software.

### Amiga

| Name                                                                        | MD5                              | Notes                                 | Copy Protection Method |
|-----------------------------------------------------------------------------|----------------------------------|---------------------------------------|------------------------|
| Apidya (Germany) (En) (Disk 1).ipf                                          | 3adf2ffa5fbf740515576c10f46e1a67 |                                       | Long Tracks (1.80 usec)|
| EnchantedLand.ipf                                                           | d907e262b6a3a72e0c690216bb9d0290 |                                       | Long Tracks (1.96 usec)|
| Gods_Disc1.ipf                                                              | 7b2a11eda49fc6841834e792dab53997 |                                       | Variable Density Track |
| James Pond II - Codename RoboCod (Europe).ipf                               | 7ef8e61c300717005e78a1b6494a84d4 |                                       | Long Tracks (1.80 usec)|
| Jim Power in Mutant Planet (Europe) (Disk 1).ipf                            | 78b2a03c31a30aadbcb269e75ae94853 |                                       | Long Tracks (1.89 usec)|
| Jumping Jack'Son (Europe).ipf                                               | b4106a4ae184f5547d87be0601c71c9e |                                       | Long Tracks (1.89 usec)|
| Rodland (Europe) (v1.32).ipf                                                | 5bf77241b8ce88a323010e82bf18f3e0 |                                       | Variable Density Track |
| Turrican2.ipf                                                               | 17abf9d8d5b2af451897f6db8c7f4868 | Might require write precompensation   | Long Tracks (1.80 usec)|
| Turrican III - Payment Day (Germany).ipf                                    | e471c215d5c58719aeec1172b6e2b0e5 |                                       | Long Tracks (1.80 usec)|
| Turrican.ipf                                                                | 654e52bec1555ab3802c21f6ea269e64 |                                       | Long Tracks (1.85 usec)|
| X-Out_1.ipf                                                                 | 1784c149245dfecde23223dc217604b0 | Sync on 0x8455. Nibble with X-Copy    | Custom Sync Word       |
| Z-Out (Europe).ipf                                                          | 0ff89947aede0817f443712d3689f503 | Can be copied with X-Copy             | No Copy Protection?    |
| Lemmings (Europe) (Amiga 500 Bundle - Cartoon Classics).ipf                 | d0d29f214ea57aef2bf1a8dfe508b8ba |                                       | Variable Density Track |
| P.P. Hammer and His Pneumatic Weapon (Europe) (Budget - Global Software).ipf| bd6477aa9a7ac1ff142812d85ed20143 | Can be copied with X-Copy             | No Copy Protection?    |
| Lotus Turbo Challenge 2 (Europe).ipf                                        | ed4321338a4544b6892383cfa2173241 | Also protected by codes in the manual | Long Tracks (1.89 usec)|

### Atari ST

| Name                                            | MD5                              | Notes                                     | Copy Protection Method                 |
|-------------------------------------------------|----------------------------------|-------------------------------------------|----------------------------------------|
| Rick Dangerous.stx                              | d365e49de69644e386ecb4dcba03509e |                                           |                                        |
| Rodland.stx                                     | 80f6322934ca1c76bb04b5c4d6d25097 |                                           | CopyLock - Rob Northen Computing       |
| Turrican (1990)(Rainbow Arts).stx               | 4865957cd83562547a722c95e9a5421a |                                           | Sector in Sector, No Flux Reversal Area|
| Turrican II (1991)(Rainbow Arts).stx            | fb96a28ad633208a973e725ceb67c155 |                                           | Long Tracks                            |


### C64

| Name                                            | MD5                              | Notes                                     | Copy Protection Method                 |
|-------------------------------------------------|----------------------------------|-------------------------------------------|----------------------------------------|
| Katakis (Side 1).g64                            | 53c47c575d057181a1911e6653229324 | Created with nibconv from .nib image      | Rainbow Arts (RADWAR)                  |
| Katakis (Side 1).nib                            | 63fcfea043054882cfc31ae43fd0a5f9 | ./nibconv -r katakis_s1.nib katakis_s1.g64| Rainbow Arts (RADWAR)                  |

## Not yet working with this tool

This list doesn't mean that these images won't be supported in the future.
It is mostly a TODO list for me and a hint for others who are struggling reconstructing this particular disk.


| Name                                             | MD5                              | Notes                                   | Copy Protection Method                                 |
|--------------------------------------------------|----------------------------------|-----------------------------------------|--------------------------------------------------------|
| Batman (2 disk) A.stx                            | a35e2a6c32dd77fefb76cc81d83db56d | Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV). Data Tracks (DTT)|
| enchanted_land.stx                               | 823066c507d10d6f69109788660eadc7 | Doesn't load                            | Data in Gap? (HDG)                                     |
| nebulus.stx                                      | c94ccfcccfa1fba31cc913ad7b8dcc2f | Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV)                   |

