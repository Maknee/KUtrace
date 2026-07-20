#define _SDT_HAS_SEMAPHORES 1
#include <stdint.h>
#include <sys/sdt.h>

unsigned short kutrace_agent_begin_semaphore;
unsigned short kutrace_agent_end_semaphore;

__attribute__((noinline)) uint64_t kutrace_usdt_fixture(uint64_t depth) {
  uint64_t result;
  if (kutrace_agent_begin_semaphore != 0) {
    STAP_PROBE1(kutrace, agent_begin, depth);
  }
  if (depth == 0) {
    result = 1;
  } else {
    result = kutrace_usdt_fixture(depth - 1) + depth;
  }
  if (kutrace_agent_end_semaphore != 0) {
    STAP_PROBE1(kutrace, agent_end, result);
  }
  return result;
}

uint32_t kutrace_usdt_semaphores(void) {
  return ((uint32_t)kutrace_agent_end_semaphore << 16) |
         kutrace_agent_begin_semaphore;
}
