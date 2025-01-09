# Notes while reading

## Ask why and what along the way

Assumptions
32KB 8 way L1
256KB 8 way L2
3MB 12 way L3

```c
// Transpose matrix
void TransposeAll(const double *x, double *xprime)
{
  for (int row = 0; row < kRowsize; ++row)
  {
    for (int col = 0; col < kColsize; ++col)
    {
      xprime[col * kRowsize + row] = x[row * kRowsize + col];
      L1((uint64)&x[row * kRowsize + col]);
      L2((uint64)&x[row * kRowsize + col]);
      L3((uint64)&x[row * kRowsize + col]);
      L1((uint64)&xprime[col * kRowsize + row]);
      L2((uint64)&xprime[col * kRowsize + row]);
      L3((uint64)&xprime[col * kRowsize + row]);
    }
  }
}
```

So constructing transpose through, it's 1m misses (1024 * 1024) for reads + writes. But reading, cache line is 64 bytes (8 doubles), so it's 128K misses + 1M for writes (since transpose writes to 8K offsets each time).

Now, for iterating:

```c
// Transpose second input array to be in column-major order
void SimpleMultiplyTranspose(const double *a, const double *b, double *c)
{
  TransposeAll(b, bb);
  for (int row = 0; row < kRowsize; ++row)
  {
    for (int col = 0; col < kColsize; ++col)
    {
      c[row * kRowsize + col] = VectorSum1(&a[row * kRowsize + 0],
                                           &bb[col * kRowsize + 0],
                                           kRowsize, 1);
    }
  }
}
```

What is `a` misses per loop?
it's cached per loop, so only 1024 for outer loop
X
it's actually 8 elements cached, so 128.

What is `bb` misses per loop?
1024 / 8 elements caches = 128 and then times outer loop which is 1024 times = 128 * 1024 = ~131k?
WRONG
it's used per loop, so only first access, which is 128.

So both are multiplied by 1M because it's done 1M for outer 2 loops and 128 + 128 for one run of inner loop

```
// Transpose one block
kBlocksize = 8
void BlockTranspose(const double *x, double *xprime)
{
  for (int row = 0; row < kBlocksize; ++row)
  {
    for (int col = 0; col < kBlocksize; col += 4)
    {
      xprime[(col + 0) * kRowsize + row] = x[row * kRowsize + col + 0];
      xprime[(col + 1) * kRowsize + row] = x[row * kRowsize + col + 1];
      xprime[(col + 2) * kRowsize + row] = x[row * kRowsize + col + 2];
      xprime[(col + 3) * kRowsize + row] = x[row * kRowsize + col + 3];
    }
  }
}

void BlockTransposeAll(const double *x, double *xprime)
{
  for (int row = 0; row < kRowsize; row += kBlocksize)
  {
    for (int col = 0; col < kColsize; col += kBlocksize)
    {
      BlockTranspose(&x[row * kRowsize + col], &xprime[col * kRowsize + row]);
    }
  }
}


```
Reduces because one cache line is 64 (8 entries), and writing now to linear subarray, so writing is 128k misses and reading is the same (128k)

```

inline double VectorSum4(const double *aptr, const double *bptr, int count, int rowsize)
{
  const double *aptr2 = aptr;
  const double *bptr2 = bptr;
  double sum0 = 0.0;
  double sum1 = 0.0;
  double sum2 = 0.0;
  double sum3 = 0.0;
  for (int k = 0; k < count; k += 4)
  {
    sum0 += aptr2[0] * bptr2[0 * rowsize];
    sum1 += aptr2[1] * bptr2[1 * rowsize];
    sum2 += aptr2[2] * bptr2[2 * rowsize];
    sum3 += aptr2[3] * bptr2[3 * rowsize];

// Transpose second input array to be in column-major order
void SimpleMultiplyTransposeFast(const double *a, const double *b, double *c)
{
  BlockTransposeAll(b, bb);
  for (int row = 0; row < kRowsize; ++row)
  {
    for (int col = 0; col < kColsize; ++col)
    {
      c[row * kRowsize + col] = VectorSum4(&a[row * kRowsize + 0],
                                           &bb[col * kRowsize + 0],
                                           kRowsize, 1);
      L1((uint64)&c[row * kRowsize + col]);
      L2((uint64)&c[row * kRowsize + col]);
      L3((uint64)&c[row * kRowsize + col]);
    }
  }
}
```

