// Sample mystery program to measure how long an add takes. Flawed.
// Copyright 2021 Richard L. Sites

#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include <x86intrin.h>

inline int64_t GetCycles()
{
  // Increments once per cycle, implemented as increment by N every N (~35) cycles
  return __rdtsc();
}

static const int kIterations = 1000 * 1000000;

int main(int argc, const char **argv)
{
  uint64_t sum = 0;

  int64_t startcy = __rdtsc();
  for (int i = 0; i < kIterations; i += 4)
  {
    sum += 1;
    sum += 1;
    sum += 1;
    sum += 1;
  }
  int64_t elapsed = __rdtsc() - startcy;

  double felapsed = elapsed;
  fprintf(stdout, "%d iterations, %lu cycles, %4.2f cycles/iteration\n",
          kIterations, elapsed, felapsed / kIterations);
  return 0;
}
