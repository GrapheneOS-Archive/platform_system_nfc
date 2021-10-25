#pragma once
#include "hal/hidl_hal.rs.h"

namespace nfc {
namespace hal {

void start_hal();
void stop_hal();
void send_command(rust::Slice<const uint8_t> data);

}  // namespace hal
}  // namespace nfc
