#include <array>
#include <cstring>
#include "third_party/catch/include/catch.hpp"
#include "xenia/kernel/xboxkrnl/xecrypt_aes.h"

using namespace xe::kernel::xboxkrnl;

TEST_CASE("Dashboard AES128 CBC known result and in-place decrypt", "[aes]") {
  // Synthetic all-zero key, plaintext and IV; no console data or title keys.
  const std::array<uint8_t, 16> key{};
  const std::array<uint8_t, 32> plain{};
  const std::array<uint8_t, 32> expected{
      0x66, 0xE9, 0x4B, 0xD4, 0xEF, 0x8A, 0x2C, 0x3B,
      0x88, 0x4C, 0xFA, 0x59, 0xCA, 0x34, 0x2B, 0x2E,
      0xF7, 0x95, 0xBD, 0x4A, 0x52, 0xE2, 0x9E, 0xD7,
      0x13, 0xD3, 0x13, 0xFA, 0x20, 0xE9, 0x8D, 0xBC};
  Aes128Schedule schedule;
  REQUIRE(CreateAes128Schedule(key.data(), key.size(), &schedule));
  std::array<uint8_t, 16> iv{};
  std::array<uint8_t, 32> encrypted;
  REQUIRE(Aes128Cbc(&schedule, plain.data(), plain.size(), encrypted.data(),
                     iv.data(), true));
  REQUIRE(encrypted == expected);
  REQUIRE(std::memcmp(iv.data(), expected.data() + 16, 16) == 0);
  iv.fill(0);
  REQUIRE(Aes128Cbc(&schedule, encrypted.data(), encrypted.size(),
                     encrypted.data(), iv.data(), false));
  REQUIRE(encrypted == plain);
  REQUIRE(std::memcmp(iv.data(), expected.data() + 16, 16) == 0);
}

TEST_CASE("Dashboard AES128 CBC preserves IV between chunks", "[aes]") {
  std::array<uint8_t, 16> key{};
  Aes128Schedule schedule;
  REQUIRE(CreateAes128Schedule(key.data(), key.size(), &schedule));
  std::array<uint8_t, 48> whole{};
  std::array<uint8_t, 48> chunks{};
  std::array<uint8_t, 16> whole_iv{};
  std::array<uint8_t, 16> chunk_iv{};
  REQUIRE(Aes128Cbc(&schedule, whole.data(), whole.size(), whole.data(),
                     whole_iv.data(), true));
  REQUIRE(Aes128Cbc(&schedule, chunks.data(), 16, chunks.data(),
                     chunk_iv.data(), true));
  REQUIRE(Aes128Cbc(&schedule, chunks.data() + 16, 32, chunks.data() + 16,
                     chunk_iv.data(), true));
  REQUIRE(chunks == whole);
  REQUIRE(chunk_iv == whole_iv);
}

TEST_CASE("Dashboard AES128 rejects unsupported inputs without writes", "[aes]") {
  std::array<uint8_t, 32> key{};
  Aes128Schedule schedule{};
  REQUIRE_FALSE(CreateAes128Schedule(key.data(), key.size(), &schedule));
  REQUIRE((schedule.magic == std::array<uint8_t, 4>{}));
  REQUIRE(CreateAes128Schedule(key.data(), 16, &schedule));
  std::array<uint8_t, 32> buffer;
  buffer.fill(0xA5);
  const auto before = buffer;
  std::array<uint8_t, 16> iv{};
  REQUIRE_FALSE(Aes128Cbc(&schedule, buffer.data(), 17, buffer.data(),
                           iv.data(), false));
  REQUIRE_FALSE(Aes128Cbc(&schedule, buffer.data(), 16, buffer.data() + 1,
                           iv.data(), true));
  REQUIRE(buffer == before);
  REQUIRE((iv == std::array<uint8_t, 16>{}));
  Aes128Schedule invalid{};
  REQUIRE_FALSE(Aes128Cbc(&invalid, buffer.data(), 16, buffer.data(),
                           iv.data(), false));
}
