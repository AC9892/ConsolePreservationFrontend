/**
 * Xenia : Xbox 360 Emulator Research Project
 * Released under the BSD license - see LICENSE in the root for more details.
 */
#include "xenia/base/cvar.h"
#include "xenia/base/logging.h"
#include "xenia/base/memory.h"
#include "xenia/cpu/xex_crypto.h"
#include "xenia/kernel/kernel_state.h"
#include "xenia/kernel/title_id_utils.h"
#include "xenia/kernel/util/shim_utils.h"
#include "xenia/kernel/util/xex2_info.h"
#include "xenia/kernel/xboxkrnl/xecrypt_aes.h"
#include "xenia/kernel/xboxkrnl/xboxkrnl_private.h"

DEFINE_bool(dashboard_disc_crypto, false,
            "Experimental retail XEX2 resource decryption for the dashboard.",
            "Kernel");

namespace xe::kernel::xboxkrnl {
static bool DashboardDiscCryptoEnabled() {
  return cvars::dashboard_disc_crypto &&
         kernel_state()->title_id() == kDashboardID;
}

dword_result_t XexTransformImageKey_entry(lpvoid_t security, dword_t length) {
  if (!DashboardDiscCryptoEnabled()) {
    return X_STATUS_NOT_IMPLEMENTED;
  }
  if (!security || length != 0x174) {
    return X_STATUS_INVALID_PARAMETER;
  }
  // Observed XEX2 input is security-info + 8, excluding its two size words.
  // Mirror the emulator loader's retail key unwrap, not console authorization.
  auto* data = security.as<uint8_t*>();
  const auto flags = xe::load_and_swap<uint32_t>(data + 0x104);
  if (flags & (XEX_IMAGE_CARDEA_KEY | XEX_IMAGE_XEIKA_KEY)) {
    return X_STATUS_NOT_IMPLEMENTED;
  }
  cpu::UnwrapRetailXex2ImageKey(data + 0x148, data + 0x148);
  XELOGI("Dashboard retail XEX2 image-key transform completed");
  return X_STATUS_SUCCESS;
}
DECLARE_XBOXKRNL_EXPORT1(XexTransformImageKey, kNone, kSketchy);

dword_result_t XeCryptAesCreateKeySchedule_entry(
    lpvoid_t key, dword_t length, pointer_t<Aes128Schedule> schedule) {
  if (!DashboardDiscCryptoEnabled()) {
    return X_STATUS_NOT_IMPLEMENTED;
  }
  return CreateAes128Schedule(key.as<const uint8_t*>(), length,
                              static_cast<Aes128Schedule*>(schedule))
             ? X_STATUS_SUCCESS
             : X_STATUS_INVALID_PARAMETER;
}
DECLARE_XBOXKRNL_EXPORT1(XeCryptAesCreateKeySchedule, kNone, kSketchy);

static uint32_t DashboardAesCbc(pointer_t<Aes128Schedule> schedule,
                                lpvoid_t input, dword_t length, lpvoid_t output,
                                lpvoid_t iv, bool encrypt) {
  if (!DashboardDiscCryptoEnabled()) {
    return X_STATUS_NOT_IMPLEMENTED;
  }
  return Aes128Cbc(static_cast<Aes128Schedule*>(schedule),
                   input.as<const uint8_t*>(), length,
                   output.as<uint8_t*>(), iv.as<uint8_t*>(), encrypt)
             ? X_STATUS_SUCCESS
             : X_STATUS_INVALID_PARAMETER;
}

dword_result_t XeCryptAesCbcEncrypt_entry(pointer_t<Aes128Schedule> schedule,
                                         lpvoid_t input, dword_t length,
                                         lpvoid_t output, lpvoid_t iv) {
  return DashboardAesCbc(schedule, input, length, output, iv, true);
}
DECLARE_XBOXKRNL_EXPORT1(XeCryptAesCbcEncrypt, kNone, kSketchy);

dword_result_t XeCryptAesCbcDecrypt_entry(pointer_t<Aes128Schedule> schedule,
                                         lpvoid_t input, dword_t length,
                                         lpvoid_t output, lpvoid_t iv) {
  return DashboardAesCbc(schedule, input, length, output, iv, false);
}
DECLARE_XBOXKRNL_EXPORT1(XeCryptAesCbcDecrypt, kNone, kSketchy);
}  // namespace xe::kernel::xboxkrnl

DECLARE_XBOXKRNL_EMPTY_REGISTER_EXPORTS(DashboardCrypto);
