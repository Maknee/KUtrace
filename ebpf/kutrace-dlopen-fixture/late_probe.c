#include <stdint.h>

__attribute__((noinline, visibility("default")))
uint64_t kutrace_late_probe(uint64_t value) {
  return (value * UINT64_C(33)) ^ UINT64_C(0x5a5a);
}
