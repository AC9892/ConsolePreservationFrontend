/**
 * Xenia : Xbox 360 Emulator Research Project
 * Released under the BSD license - see LICENSE in the root for more details.
 */
#ifndef XENIA_KERNEL_XBOXKRNL_XECRYPT_AES_H_
#define XENIA_KERNEL_XBOXKRNL_XECRYPT_AES_H_

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>

extern "C" {
#include "third_party/aes_128/aes.h"
}

namespace xe::kernel::xboxkrnl {

// Private HLE representation for the paired modern AES exports. The observed
// dashboard treats the schedule as opaque and reserves 0x1D8 bytes for it.
// This is not a claim about the console's expanded-key memory layout.
struct Aes128Schedule {
  std::array<uint8_t, 4> magic;
  std::array<uint8_t, AES_ROUND_KEY_SIZE> encryption;
};
static_assert(sizeof(Aes128Schedule) <= 0x1D8);
inline constexpr std::array<uint8_t, 4> kAes128ScheduleMagic{'X', 'A', 'E', '1'};

inline bool CreateAes128Schedule(const uint8_t* key, size_t length,
                                 Aes128Schedule* output) {
  if (!key || !output || length != AES_BLOCK_SIZE) {
    return false;
  }
  Aes128Schedule result{};
  aes_key_schedule_128(key, result.encryption.data());
  result.magic = kAes128ScheduleMagic;
  *output = result;
  return true;
}

inline bool Aes128Cbc(const Aes128Schedule* schedule, const uint8_t* input,
                      size_t length, uint8_t* output, uint8_t* iv,
                      bool encrypt) {
  if (!schedule || schedule->magic != kAes128ScheduleMagic || !iv ||
      length % AES_BLOCK_SIZE || (length && (!input || !output))) {
    return false;
  }
  // Exact in-place operation is supported; shifted overlaps are not.
  if (input != output && length) {
    const auto in = reinterpret_cast<uintptr_t>(input);
    const auto out = reinterpret_cast<uintptr_t>(output);
    if ((in < out ? out - in : in - out) < length) {
      return false;
    }
  }
  for (size_t offset = 0; offset < length; offset += AES_BLOCK_SIZE) {
    std::array<uint8_t, AES_BLOCK_SIZE> block;
    std::memcpy(block.data(), input + offset, block.size());
    if (encrypt) {
      for (size_t j = 0; j < block.size(); ++j) {
        block[j] ^= iv[j];
      }
      aes_encrypt_128(schedule->encryption.data(), block.data(), output + offset);
      std::memcpy(iv, output + offset, block.size());
    } else {
      const std::array<uint8_t, AES_BLOCK_SIZE> original = block;
      aes_decrypt_128(schedule->encryption.data(), block.data(), output + offset);
      for (size_t j = 0; j < block.size(); ++j) {
        output[offset + j] ^= iv[j];
      }
      // Decryption requires carrying the original ciphertext block forward as the
      // next IV. The decrypted block is only used as the plaintext output.
      std::memcpy(iv, original.data(), original.size());
    }
  }
  return true;
}
}  // namespace xe::kernel::xboxkrnl
#endif  // XENIA_KERNEL_XBOXKRNL_XECRYPT_AES_H_
