#define _GNU_SOURCE

#include <dlfcn.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

typedef uint64_t (*late_probe_fn)(uint64_t);

static void sleep_ms(unsigned long milliseconds) {
  struct timespec delay = {
      .tv_sec = (time_t)(milliseconds / 1000),
      .tv_nsec = (long)(milliseconds % 1000) * 1000000L,
  };
  while (nanosleep(&delay, &delay) != 0 && errno == EINTR) {
  }
}

static unsigned long parse_number(const char *text, const char *name) {
  char *end = NULL;
  errno = 0;
  unsigned long value = strtoul(text, &end, 10);
  if (errno != 0 || end == text || *end != '\0') {
    fprintf(stderr, "invalid %s: %s\n", name, text);
    exit(2);
  }
  return value;
}

int main(int argc, char **argv) {
  if (argc != 5) {
    fprintf(stderr,
            "usage: %s LIBRARY PRELOAD_DELAY_MS ATTACH_DELAY_MS ITERATIONS\n",
            argv[0]);
    return 2;
  }
  const unsigned long preload_delay_ms = parse_number(argv[2], "preload delay");
  const unsigned long attach_delay_ms = parse_number(argv[3], "attach delay");
  const unsigned long iterations = parse_number(argv[4], "iterations");

  sleep_ms(preload_delay_ms);
  void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
  if (library == NULL) {
    fprintf(stderr, "dlopen failed: %s\n", dlerror());
    return 1;
  }
  dlerror();
  late_probe_fn probe = (late_probe_fn)dlsym(library, "kutrace_late_probe");
  const char *symbol_error = dlerror();
  if (symbol_error != NULL) {
    fprintf(stderr, "dlsym failed: %s\n", symbol_error);
    return 1;
  }

  sleep_ms(attach_delay_ms);
  volatile uint64_t result = 0;
  for (unsigned long index = 0; index < iterations; ++index) {
    result ^= probe((uint64_t)index);
  }
  printf("{\"iterations\":%lu,\"result\":%llu}\n", iterations,
         (unsigned long long)result);
  fflush(stdout);
  _Exit(0);
}
