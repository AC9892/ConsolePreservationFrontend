#ifndef XENIA_KERNEL_XAM_TILE_TEXTURE_H_
#define XENIA_KERNEL_XAM_TILE_TEXTURE_H_

#include <cstddef>
#include <cstdint>
#include <span>

namespace xe::kernel::xam {

// Copy decoded RGBA into guest ARGB rows, preserving pitch padding.
inline bool CopyTileRgbaToArgb(std::span<const uint8_t> rgba, size_t width,
                               size_t height, std::span<uint8_t> destination,
                               size_t stride, size_t destination_height) {
  if (!width || !height || width > stride / 4 ||
      height > destination_height || stride == 0 ||
      destination_height > destination.size() / stride ||
      height > rgba.size() / 4 / width) {
    return false;
  }
  for (size_t y = 0; y < height; ++y) {
    for (size_t x = 0; x < width; ++x) {
      const auto* source = rgba.data() + (y * width + x) * 4;
      auto* target = destination.data() + y * stride + x * 4;
      target[0] = source[3];
      target[1] = source[0];
      target[2] = source[1];
      target[3] = source[2];
    }
  }
  return true;
}

}  // namespace xe::kernel::xam

#endif  // XENIA_KERNEL_XAM_TILE_TEXTURE_H_
