// CPF addition to Xenia Canary, distributed under upstream BSD 3-clause terms.
// Source view only: the canonical patch inserts this handler into
// src/xenia/kernel/xboxkrnl/xboxkrnl_crypt.cc. Do not also compile this view.
// The rest of the upstream cryptography file is intentionally not republished.
#include "xenia/emulator.h"
#include "xenia/kernel/kernel_state.h"
#include "xenia/kernel/title_id_utils.h"
#include "xenia/kernel/util/shim_utils.h"
#include "xenia/kernel/xboxkrnl/xboxkrnl_private.h"

namespace xe::kernel::xboxkrnl {
dword_result_t XeKeysGetMediaID_entry(lpvoid_t media_id, dword_t source) {
  // The dashboard asks for the inserted disc's 16-byte XGD2 media ID with
  // source 1. This is the security-header ID, not the 32-bit execution ID.
  // Other sources and title contexts still need their own implementation.
  if (!media_id) {
    return X_STATUS_INVALID_PARAMETER;
  }
  if (source != 1 || kernel_state()->title_id() != kDashboardID) {
    return X_STATUS_NOT_IMPLEMENTED;
  }
  if (!kernel_state()->emulator()->GetDashboardDiscMediaId(
          media_id.as<uint8_t*>())) {
    return X_STATUS_NOT_FOUND;
  }
  return X_STATUS_SUCCESS;
}
DECLARE_XBOXKRNL_EXPORT1(XeKeysGetMediaID, kNone, kSketchy);
}  // namespace xe::kernel::xboxkrnl
