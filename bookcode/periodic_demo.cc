// Ch28 illustration (not a book program): periodic work via nanosleep.
// Each tick does ~200us of fake work, then sleeps for a 5ms period, so the
// trace shows the classic timer-wakeup periodicity of "waiting for time".
#include <time.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
int main(int argc, char** argv) {
  int ticks = (argc > 1) ? atoi(argv[1]) : 400;     // ~2s at 5ms
  int period_us = (argc > 2) ? atoi(argv[2]) : 5000; // 5ms period
  volatile double x = 1.0;
  for (int t = 0; t < ticks; ++t) {
    // ~200us of fake work
    for (int i = 0; i < 200000; ++i) { x /= 1.0000001; x *= 1.0000001; }
    struct timespec req = {0, (long)period_us * 1000};
    nanosleep(&req, NULL);    // <-- waiting for time (timer interrupt)
  }
  printf("%f\n", x);
  return 0;
}
