// I don't get this. But it seems to be required under Windows
// to make bindgen to work while compilation is not an issue.
#undef _WIN32

#include <stdint.h>
#include <caps/CapsLibAll.h>

// Fixes indirection as with newer versions of libcapsimage, the lock flags are now
// enums instead of actual constants. bindgen has issues detecting that.
const uint32_t FLAG_LOCK_INDEX = DI_LOCK_INDEX;
const uint32_t FLAG_LOCK_DENVAR = DI_LOCK_DENVAR;
const uint32_t FLAG_LOCK_TYPE = DI_LOCK_TYPE;
