// Names for syscall, etc. in dclab_tracing 
// dick sites 2019.03.13, 2020.07.10
// These are from linux-4.19.19 x86 AMD 64-bit. Others will vary.
//

#ifndef __KUTRACE_CONTROL_NAMES_H__
#define __KUTRACE_CONTROL_NAMES_H__

#if defined(__x86_64)
    #define Isx86_64 1
#else 
    #define Isx86_64 0
#endif

/* AMD Zen architecture checks */
#if defined(__znver1) || defined(__znver2) || defined(__znver3) || defined(__znver4) || defined(__znver5)
    #define Is_Znver 1
#else
    #define Is_Znver 0
#endif

/* ARM architecture checks */
#if defined(__aarch64__)
    #define IsArm_64 1
#else
    #define IsArm_64 0
#endif

#if defined(__ARM_ARCH) && (__ARM_ARCH == 8)
    #define IsRPi4 1
#else
    #define IsRPi4 0
#endif

/* Step 2: Define combined architecture macros using the 0/1 values */
#define IsAmd_64    (Isx86_64 && Is_Znver)
#define IsIntel_64  (Isx86_64 && !Is_Znver)
#define IsRPi4_64   (IsRPi4 && IsArm_64)

/* Use the macros */
#if IsAmd_64
#include "kutrace_control_names_ryzen.h"

#elif IsIntel_64
#include "kutrace_control_names_i3.h"

#elif IsRPi4_64
#include "kutrace_control_names_rpi4.h"

#else
#error Need control_names for your architecture
#endif

#endif	// __KUTRACE_CONTROL_NAMES_H__