stats
```
  Model name:             Intel(R) Xeon(R) Gold 6418H
    Core(s) per socket:   24
    Socket(s):            4
Caches (sum of all):      
  L1d:                    4.5 MiB (96 instances)
  L1i:                    3 MiB (96 instances)
  L2:                     192 MiB (96 instances)
  L3:                     240 MiB (4 instances)
NUMA:                     
  NUMA node(s):           4
  NUMA node0 CPU(s):      0,4,8,12,16,20,24,28,32,36,40,44,48,52,56,60,64,68,72,76,80,84,88,92
  NUMA node1 CPU(s):      1,5,9,13,17,21,25,29,33,37,41,45,49,53,57,61,65,69,73,77,81,85,89,93
  NUMA node2 CPU(s):      2,6,10,14,18,22,26,30,34,38,42,46,50,54,58,62,66,70,74,78,82,86,90,94
  NUMA node3 CPU(s):      3,7,11,15,19,23,27,31,35,39,43,47,51,55,59,63,67,71,75,79,83,87,91,95

Cache L1:	80 KB (per core)
Cache L2:	2 MB (per core)
Cache L3:	60 MB

(llm) henry@dassl-serv-01:/vectordb/KUtrace/bookcode$ numactl --cpunodebind=0 --membind=0  ./matrix_ku 

Equal
BlockTranspose Misses L1/L2/L3          0          0          0
SimpleMultiply                  5.540 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyColumnwise        5.871 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyTranspose         0.979 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyTransposeFast     0.536 seconds, sum=2494884076.030954838
Misses L1/L2/L3          0          0          0
BlockMultiplyRemap              0.486 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
IGNORE SimpleMultiplyOne        0.278 seconds, sum=    1024.003072004
Misses L1/L2/L3          0          0          0

g++ -O3 -mavx2 -mfma -fopenmp -march=native -funroll-loops -ffast-math matrix.cc kutrace_lib.cc -o matrix_ku

BlockTranspose Misses L1/L2/L3          0          0          0
SimpleMultiply                  5.523 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
SimpleMultiplyColumnwise        5.882 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
SimpleMultiplyTranspose         0.397 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
SimpleMultiplyTransposeFast     0.390 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
BlockMultiplyRemap              0.118 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
IGNORE SimpleMultiplyOne        0.096 seconds, sum=    1024.003072004
Misses L1/L2/L3          0          0          0

g++ -O3 -mavx2 -mfma -fopenmp -march=native -funroll-loops -ffast-math -fno-strict-aliasing -funroll-all-loops -fprefetch-loop-arrays matrix.cc kutrace_lib.cc -o matrix_ku


```

```
Vendor ID:                AuthenticAMD
  Model name:             AMD Ryzen 7 7800X3D 8-Core Processor
Virtualization features:  
  Virtualization:         AMD-V
Caches (sum of all):      
  L1d:                    256 KiB (8 instances)
  L1i:                    256 KiB (8 instances)
  L2:                     8 MiB (8 instances)
  L3:                     96 MiB (1 instance)

SimpleMultiply                  4.276 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyColumnwise        4.437 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyTranspose         0.627 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
SimpleMultiplyTransposeFast     0.168 seconds, sum=2494884076.030954838
Misses L1/L2/L3          0          0          0
BlockMultiplyRemap              0.235 seconds, sum=2494884076.030955315
Misses L1/L2/L3          0          0          0
IGNORE SimpleMultiplyOne        0.165 seconds, sum=    1024.003072004
Misses L1/L2/L3          0          0          0

g++ -O3 -mavx2 -mfma -fopenmp -march=native -funroll-loops -ffast-math matrix.cc kutrace_lib.cc -o matrix_ku

BlockTranspose Misses L1/L2/L3          0          0          0
SimpleMultiply                  3.869 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
SimpleMultiplyColumnwise        4.142 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
SimpleMultiplyTranspose         0.082 seconds, sum=2494884076.030890465
Misses L1/L2/L3          0          0          0
SimpleMultiplyTransposeFast     0.078 seconds, sum=2494884076.030890465
Misses L1/L2/L3          0          0          0
BlockMultiplyRemap              0.058 seconds, sum=2494884076.030889988
Misses L1/L2/L3          0          0          0
IGNORE SimpleMultiplyOne        0.034 seconds, sum=    1024.003072004
Misses L1/L2/L3          0          0          0
```

Speed 

![](images/ch3/compare.png)

Let's revisit this.
