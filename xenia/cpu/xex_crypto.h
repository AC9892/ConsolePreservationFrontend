/**
 * Xenia : Xbox 360 Emulator Research Project
 * Released under the BSD license - see LICENSE in the root for more details.
 */
#ifndef XENIA_CPU_XEX_CRYPTO_H_
#define XENIA_CPU_XEX_CRYPTO_H_
#include <cstdint>
namespace xe::cpu {
// Reuses the loader's existing retail XEX2 key handling. No key material is
// exposed through this interface or added to callers.
void UnwrapRetailXex2ImageKey(const uint8_t* wrapped, uint8_t* output);
}  // namespace xe::cpu
#endif  // XENIA_CPU_XEX_CRYPTO_H_
