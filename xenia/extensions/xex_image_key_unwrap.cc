// CPF addition under Xenia BSD 3-clause terms; see LICENSE.xenia.
// Source view only; apply the canonical patch, do not compile this view.
// Existing upstream key constants are intentionally not republished.

namespace xe::cpu {
void UnwrapRetailXex2ImageKey(const uint8_t* wrapped, uint8_t* output) {
  uint8_t result[16];
  aes_decrypt_buffer(xe_xex2_retail_key, wrapped, sizeof(result), result,
                     sizeof(result));
  std::memcpy(output, result, sizeof(result));
}
}  // namespace xe::cpu
