## Copy protected images which have been checked

### Supported by this tool and tested

Quality can vary between raw images. Writing and verification of .g64 and .ipf images is not guaranteed.
Writing of .stx images sometimes requires lots of patching as quality between images varies.
Therefore I try to keep a list of images which are expected to work with this software.

| Name                                            | MD5                              | Notes                                     | Copy Protection Method                 |
|-------------------------------------------------|----------------------------------|-------------------------------------------|----------------------------------------|
| Apidya (Germany) (En) (Disk 1).ipf              | 3adf2ffa5fbf740515576c10f46e1a67 |                                           | Long Tracks                            |
| EnchantedLand.ipf                               | d907e262b6a3a72e0c690216bb9d0290 |                                           |                                        |
| Gods_Disc1.ipf                                  | 7b2a11eda49fc6841834e792dab53997 |                                           |                                        |
| Jim Power in Mutant Planet (Europe) (Disk 1).ipf| 78b2a03c31a30aadbcb269e75ae94853 |                                           |                                        |
| Jumping Jack'Son (Europe).ipf                   | b4106a4ae184f5547d87be0601c71c9e |                                           |                                        |
| Katakis (Side 1).g64                            | 53c47c575d057181a1911e6653229324 | Created with nibconv from .nib image      | Rainbow Arts (RADWAR)                  |
| Katakis (Side 1).nib                            | 63fcfea043054882cfc31ae43fd0a5f9 | ./nibconv -r katakis_s1.nib katakis_s1.g64| Rainbow Arts (RADWAR)                  |
| Rick Dangerous.stx                              | d365e49de69644e386ecb4dcba03509e |                                           |                                        |
| Rodland (Europe) (v1.32).ipf                    | 5bf77241b8ce88a323010e82bf18f3e0 |                                           | CopyLock - Rob Northen Computing       |
| Rodland.stx                                     | 80f6322934ca1c76bb04b5c4d6d25097 |                                           | CopyLock - Rob Northen Computing       |
| Turrican (1990)(Rainbow Arts).stx               | 4865957cd83562547a722c95e9a5421a |                                           | Sector in Sector, No Flux Reversal Area|
| Turrican2.ipf                                   | 17abf9d8d5b2af451897f6db8c7f4868 | Might require write precompensation       | Long Tracks                            |
| Turrican II (1991)(Rainbow Arts).stx            | fb96a28ad633208a973e725ceb67c155 |                                           | Long Tracks                            |
| Turrican III - Payment Day (Germany).ipf        | e471c215d5c58719aeec1172b6e2b0e5 |                                           | Long Tracks                            |
| Turrican.ipf                                    | 654e52bec1555ab3802c21f6ea269e64 |                                           | Long Tracks                            |
| X-Out_1.ipf                                     | 1784c149245dfecde23223dc217604b0 |                                           | Long Tracks                            |
| Z-Out (Europe).ipf                              | 0ff89947aede0817f443712d3689f503 |                                           | Long Tracks                            |


### Not yet working with this tool

This list doesn't mean that these images won't be supported in the future.
It is mostly a TODO list for me and a hint for others who are struggling reconstructing this particular disk.


| Name                                             | MD5                              | Notes                                   | Copy Protection Method                                 |
|--------------------------------------------------|----------------------------------|-----------------------------------------|--------------------------------------------------------|
| Batman (2 disk) A.stx                            | a35e2a6c32dd77fefb76cc81d83db56d | Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV). Data Tracks (DTT)|
| enchanted_land.stx                               | 823066c507d10d6f69109788660eadc7 | Doesn't load                            | Data in Gap? (HDG)                                     |
| nebulus.stx                                      | c94ccfcccfa1fba31cc913ad7b8dcc2f | Unsupported fdc flags                   | Fuzzy Bits? Macrodos/Speedlock (SBV)                   |

