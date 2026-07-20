#ifndef KUTRACE_CLIENT_H_
#define KUTRACE_CLIENT_H_

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Set KUTRACE_AGENT_SHM (preferred) or KUTRACE_AGENT_SOCKET before the first
// call. Emission is non-blocking and best effort.
#define KUTRACE_CLIENT_RPC_REQUEST       0x201
#define KUTRACE_CLIENT_RPC_RESPONSE      0x202
#define KUTRACE_CLIENT_RPC_MIDDLE        0x203
#define KUTRACE_CLIENT_RPC_RX_MESSAGE    0x204
#define KUTRACE_CLIENT_RPC_TX_MESSAGE    0x205
#define KUTRACE_CLIENT_MARK_A            0x20A
#define KUTRACE_CLIENT_MARK_B            0x20B
#define KUTRACE_CLIENT_MARK_C            0x20C
#define KUTRACE_CLIENT_MARK_D            0x20D
#define KUTRACE_CLIENT_LOCK_NO_ACQUIRE   0x210
#define KUTRACE_CLIENT_LOCK_ACQUIRE      0x211
#define KUTRACE_CLIENT_LOCK_WAKEUP       0x212
#define KUTRACE_CLIENT_RX_USER           0x216
#define KUTRACE_CLIENT_TX_USER           0x217
#define KUTRACE_CLIENT_RESOURCE          0x219
#define KUTRACE_CLIENT_ENQUEUE           0x21A
#define KUTRACE_CLIENT_DEQUEUE           0x21B
#define KUTRACE_CLIENT_MONITOR_STORE     0x21E

#define KUTRACE_ANNOTATION_QUERY         0
#define KUTRACE_ANNOTATION_OBSERVATION   1
#define KUTRACE_ANNOTATION_DECISION      2
#define KUTRACE_ANNOTATION_RESULT        3

uint64_t kutrace_span_begin(const char* label);
void kutrace_span_end(uint64_t span_id);
/* eventtospan3 reconstructs final JSON RPC context from RPC marker events. */
int kutrace_legacy_marker(uint16_t event, uint64_t arg, int32_t retval,
                          uint32_t rpc, const char* label);
int kutrace_span_annotate(uint64_t span_id, uint16_t kind, uint64_t value,
                          const char* label);
uint64_t kutrace_dropped_events(void);

#ifdef __cplusplus
}
#endif

#endif  // KUTRACE_CLIENT_H_
