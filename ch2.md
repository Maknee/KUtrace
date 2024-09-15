# Cycles

```c
// Sample mystery program to measure how long an add takes. Flawed.
// Copyright 2021 Richard L. Sites

#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include "timecounters.h"

static const int kIterations = 1000 * 1000000;

int main (int argc, const char** argv) {
  uint64_t sum = 0;

  int64_t startcy = GetCycles();
  for (int i = 0; i < kIterations; ++i) {
    sum += 1;
  }
  int64_t elapsed = GetCycles() - startcy;
	
  double felapsed = elapsed;
  fprintf(stdout, "%d iterations, %lu cycles, %4.2f cycles/iteration\n", 
          kIterations, elapsed, felapsed / kIterations);
  return 0;
}
```

```
(llm) maknee@maknee-gpu:/4nvme/code/KUtrace/bookcode$ ./mystery0
1000000000 iterations, 2002681380 cycles, 2.00 cycles/iteration
(llm) maknee@maknee-gpu:/4nvme/code/KUtrace/bookcode$ ./mystery0_opt 
1000000000 iterations, 42 cycles, 0.00 cycles/iteration
```

RDTSC gets the cyles between two samples.


## Unrolled

```c
// Sample mystery program to measure how long an add takes. Flawed.
// Copyright 2021 Richard L. Sites

#include <stdint.h>
#include <stdio.h>
#include <time.h>
#include "timecounters.h"

static const int kIterations = 1000 * 1000000;

int main(int argc, const char **argv)
{
  uint64_t sum = 0;

  int64_t startcy = GetCycles();
  for (int i = 0; i < kIterations; i += 4)
  {
    sum += 1;
    sum += 1;
    sum += 1;
    sum += 1;
  }
  int64_t elapsed = GetCycles() - startcy;

  double felapsed = elapsed;
  fprintf(stdout, "%d iterations, %lu cycles, %4.2f cycles/iteration\n",
          kIterations, elapsed, felapsed / kIterations);
  return 0;
}
```

```x86
.LC1:
        .string "%d iterations, %lu cycles, %4.2f cycles/iteration\n"
main:
        push    rbp
        mov     rbp, rsp
        sub     rsp, 64
        mov     DWORD PTR [rbp-52], edi
        mov     QWORD PTR [rbp-64], rsi
        mov     QWORD PTR [rbp-8], 0
        rdtsc
        sal     rdx, 32
        or      rax, rdx
        nop
        mov     QWORD PTR [rbp-24], rax
        mov     DWORD PTR [rbp-12], 0
        jmp     .L3
.L4:
        add     QWORD PTR [rbp-8], 1
        add     QWORD PTR [rbp-8], 1
        add     QWORD PTR [rbp-8], 1
        add     QWORD PTR [rbp-8], 1
        add     DWORD PTR [rbp-12], 4
.L3:
        cmp     DWORD PTR [rbp-12], 999999999
        jle     .L4
        rdtsc
        sal     rdx, 32
        or      rax, rdx
        mov     rdx, rax
        nop
        mov     rax, QWORD PTR [rbp-24]
        sub     rdx, rax
        mov     QWORD PTR [rbp-32], rdx
        pxor    xmm0, xmm0
        cvtsi2sd        xmm0, QWORD PTR [rbp-32]
        movsd   QWORD PTR [rbp-40], xmm0
        movsd   xmm0, QWORD PTR [rbp-40]
        movsd   xmm1, QWORD PTR .LC0[rip]
        divsd   xmm0, xmm1
        movq    rcx, xmm0
        mov     rax, QWORD PTR stdout[rip]
        mov     rdx, QWORD PTR [rbp-32]
        movq    xmm0, rcx
        mov     rcx, rdx
        mov     edx, 1000000000
        mov     esi, OFFSET FLAT:.LC1
        mov     rdi, rax
        mov     eax, 1
        call    fprintf
        mov     eax, 0
        leave
        ret
.LC0:
        .long   0
        .long   1104006501
```

[Godbolt](https://godbolt.org/#g:!((g:!((g:!((h:codeEditor,i:(filename:'1',fontScale:14,fontUsePx:'0',j:1,lang:c%2B%2B,selection:(endColumn:1,endLineNumber:28,positionColumn:1,positionLineNumber:28,selectionStartColumn:1,selectionStartLineNumber:28,startColumn:1,startLineNumber:28),source:'%0A%23include+%3Cstdint.h%3E%0A%23include+%3Cstdio.h%3E%0A%23include+%3Ctime.h%3E%0A%23include+%3Cx86intrin.h%3E%0A%0Astatic+const+int+kIterations+%3D+1000+*+1000000%3B%0A%0Aint+main(int+argc,+const+char+**argv)%0A%7B%0A++uint64_t+sum+%3D+0%3B%0A%0A++int64_t+startcy+%3D+__rdtsc()%3B%0A++for+(int+i+%3D+0%3B+i+%3C+kIterations%3B+i+%2B%3D+4)%0A++%7B%0A++++sum+%2B%3D+1%3B%0A++++sum+%2B%3D+1%3B%0A++++sum+%2B%3D+1%3B%0A++++sum+%2B%3D+1%3B%0A++%7D%0A++int64_t+elapsed+%3D+__rdtsc()+-+startcy%3B%0A%0A++double+felapsed+%3D+elapsed%3B%0A++fprintf(stdout,+%22%25d+iterations,+%25lu+cycles,+%254.2f+cycles/iteration%5Cn%22,%0A++++++++++kIterations,+elapsed,+felapsed+/+kIterations)%3B%0A++return+0%3B%0A%7D%0A'),l:'5',n:'1',o:'C%2B%2B+source+%231',t:'0')),k:50,l:'4',n:'0',o:'',s:0,t:'0'),(g:!((h:compiler,i:(compiler:g142,filters:(b:'0',binary:'1',binaryObject:'1',commentOnly:'0',debugCalls:'1',demangle:'0',directives:'0',execute:'1',intel:'0',libraryCode:'0',trim:'1',verboseDemangling:'0'),flagsViewOpen:'1',fontScale:14,fontUsePx:'0',j:1,lang:c%2B%2B,libs:!(),options:'',overrides:!(),selection:(endColumn:1,endLineNumber:1,positionColumn:1,positionLineNumber:1,selectionStartColumn:1,selectionStartLineNumber:1,startColumn:1,startLineNumber:1),source:1),l:'5',n:'0',o:'+x86-64+gcc+14.2+(Editor+%231)',t:'0')),k:50,l:'4',n:'0',o:'',s:0,t:'0')),l:'2',n:'0',o:'',t:'0')),version:4)

```
(llm) maknee@maknee-gpu:/4nvme/code/KUtrace/bookcode$ ./mystery0_unrolled
1000000000 iterations, 985141164 cycles, 0.99 cycles/iteration
(llm) maknee@maknee-gpu:/4nvme/code/KUtrace/bookcode$ ./mystery0_unrolled_opt 
1000000000 iterations, 42 cycles, 0.00 cycles/iteration
```

Gets twice as fast, overlapping instructions?

