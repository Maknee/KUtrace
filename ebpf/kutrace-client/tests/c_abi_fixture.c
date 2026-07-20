#include "kutrace_client.h"

#include <assert.h>
#include <inttypes.h>
#include <stdio.h>

int main(void) {
  const uint64_t span = kutrace_span_begin("agent.c.tool");
  assert(span != 0);
  assert(kutrace_span_annotate(span, KUTRACE_ANNOTATION_QUERY, 55,
                              "c.abi") == 1);
  assert(kutrace_legacy_marker(KUTRACE_CLIENT_RESOURCE, 99, 0, 88,
                               "agent.c.resource") == 1);
  kutrace_span_end(span);
  const uint64_t dropped = kutrace_dropped_events();
  assert(dropped == 0);
  printf("{\"span_id\":%" PRIu64 ",\"client_events_dropped\":%" PRIu64
         "}\n",
         span, dropped);
  return 0;
}
