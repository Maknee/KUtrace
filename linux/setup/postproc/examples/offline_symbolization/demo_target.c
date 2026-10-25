#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

__attribute__((noinline)) uint64_t demo_hot_function(uint64_t value) {
  for (int i = 0; i < 10000; ++i) {
    value = (value * 6364136223846793005ULL) + 1;
  }
  return value;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s CAPTURE_FILE\n", argv[0]);
    return 2;
  }

  FILE *capture = fopen(argv[1], "w");
  if (capture == NULL) {
    perror("fopen capture");
    return 1;
  }
  fprintf(capture, "%ld\t0x%" PRIxPTR "\n", (long)getpid(),
          (uintptr_t)&demo_hot_function);
  if (fclose(capture) != 0) {
    perror("fclose capture");
    return 1;
  }

  volatile uint64_t result = demo_hot_function(7);
  return result == 0;
}
