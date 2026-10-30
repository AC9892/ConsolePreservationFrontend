#include <array>
#include "third_party/catch/single_include/catch2/catch.hpp"
#include "xenia/kernel/xam/tile_texture.h"

using xe::kernel::xam::CopyTileRgbaToArgb;

TEST_CASE("Gamer tile conversion honors row pitch and channel order", "[tile]") {
  const std::array<uint8_t, 8> source{1, 2, 3, 4, 5, 6, 7, 8};
  std::array<uint8_t, 16> destination;
  destination.fill(0xCD);
  REQUIRE(CopyTileRgbaToArgb(source, 1, 2, destination, 8, 2));
  const std::array<uint8_t, 16> expected{
      4, 1, 2, 3, 0xCD, 0xCD, 0xCD, 0xCD,
      8, 5, 6, 7, 0xCD, 0xCD, 0xCD, 0xCD};
  REQUIRE(destination == expected);
}

TEST_CASE("Gamer tile rejects invalid extents without touching output", "[tile]") {
  const std::array<uint8_t, 8> source{};
  std::array<uint8_t, 16> destination;
  destination.fill(0xCD);
  const auto before = destination;
  REQUIRE_FALSE(CopyTileRgbaToArgb(source, 2, 1, destination, 4, 1));
  REQUIRE_FALSE(CopyTileRgbaToArgb(source, 1, 2, destination, 4, 1));
  REQUIRE_FALSE(CopyTileRgbaToArgb(source, 1, 2, destination, 12, 2));
  REQUIRE_FALSE(CopyTileRgbaToArgb(source, 1, 3, destination, 4, 3));
  REQUIRE_FALSE(CopyTileRgbaToArgb(source, 0, 1, destination, 4, 1));
  REQUIRE(destination == before);
}
