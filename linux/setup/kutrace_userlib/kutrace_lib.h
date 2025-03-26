// kutrace_lib.h 
// Copyright 2021 Richard L. Sites
//
// This is a simple interface for user-mode code to control kernel/user tracing and add markers
//

#ifndef __KUTRACE_LIB_H__
#define __KUTRACE_LIB_H__

#include <stdio.h>
#include <stdlib.h>     // exit, system
#include <string.h>
#include <time.h>	// nanosleep
#include <unistd.h>     // getpid gethostname syscall
#include <sys/time.h>   // gettimeofday
#include <sys/types.h>	

#if defined(__x86_64__)
#include <x86intrin.h>		// _rdtsc
#endif

// x should be declared uint8
#define POLY8 (0x1d)   // From CRC-8-SAE J1850
#define POLYSHIFT8(x) ( ((x) << 1) ^ static_cast<uint8>((static_cast<int8>((x)) >> 7) & POLY8) )
#define POLYINIT8 (0xffu)

// x should be declared uint32
#define POLY32 (0x04c11db7)   // From CRC-32
#define POLYSHIFT32(x) ( ((x) << 1) ^ static_cast<uint32>((static_cast<int32>((x)) >> 31) & POLY32) )
#define POLYINIT32 (0xffffffffu)

// x should be declared uint64
#define POLY64 (0x42F0E1EBA9EA3693lu)   // From CRC-64-ECMA
#define POLYSHIFT64(x) ( ((x) << 1) ^ static_cast<uint64>((static_cast<int64>((x)) >> 63) & POLY64) )
#define POLYINIT64 (0xfffffffffffffffflu)

typedef struct {
  int number;
  const char* name; 
} NumNamePair;

#include "basetypes.h"
#include "kutrace_control_names.h"	// PidNames, TrapNames, IrqNames, Syscall64Names

typedef uint32 u32;
typedef uint64 u64;
typedef int64  s64;

/* This is the definitive list of raw trace 12-bit event numbers */
// These user-mode declarations need to exactly match 
// source pool kutrace.h kernel-mode ones 

/* kutrace_control() commands */
#define KUTRACE_CMD_OFF 0
#define KUTRACE_CMD_ON 1
#define KUTRACE_CMD_FLUSH 2
#define KUTRACE_CMD_RESET 3
#define KUTRACE_CMD_STAT 4
#define KUTRACE_CMD_GETCOUNT 5
#define KUTRACE_CMD_GETWORD 6
#define KUTRACE_CMD_INSERT1 7
#define KUTRACE_CMD_INSERTN 8
#define KUTRACE_CMD_GETIPCWORD 9
#define KUTRACE_CMD_TEST 10
#define KUTRACE_CMD_VERSION 11
// Added 2023.02.13
#define KUTRACE_CMD_SET4KB 12
#define KUTRACE_CMD_GET4KB 13
#define KUTRACE_CMD_GETIPC4KB 14




// All events are single uint64 entries unless otherwise specified
// +-------------------+-----------+---------------+-------+-------+
// | timestamp         | event     | delta | retval|      arg0     |
// +-------------------+-----------+---------------+-------+-------+
//          20              12         8       8           16 

// Add KUTRACE_ and uppercase
#define KUTRACE_NOP             0x000
#define KUTRACE_RDTSC           0x001	// unused
#define KUTRACE_GETTOD          0x002	// unused

#define KUTRACE_VARLENLO        0x010
#define KUTRACE_VARLENHI        0x1FF

// Variable-length starting numbers. Only events 010-1FF are variable length
// Middle hex digit of event number is 2..8, giving total length of entry including first uint64
// The arg is the lock# or PID# etc. that this name belongs to.
// +-------------------+-----------+-------------------------------+
// | timestamp         | event     |              arg              |
// +-------------------+-----------+-------------------------------+
// |  character name, 1-56 bytes, NUL padded                       |
// +- - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -+
// ~                                                               ~
// +---------------------------------------------------------------+
//          20              12                    32 

// TimePair (DEFUNCT)
// +-------------------+-----------+-------------------------------+
// | timestamp         | event     |              arg              |
// +-------------------+-----------+-------------------------------+
// |   cycle counter value                                         |
// +---------------------------------------------------------------+
// |   matching gettimeofday value                                 |
// +---------------------------------------------------------------+
//          20              12                    32 


// Variable-length starting numbers. 
// Middle hex digit will become length in u64 words, 2..8
#define KUTRACE_FILENAME        0x001
#define KUTRACE_PIDNAME         0x002
#define KUTRACE_METHODNAME      0x003
#define KUTRACE_TRAPNAME        0x004
#define KUTRACE_INTERRUPTNAME   0x005
#define KUTRACE_TIMEPAIR        0x006	/* DEPRECATED */
#define KUTRACE_LOCKNAME        0x007	/* added 2019.10.25 */
#define KUTRACE_SYSCALL64NAME   0x008
#define KUTRACE_SYSCALL32NAME   0x00C
#define KUTRACE_ERRNONAME	0x00E
#define KUTRACE_PACKETNAME      0x100
#define KUTRACE_PC_TEMP         0x101	/* scaffolding 2020.01.29 now PC_U and PC_K */
#define KUTRACE_KERNEL_VER      0x102	/* Kernel version, uname -rv */
#define KUTRACE_MODEL_NAME      0x103	/* CPU model name, /proc/cpuinfo */
#define KUTRACE_HOST_NAME       0x104 	/* CPU host name */
#define KUTRACE_QUEUE_NAME      0x105 	/* Queue name */
#define KUTRACE_RES_NAME        0x106 	/* Arbitrary resource name */
#define KUTRACE_MARKE_NAME      0x107

// Specials are point events. Hex 200-220 currently. PC sample is outside this range
#define KUTRACE_USERPID         0x200	/* Context switch */
#define KUTRACE_RPCIDREQ        0x201	/* CPU is processing RPC# n request */
#define KUTRACE_RPCIDRESP       0x202	/* CPU is processing RPC# n response */
#define KUTRACE_RPCIDMID        0x203	/* CPU is processing RPC# n middle */
#define KUTRACE_RPCIDRXMSG      0x204	/* For display: RPC message received, approx packet time */
#define KUTRACE_RPCIDTXMSG      0x205	/* For display: RPC message sent, approx packet time */
#define KUTRACE_RUNNABLE        0x206	/* Make runnable */
#define KUTRACE_IPI             0x207	/* Send IPI */
#define KUTRACE_MWAIT           0x208	/* C-states: how deep to sleep */
#define KUTRACE_PSTATE          0x209	/* P-states: cpu freq sample in MHz increments */

// MARK_A,B,C arg is six base-40 chars NUL, A-Z, 0-9, . - /
// MARK_D     arg is unsigned int
// +-------------------+-----------+-------------------------------+
// | timestamp         | event     |              arg              |
// +-------------------+-----------+-------------------------------+
//          20              12                    32 

#define KUTRACE_MARKA           0x20A
#define KUTRACE_MARKB           0x20B
#define KUTRACE_MARKC           0x20C
#define KUTRACE_MARKD           0x20D
#define KUTRACE_LEFTMARK        0x20E	// Inserted by eventtospan
#define KUTRACE_RIGHTMARK       0x20F	// Inserted by eventtospan
#define KUTRACE_LOCKNOACQUIRE   0x210
#define KUTRACE_LOCKACQUIRE     0x211
#define KUTRACE_LOCKWAKEUP      0x212
#define KUTRACE_MARKE           0x213
        
// Added 2020.10.29
#define KUTRACE_RX_PKT          0x214 	/* Raw packet received w/32-byte payload hash */ 
#define KUTRACE_TX_PKT          0x215 	/* Raw packet sent w/32-byte payload hash */

#define KUTRACE_RX_USER         0x216 	/* Request beginning at user code w/32-byte payload hash */ 
#define KUTRACE_TX_USER         0x217 	/* Response ending at user code w/32-byte payload hash */
  
#define KUTRACE_MBIT_SEC        0x218 	/* Network rate in Mb/s */

#define KUTRACE_RESOURCE	    0x219  /* Arbitrary resource span; arg says which resource */
#define KUTRACE_ENQUEUE	 	    0x21A  /* Put RPC on a work queue; arg says which queue */
#define KUTRACE_DEQUEUE	 	    0x21B  /* Remove RPC from a queue; arg says which queue */
#define KUTRACE_PSTATE2         0x21C  /* P-states: cpu freq change, new in MHz increments */
// Added 2022.05.24
#define KUTRACE_TSDELTA         0x21D  /* Delta to advance timestamp */
#define KUTRACE_MONITORSTORE    0x21E  /* Store into a monitored location; does wakeup */
#define KUTRACE_MONITOREXIT     0x21F  /* Mwait exits due to store */

#define KUTRACE_MAX_SPECIAL     0x27F	// Last special, range 200..27F

// Extra events have duration, but are otherwise similar to specials
// PC sample. Not a special
#define KUTRACE_PC_U            0x280	/* added 2020.01.29 */
#define KUTRACE_PC_K            0x281	/* added 2020.02.01 */

// Lock held
#define KUTRACE_LOCK_HELD	    0x282	/* Inserted by eventtospan 2020.09.27 */
#define KUTRACE_LOCK_TRY	    0x283	/* Inserted by eventtospan 2020.09.27 */


/* Reasons for waiting, inserted only in postprocessing */
/* dsites 2019.10.25 */
#define KUTRACE_WAITA           0x300	/* a-z, through 0x0319 */
#define KUTRACE_WAITZ           0x319		

/* These are in blocks of 256 or 512 numbers */
#define KUTRACE_TRAP            0x400
#define KUTRACE_IRQ             0x500
#define KUTRACE_TRAPRET         0x600
#define KUTRACE_IRQRET          0x700
#define KUTRACE_SYSCALL64       0x800
#define KUTRACE_SYSRET64        0xA00
#define KUTRACE_SYSCALL32       0xC00
#define KUTRACE_SYSRET32        0xE00

/* Event numbers added in postprocessing or manually */
/*  -1 bracket, big } */
/*  -2 oval, fades out part of diagram */
/*  -3 arc, wakeup from one thread to another */
/*  -4 callout, bubble to label some event */
/*  -5 ... */

/* Specific trap number for device not available */
#define KUTRACE_DNA            7
/* Specific trap number for page fault */
#define KUTRACE_PAGEFAULT      14

/* Specific IRQ numbers. Originally from arch/x86/include/asm/irq_vectors.h */
#define KUTRACE_LOCAL_TIMER_VECTOR     0xec

/* Reuse the spurious_apic vector to show bottom halves executing */
#define KUTRACE_BOTTOM_HALF    255
#define AST_SOFTIRQ		    	15

#define RESCHEDULE_VECTOR      IPI_PREEMPT


// Names for events 000-00F could be added when one of these code points is
// actually used

// Names for the variable-length events 0y0-0yF and 1y0-1yF, where y is length in words 2..8
static const char* const kNameName[32] = {
  "-000-", "file", "pid", "rpc", 
  "trap", "irq", "trap", "irq",
  "syscall", "syscall", "syscall", "syscall",
  "syscall32", "syscall32", "syscall32", "syscall32",

  "packet", "pctmp", "kernv", "cpum",
  "host", "", "", "",
  "", "", "", "",
  "", "", "", "",
};

// Names for the special events 200-21F
static const char* const kSpecialName[32] = {
  "userpid", "rpcreq", "rpcresp", "rpcmid", 
  "rxmsg", "txmsg", "runnable", "sendipi",
  "mwait", "-freq-", "mark_a", "mark_b", 
  "mark_c", "mark_d", "-20e-", "-20f-", 
  "try_", "acq_", "rel_", "mark_e",		// Locks
  "rx", "tx", "urx", "utx",
  "mbs", "res", "enq", "deq",
  "-21c-", "tsdelta", "mon_st", "mon_ex",
};

// Names for events 210-3FF could be added when one of these code points is
// actually used

// Names for events 400-FFF are always embedded in the trace

// x86- and ARM-specific Names for return codes -128 to -1
// If errno is in [-128..-1], subscript this by -errno - 1. 
// Error -1 EPERM thus maps to kErrnoName[0], not [1]
// See include/uapi/asm-generic/errno-base.h
// See include/uapi/asm-generic/errno.h
// ...more could be added
static const char* const kErrnoName[128] = {
  "EPERM", "ENOENT", "ESRCH", "EINTR", "EIO", "ENXIO", "E2BIG", "ENOEXEC",
  "EBADF", "ECHILD", "EAGAIN", "ENOMEM", "EACCES", "EFAULT", "ENOTBLK", "EBUSY",
  "EEXIST", "EXDEV", "ENODEV", "ENOTDIR", "EISDIR", "EINVAL", "ENFILE", "EMFILE",
  "ENOTTY", "ETXTBSY", "EFBIG", "ENOSPC", "ESPIPE", "EROFS", "EMLINK", "EPIPE",

  "EDOM", "ERANGE", "EDEADLK", "ENAMETOOLONG", "ENOLCK", "ENOSYS", "ENOTEMPTY", "ELOOP", 
  "", "ENOMSG", "EIDRM", "ECHRNG", "EL2NSYNC", "EL3HLT", "EL3RST", "ELNRNG", 
  "EUNATCH", "ENOCSI", "EL2HLT", "EBADE", "EBADR", "EXFULL", "ENOANO", "EBADRQC", 
  "EBADSLT", "", "EBFONT", "ENOSTR", "ENODATA", "ETIME", "ENOSR", "ENONET", 

  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 

  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 
  "", "", "", "", "", "", "", "", 
};

namespace kutrace {
  inline bool test();
  inline void go(const char* process_name);
  inline void goipc(const char* process_name);
  inline void goipcwrap(const char* process_name);
  inline void stop(const char* fname);
  inline void mark_a(const char* label);
  inline void mark_b(const char* label);
  inline void mark_c(const char* label);
  inline void mark_d(u64 n);
  inline void mark_e(const char* label, u64 num);

  // Returns number of words inserted 1..8, or
  //   0 if tracing is off, negative if module is not not loaded 
  inline u64 addevent(u64 eventnum, u64 arg);
  inline void addname(u64 eventnum, u64 number, const char* name);

  inline void msleep(int msec);
  inline int64 readtime();

  inline const char* Base40ToChar(u64 base40, char* str);
  inline u64 CharToBase40(const char* str);

  inline u64 DoControl(u64 command, u64 arg);
  inline void DoDump(const char* fname);
  inline u64 DoEvent(u64 eventnum, u64 arg);
  inline void DoFlush();
  inline void DoInit(const char* process_name);
  inline void DoMark(u64 n, u64 arg);
  inline bool DoTest();
  inline bool DoOff();
  inline bool DoOn();
  inline void DoQuit();
  inline void DoReset(u64 doing_ipc);
  inline void DoStat(u64 control_flags);
  inline void EmitNames(const NumNamePair* ipair, u64 n);
  inline u64 GetUsec();
  inline const char* MakeTraceFileName(const char* name, char* str);
  inline bool TestModule();
}


// All the real stuff is inside this anonymous namespace
namespace KutraceInternal {

/* Outgoing arg to DoReset  */
#define DO_IPC 1
#define DO_WRAP 2
#define DO_LLC 4

/* For the flags byte in traceblock[1] */
#define IPC_Flag     CLU(0x80)
#define WRAP_Flag    CLU(0x40)
#define Unused2_Flag CLU(0x20)
#define Unused1_Flag CLU(0x10)
#define VERSION_MASK CLU(0x0F)


// Module/code must be at least this version number for us to run
inline const u64 kMinModuleVersionNumber = 3;

// This defines the format of the resulting trace file
inline const u64 kTracefileVersionNumber = 3;

// Number of u64 values per trace block
inline const int kTraceBufSize = 8192;

// Number of u64 values per IPC block, one u8 per u64 in trace buf
inline const int kIpcBufSize = kTraceBufSize >> 3;

// For wraparound fixup on Raspberry Pi-4B Arm-v7
inline const int mhz_32bit_cycles = 54;

// Globals for mapping cycles to gettimeofday
inline static int64 start_cycles = 0;
inline static int64 stop_cycles = 0;
inline static int64 start_usec = 0;
inline static int64 stop_usec = 0;

inline static char kernelversion[256];
inline static char modelname[256];

// Useful utility routines
inline int64 GetUsec() {
  struct timeval tv; gettimeofday(&tv, NULL);
  return (tv.tv_sec * CL(1000000)) + tv.tv_usec;
}



/*x86-64 or Arm-specific timer */
/* Arm-64 returns 32MHz counts: 31.25 ns each */
/* Arm-32 Raspberry Pi4B 54MHz counts: 18.52 nsec */
/* x86-64 version returns rdtsc() >> 6 to give ~20ns resolution */
inline u64 ku_get_cycles(void)
{
	u64 timer_value;
#if defined(__aarch64__)
	asm volatile("mrs %0, cntvct_el0" : "=r"(timer_value));
#elif defined(__ARM_ARCH_ISA_ARM)
	/* This 32-bit result at 54 MHz RPi4 wraps every 75 seconds */
	asm volatile("mrrc p15, 1, %Q0, %R0, c14" : "=r" (timer_value));
	timer_value &= CLU(0x00000000FFFFFFFF);
#elif defined(__x86_64__)
	timer_value = _rdtsc() >> 6;
#else
	BUILD_BUG_ON_MSG(1, "Define the time base for your architecture");
#endif
	return timer_value;
}



// Read time counter and gettimeofday() close together, returning both
inline void GetTimePair(int64* cycles, int64* usec) {
  int64 startcy, stopcy;
  int64 gtodusec, elapsedcy;
  // Do more than once if we get an interrupt or other big delay in the middle of the loop
  do {
    startcy = ku_get_cycles();
    gtodusec = GetUsec();
    stopcy = ku_get_cycles();
    elapsedcy = stopcy - startcy;
    // In a quick test on an Intel i3 chip, GetUsec() took about 150 cycles (50 nsec)
    //  Perhaps 4x this on Arm chips
    // printf("%ld elapsed cycles\n", elapsedcy);
  } while (elapsedcy > 320);  // About 10 usec at 32MHz
  *cycles = startcy;
  *usec = gtodusec;
}


// For the trace_control system call,
// arg is declared to be u64. In reality, it is either a u64 or
// a pointer to a u64, depending on the command. Caller casts as
// needed, and the command implementations in kutrace_mod
// cast back as needed.

/* VERYTEMP to see result values */
// u32 swi_ret0, swi_ret1;

// These numbers must exactly match the numbers in include/linux/kutrace.h
#define __NR_kutrace_control 1023
#define KUTRACE_SCHEDSYSCALL 511

#if defined(__ARM_ARCH_ISA_ARM) && !defined(__aarch64__)

#define noinline        __attribute__((noinline))
u64 noinline DoControl(u64 command, u64 arg)
{
	/* gcc -O2 removes all the crap and makes 5 instructions! */
	/* str r7; ldr r7; swi; ldr r7; bx */
        /* Order avoids overwriting r0, r2 too soon */
	register u32 command_hi asm("r1") = (u32) (command >> 32);
        register u32 command_lo asm("r0") = (u32)(command & 0xffffffff);
	register u32 arg_hi asm("r3") = (u32)(arg >> 32);
	register u32 arg_lo asm("r2") = (u32)(arg & 0xffffffff);
        register u32 ret0 asm ("r0");
        register u32 ret1 asm ("r1");
        register u32 nr asm("r7") = __NR_kutrace_control;

        asm volatile(
        "       swi #0\n"
        : "=r" (ret0), "=r" (ret1)
        : "r" (command_lo), "r" (command_hi), "r" (arg_lo), "r" (arg_hi), "r" (nr)
        : "memory");
// swi_ret0 = ret0;
// swi_ret1 = ret1;
        return ((u64)ret1 << 32) | (u64)ret0;
}

#else

u64 inline DoControl(u64 command, u64 arg)
{
  return syscall(__NR_kutrace_control, command, arg);
}

#endif

// X86-64 inline version
//    u64 retval;
//    asm volatile
//    (
//        "syscall"
//        : "=a" (retval)
//        : "0"(__NR_dclab_control), "D"(command), "S"(arg)
//        : "cc", "rcx", "r11", "memory"
//    );
//    return retval;

// Sleep for n milliseconds
inline void msleep(int msec) {
  struct timespec ts;
  ts.tv_sec = msec / 1000;
  ts.tv_nsec = (msec % 1000) * 1000000;
  nanosleep(&ts, NULL);
}

// Single inline static buffer. In real production code, this would 
// all be std::string value, or something else at least as safe.
inline const int kMaxDateTimeBuffer = 32;
inline static char gTempDateTimeBuffer[kMaxDateTimeBuffer];

// Turn seconds since the epoch into yyyymmdd_hhmmss
// Not valid after January 19, 2038
inline const char* FormatSecondsDateTime(int32 sec) {
  // if (sec == 0) {return "unknown";}  // Longer spelling: caller expecting date
  time_t tt = sec;
  struct tm* t = localtime(&tt);
  sprintf(gTempDateTimeBuffer, "%04d%02d%02d_%02d%02d%02d",
         t->tm_year + 1900, t->tm_mon + 1, t->tm_mday,
         t->tm_hour, t->tm_min, t->tm_sec);
  return gTempDateTimeBuffer;
}

// Construct a name for opening a trace file, using name of program from command line
//   name: program_time_host_pid
// str should hold at least 256 bytes
inline const char* MakeTraceFileName(const char* argv0, char* str) {
  const char* slash = strrchr(argv0, '/');
  // Point to first char of image name
  if (slash == NULL) {
    slash = argv0;
  } else {
    slash = slash + 1;  // over the slash
  }

  const char* timestr;
  time_t tt = time(NULL);
  timestr = FormatSecondsDateTime(tt);

  char hostnamestr[256];
  gethostname(hostnamestr, 256) ;
  hostnamestr[255] = '\0';

  int pid = getpid();

  sprintf(str, "%s_%s_%s_%d.trace", slash, timestr, hostnamestr, pid);
  return str; 
}           

// This depends on ~KUTRACE_CMD_INSERTN working even with tracing off. 
inline void InsertVariableEntry(const char* str, u64 event, u64 arg) {
  u64 temp[8];		// Up to 56 bytes
  u64 bytelen = strlen(str);
  if (bytelen > 56) {bytelen = 56;}	// If too long, truncate
  u64 wordlen = 1 + ((bytelen + 7) / 8);
  // Build the initial word
  u64 event_with_length = event + (wordlen * 16);
  //         T               N                           ARG
  temp[0] = (CLU(0) << 44) | (event_with_length << 32) | arg;
  memset(&temp[1], 0, 7 * sizeof(u64));
  memcpy((char*)&temp[1], str, bytelen);
  DoControl(~KUTRACE_CMD_INSERTN, (u64)&temp[0]);
}

// Add a list of names to the trace
inline void EmitNames(const NumNamePair* ipair, u64 event) {
  u64 temp[9];		// One extra word for strcpy(56 bytes + '\0')
  const NumNamePair* pair = ipair;
  while (pair->name != NULL) {
    InsertVariableEntry(pair->name, event, pair->number);
    ++pair;
  }
}



// This depends on ~TRACE_INSERTN working even with tracing off. 
inline void InsertTimePair(int64 cycles, int64 usec) {
  u64 temp[8];		// Always 8 words for TRACE_INSERTN
  u64 n_with_length = KUTRACE_TIMEPAIR + (3 << 4);
  temp[0] = (CLU(0) << 44) | (n_with_length << 32);
  temp[1] = cycles;
  temp[2] = usec;
  DoControl(~KUTRACE_CMD_INSERTN, (u64)&temp[0]);
}



// Return false if the module is not loaded or too old. No delay. No side effect on time.
inline bool TestModule() {
  // If module is not loaded, syscall 511 returns -1 or -ENOSYS (= -38)
  // Unsigned, these are bigger than the biggest plausible version number, 255
  u64 retval = DoControl(KUTRACE_CMD_VERSION, 0);
// VERYTEMP
// fprintf(stderr, "TestModule %08lx %08lx\n", swi_ret0, swi_ret1);

  if (retval > 255) {
    // Module is not loaded
    fprintf(stderr, "KUtrace module/code not loaded\n");
    return false;
  }
  if (retval < kMinModuleVersionNumber) {
    // Module is loaded but older version
    fprintf(stderr, "KUtrace module/code is version %lld. Need at least %lld\n",
      retval, kMinModuleVersionNumber);
    return false;
  }
  //fprintf(stderr, "KUtrace module/code is version %ld.\n", retval);
  return true;
}


// Return true if module is loaded and tracing is on, else false
// CMD_TEST returns -ENOSYS (= -38) if not a tracing kernel
// else returns 0 if tracing is off
// else returns 1 if tracing is on
inline bool DoTest() {
  u64 retval = DoControl(KUTRACE_CMD_TEST, 0);
  if ((int64)retval < 0) {
    // KUtrace module/code is not available
    fprintf(stderr, "KUtrace module/code not available\n");
    return false;
  }
  return (retval == 1);
}

// Turn off tracing
// Complain and return false if module is not loaded
inline bool DoOff() {
  u64 retval = DoControl(KUTRACE_CMD_OFF, 0);
//fprintf(stderr, "DoOff DoControl = %016lx\n", retval);

  msleep(20);	/* Wait 20 msec for any pending tracing to finish */
  if (retval != 0) {
    // Module is not loaded
    fprintf(stderr, "KUtrace module/code not available\n");
    return false;
  }
  // Get stop time pair with tracing off
  if (stop_usec == 0) {GetTimePair(&stop_cycles, &stop_usec);}
//fprintf(stdout, "DoOff  GetTimePair %lx %lx\n", stop_cycles, stop_usec);
  return true;
}

// Turn on tracing
// Complain and return false if module is not loaded
inline bool DoOn() {
//fprintf(stderr, "DoOn\n");
  // Get start time pair with tracing off
  if (start_usec == 0) {GetTimePair(&start_cycles, &start_usec);}
//fprintf(stderr, "DoOn   GetTimePair %lx %lx\n", start_cycles, start_usec);
  u64 retval = DoControl(KUTRACE_CMD_ON, 0);
//fprintf(stderr, "DoOn DoControl = %016lx\n", retval);
  if (retval != 1) {
    // Module is not loaded
    fprintf(stderr, "KUtrace module/code not available\n");
    return false;
  }
  return true;
}

inline void StripCRLF(char* s) {
  int len = strlen(s);
  if ((0 < len) && s[len - 1] == '\n') {s[len - 1] = '\0'; --len;}
  if ((0 < len) && s[len - 1] == '\r') {s[len - 1] = '\0'; --len;}
}


// We want to run all this stuff at kutrace_control startup and/or at reset, but just once
// per execution is sufficient once these values don't change until reboot.
// Current design is two-step:
//   one routine to capture all the info
//   second routine to insert into trace buffer
// We want the inserts to be fast and have no delays that might allow a process 
// migration in the *middle* of building the initial name list. Doing so will 
// confuse wraparound and my leave some events unnamed for the first few 
// housand rawtoevent entries if some CPU blocks precede the remainder of
//  the name entries.

//   Linux kernel version
//   CPU model name
//   Hostname
//   Network link speed
//   Interrupt number to name mapping
//

// TODO: Get interrupt <number, name> pairs from /proc/interrupt
// TODO: Get hostname from  system call gethostname
// TODO: Get Ethernet link speed from /sys/class/net/*/speed

/***
// Get next interrupt description line from xxx, if any, and return 
// interrupt number and name and return value of true.
// If no more lines, return false
//
// Expecting:
// cat /proc/interrupts
//            CPU0       CPU1       
//   0:         20          0   IO-APIC   2-edge      timer
//   1:          3          0   IO-APIC   1-edge      i8042
//   8:          1          0   IO-APIC   8-edge      rtc0

inline bool NextIntr(FILE* procintrfile, int* intrnum, char* intrname) {
  char buffer[kMaxBufferSize];
  while (ReadLine(procintrfile, buffer, kMaxBufferSize)) {
    int n = sscanf(buffer, "%d:", intrnum);
    if (n != 1) {continue;}			// No intr on this line
    const char* space = strrchr(buffer, ' ');
    if (space == NULL) {continue;}		// No name on this line
    if (space[1] == '\0') {continue;}		// Empty name on this line
    strncpy(intrname, space + 1, kMaxBufferSize);
    return true;
  }
 
  return false;
}

// Litte code to read /proc/interrupts and capture names for each interrupt number
// dsites 2017.12.15
//
// use InsertVariableEntry now 2020.11.12
//
inline void EmitLocalIrqNames(u64 n) {
  FILE* procintrfile = fopen("/proc/interrupts", "r");
  if (procintrfile == NULL) {return;}

  u64 temp[9];		// One extra word for strcpy(56B + \n)
  int intrnum;
  char intrname[kMaxBufferSize];
  while (NextIntr(procintrfile, &intrnum, intrname)) {
    u64 bytelen = strlen(intrname);
    if (bytelen > 55) {continue;}	// Error if too long. Drop
    u64 wordlen = 1 + ((bytelen + 7) / 8);
    // Build the initial word
    u64 n_with_length = n + (wordlen * 16);
    //         T             N                       ARG
    temp[0] = (0ll << 44) | (n_with_length << 32) | (intrnum);
    memset(&temp[1], 0, 8 * sizeof(u64));
    strcpy((char*)&temp[1], intrname);
    DoControl(~TRACE_INSERTN, (u64)&temp[0]);
  }
  fclose(procintrfile);
}
***/


// Kernel version is the result of command: uname -rv
inline void GetKernelVersion(char* kernelversion, int len) {
  kernelversion[0] = '\0';
  FILE *fp = popen("uname -rv", "r");
  if (fp == NULL) {return;}
  char* s = fgets(kernelversion, len, fp);
  pclose(fp);
  if (s != NULL) {
    StripCRLF(kernelversion);
  }
}

// Model number is in /proc/cpuinfo
inline void GetModelName(char* modelname, int len) {
  modelname[0] = '\0';
  FILE *cpuinfo = fopen("/proc/cpuinfo", "rb");
  if (cpuinfo == NULL) {return;}
  char *arg = NULL;
  size_t size = 0;
  while(getline(&arg, &size, cpuinfo) != -1)
  {
    // Expecting something like
    // model name	: ARMv7 Processor rev 3 (v7l)
    if(memcmp(arg, "model name", 10) == 0) {
      const char* colon = strchr(arg, ':');
      if (colon != NULL) {	// Skip the colon and the next space
        StripCRLF(arg);
        strncpy(modelname, colon + 2, len);
        modelname[len - 1] = '\0';
        break;			// Just the first one, then get out
      }
    }
  }
  free(arg);
  fclose(cpuinfo);
}

// Inserts result of uname -rv
inline void InsertKernelVersion(const char* kernelversion) {
  InsertVariableEntry(kernelversion, KUTRACE_KERNEL_VER, 0);
}

inline void InsertModelName(const char* modelname) {
  InsertVariableEntry(modelname, KUTRACE_MODEL_NAME, 0);
}

// Initialize trace buffer with syscall/irq/trap names
// and processor model name, uname -rv
// Module must be loaded. Tracing must be off
inline void DoInit(const char* process_name) {
//fprintf(stderr, "DoInit\n");
  if (!TestModule()) {return;}		// No module loaded

  // AHHA. These take more than 10msec to execute. so 20-bit time can wrap.
  // We need to capture the strings up front before creating the frist trace entry,
  // then insert later.
  GetKernelVersion(kernelversion, 256);		// Slow. Must do first
  GetModelName(modelname, 256);			// Slow. Must do first
  GetTimePair(&start_cycles, &start_usec);	// Now OK to look at time

//fprintf(stderr, "DoInit GetTimePair %lx %lx\n", start_cycles, start_usec);
  // Insert the timepair as a trace entry. 
  // This is a hedge against separate programs starting (wraparound) tracing 
  // and stopping tracing. If this happens, start_usec will be zero at DoOff().
  
  // DEPRECATED
  // We want this to be the very first trace entry so we can find it easily
  InsertTimePair(start_cycles, start_usec); 

  // A little trace environment information
  InsertKernelVersion(kernelversion);
  InsertModelName(modelname);

  // Put trap/irq/syscall names into front of trace
  EmitNames(PidNames, KUTRACE_PIDNAME);
  EmitNames(TrapNames, KUTRACE_TRAPNAME);
  EmitNames(IrqNames, KUTRACE_INTERRUPTNAME);
  EmitNames(Syscall64Names, KUTRACE_SYSCALL64NAME);
  EmitNames(Syscall32Names, KUTRACE_SYSCALL32NAME);

  // Put current pid name into front of real part of trace
  int pid = getpid() & 0x0000ffff;
  InsertVariableEntry(process_name, KUTRACE_PIDNAME, pid);

  // And then establish that pid on this CPU
  //         T             N                       ARG
  u64 temp = (CLU(0) << 44) | ((u64)KUTRACE_USERPID << 32) | (pid);
  DoControl(~KUTRACE_CMD_INSERT1, temp);
}

// With tracing off, zero out the rest of each partly-used traceblock
// Module must be loaded. Tracing must be off
inline void DoFlush() {
//fprintf(stderr, "DoFlush\n");
  if (!TestModule()) {return;}		// No module loaded
  DoControl(KUTRACE_CMD_FLUSH, 0);
//fprintf(stderr, "DoFlush DoControl returned\n");
}

// Set up for a new tracing run
// Module must be loaded. Tracing must be off
inline void DoReset(u64 control_flags) {
  if (!TestModule()) {return;}		// No module loaded
  DoControl(KUTRACE_CMD_RESET, control_flags);

  start_usec = 0;
  stop_usec = 0;
  start_cycles = 0;
  stop_cycles = 0;
}

// Show some sort of tracing status
// Module must be loaded. Tracing may well be on
// If IPC,only 7/8 of the blocks are counted: 
//  for every 64KB traceblock there is another 8KB IPCblock (and some wasted space)
inline void DoStat(u64 control_flags) {
  u64 retval = DoControl(KUTRACE_CMD_STAT, 0);
  double blocksize = kTraceBufSize * sizeof(u64);
  if ((control_flags & DO_IPC) != 0) {blocksize = (blocksize * 8) / 7;}
  fprintf(stderr, "Stat: %lld trace blocks used (%3.1fMB)\n", 
          retval, (retval * blocksize) / (1024 * 1024));
}

// Called with the very first trace block, moduleversion >= 3
// This block has 12 words on the front, then a 3-word TimePairNum trace entry
inline void ExtractTimePair(u64* traceblock, int64* fallback_cycles, int64* fallback_usec) {
  u64 entry0 =       traceblock[12];
  u64 entry0_event = (entry0 >> 32) & 0xFFF;
  if ((entry0_event & 0xF0F) != KUTRACE_TIMEPAIR) {	// take out length nibble
    fprintf(stderr, "ExtractTimePair missing event\n");
    *fallback_cycles = 0;
    *fallback_usec =   0;
    return;
  }
  *fallback_cycles = traceblock[13];
  *fallback_usec =   traceblock[14];
}

// F(cycles) gives usec = base_usec + (cycles - base_cycles) * m;
typedef struct {
  u64 base_cycles;
  u64 base_usec;
  double m_slope;
} CyclesToUsecParams;

inline void SetParams(int64 start_cycles, int64 start_usec, 
               int64 stop_cycles, int64 stop_usec, CyclesToUsecParams* param) {
  param->base_cycles = start_cycles;
  param->base_usec = start_usec;
  if (stop_cycles <= start_cycles) {stop_cycles = start_cycles + 1;}	// ainline void zdiv
  param->m_slope = (stop_usec - start_usec) * 1.0 / (stop_cycles - start_cycles);
}

inline int64 CyclesToUsec(int64 cycles, const CyclesToUsecParams& param) {
  int64 delta_usec = (cycles - param.base_cycles) * param.m_slope;
  return param.base_usec + delta_usec;
}



// Dump the trace buffer to filename
// Module must be loaded. Tracing must be off
inline void DoDump(const char* fname) {
  // if (!TestModule()) {return;}		// No module loaded
  DoControl(KUTRACE_CMD_FLUSH, 0);

  // Start timepair is set by DoInit
  // Stop timepair is set by DoOff
  // If start_cycles is zero, we got here directly without calling DoInit, 
  // which was done in some earlier run of this program. In that case, go 
  // find the start pair as the first real trace entry in the first trace block.
  CyclesToUsecParams params;

  FILE* f = fopen(fname, "wb");
  if (f == NULL) {
    fprintf(stderr, "%s did not open\n", fname);
    return;
  }

  u64 traceblock[kTraceBufSize];
  u64 ipcblock[kIpcBufSize];
  // Get number of trace blocks
  // If tracing wraped around, the count is complemented
  bool did_wrap_around = false;
  u64 wordcount = DoControl(KUTRACE_CMD_GETCOUNT, 0);
  if ((s64)wordcount < 0) {
    wordcount = ~wordcount;
    did_wrap_around = true;
  }
  u64 blockcount = wordcount >> 13;
//fprintf(stderr, "wordcount = %ld\n", wordcount);
//fprintf(stderr, "blockcount = %ld\n", blockcount);

  // Loop on trace blocks
  for (int i = 0; i < blockcount; ++i) {
    u64 k = i * kTraceBufSize;  // Trace Word number to fetch next
    u64 k2 = i * kIpcBufSize;  	// IPC Word number to fetch next

    // Extract 64KB trace block
    for (int j = 0; j < kTraceBufSize; ++j) {
      traceblock[j] = DoControl(KUTRACE_CMD_GETWORD, k++);
    }

    // traceblock[0] has cpu number and cycle counter
    // traceblock[1] has flags in top byte, then zeros
    // We put the reconstructed getimeofday value into traceblock[1] 
    uint8 flags = traceblock[1] >> 56;
    bool this_block_has_ipc = ((flags & IPC_Flag) != 0);

    // For very first block, insert value of m as a double, for dump program to use
    // and clear traceblock[3], reserved for future use
    bool very_first_block = (i == 0);
    if (very_first_block) {
      // Fill in the tracefile version 
      traceblock[1] |= ((kTracefileVersionNumber & VERSION_MASK) << 56);
      if (!did_wrap_around) {
        // The kernel exports the wrap flag in the first block before 
        // it is known whether the trace actually wrapped.
        // It did not, so turn off that bit
        traceblock[1] &= ~(WRAP_Flag << 56);
      }
      // Extract the fallback start timepair
      int64 fallback_usec, fallback_cycles;
      ExtractTimePair(traceblock, &fallback_cycles, &fallback_usec);
      if (start_usec == 0) {
        start_usec = fallback_usec;
        start_cycles = fallback_cycles;
      }

      // For Arm-32, the "cycle" counter is only 32 bits at 54 MHz, so wraps about every 79 seconds.
      // This can leave stop_cycles small by a few multiples of 4G. We do a temporary fix here
      // for exactly 54 MHz. Later, we could find or take as input a different approximate
      // frequency. We could also do something similar for a 40-bit counter.
      bool has_32bit_cycles = ((start_cycles | stop_cycles) & 0xffffffff00000000llu) == 0;
      if (has_32bit_cycles) {
        uint64 elapsed_usec = (uint64)(stop_usec - start_usec);
        uint64 elapsed_cycles = (uint64)(stop_cycles - start_cycles);
        uint64 expected_cycles = elapsed_usec * mhz_32bit_cycles;
        // Pick off the expected high bits
        uint64 approx_hi = (start_cycles + expected_cycles) & 0xffffffff00000000llu;
        // Put them in
        stop_cycles |= (int64)approx_hi;
        // Cross-check and change by 1 if right at a boundary
        // and off by more than 12.5% from expected MHz
        elapsed_cycles = (uint64)(stop_cycles - start_cycles);
        uint64 ratio = elapsed_cycles / elapsed_usec;
        if (ratio > (mhz_32bit_cycles + (mhz_32bit_cycles >> 3))) {stop_cycles -= 0x0000000100000000llu;}
        if (ratio < (mhz_32bit_cycles - (mhz_32bit_cycles >> 3))) {stop_cycles += 0x0000000100000000llu;}
        elapsed_cycles = (uint64)(stop_cycles - start_cycles);
      }

      uint64 block_0_cycle = traceblock[0] & CLU(0x00ffffffffffffff);

      // Get ready to reconstruct gettimeofday values for each traceblock
      SetParams(start_cycles, start_usec, stop_cycles, stop_usec, &params);

      // Fill in the start/stop timepairs we are using, so
      // downstream programs can also SetParams
      traceblock[2] = start_cycles;
      traceblock[3] = start_usec;
      traceblock[4] = stop_cycles;
      traceblock[5] = stop_usec;
    }

    // Reconstruct the gettimeofday value for this block
    int64 block_cycles = traceblock[0] & CLU(0x00ffffffffffffff);
    int64 block_usec = CyclesToUsec(block_cycles, params);
    traceblock[1] |= (block_usec &  CLU(0x00ffffffffffffff));
    fwrite(traceblock, 1, sizeof(traceblock), f);

    // For each 64KB traceblock that has IPC_Flag set, also read the IPC bytes
    if (this_block_has_ipc) {
      // Extract 8KB IPC block
      for (int j = 0; j < kIpcBufSize; ++j) {
        ipcblock[j] = DoControl(KUTRACE_CMD_GETIPCWORD, k2++);
      }
      fwrite(ipcblock, 1, sizeof(ipcblock), f);
    }
  }
  fclose(f);

  fprintf(stdout, "  %s written (%3.1fMB)\n", fname, blockcount / 16.0);

  // Go ahead and set up for another trace
  DoControl(KUTRACE_CMD_RESET, 0);
}



// Exit this program
// Tracing must be off
inline void DoQuit() {
  DoOff();
  exit(0);
}

// Add a name of type n, value number, to the trace
inline void addname(uint64 eventnum, uint64 number, const char* name) {
  u64 temp[8];		// Buffer for name entry
  u64 bytelen = strlen(name);
  if (bytelen > 55) {bytelen = 55;}
  u64 wordlen = 1 + ((bytelen + 7) / 8);
  // Build the initial word
  u64 n_with_length = eventnum + (wordlen * 16);
  //             T             N                       ARG
  temp[0] = (CLU(0) << 44) | (n_with_length << 32) | (number);
  memset((char*)&temp[1], 0, 7 * sizeof(u64));
  memcpy((char*)&temp[1], name, bytelen);
  kutrace::DoControl(KUTRACE_CMD_INSERTN, (u64)&temp[0]);
}

// Create a Mark entry
inline void DoMark(u64 n, u64 arg) {
  //         T             N                       ARG
  u64 temp = (CLU(0) << 44) | (n << 32) | (arg &  CLU(0x00000000FFFFFFFF));
  DoControl(KUTRACE_CMD_INSERT1, temp);
}

// Create an arbitrary entry, returning 1 if tracing is on, <=0 otherwise
inline u64 DoEvent(u64 eventnum, u64 arg) {
  //         T             N                       ARG
  u64 temp = ((eventnum & CLU(0xFFF)) << 32) | (arg & CLU(0x00000000FFFFFFFF));
  return DoControl(KUTRACE_CMD_INSERT1, temp);
}

// Uppercase are mapped to lowercase
// All unexpected characters are mapped to '.'
//   - = 0x2D . = 0x2E / = 0x2F
// Base40 characters are _abcdefghijklmnopqrstuvwxyz0123456789-./
//                       0         1         2         3
//                       0123456789012345678901234567890123456789
// where the first is NUL.
inline const char kToBase40[256] = {
   0,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,37,38,39, 
  27,28,29,30, 31,32,33,34, 35,36,38,38, 38,38,38,38, 

  38, 1, 2, 3,  4, 5, 6, 7,  8, 9,10,11, 12,13,14,15,
  16,17,18,19, 20,21,22,23, 24,25,26,38, 38,38,38,38, 
  38, 1, 2, 3,  4, 5, 6, 7,  8, 9,10,11, 12,13,14,15,
  16,17,18,19, 20,21,22,23, 24,25,26,38, 38,38,38,38, 

  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 

  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
  38,38,38,38, 38,38,38,38, 38,38,38,38, 38,38,38,38, 
};

inline const char kFromBase40[40] = {
  '\0','a','b','c', 'd','e','f','g',  'h','i','j','k',  'l','m','n','o',
  'p','q','r','s',  't','u','v','w',  'x','y','z','0',  '1','2','3','4',
  '5','6','7','8',  '9','-','.','/', 
};


// Unpack six characters from 32 bits.
// str must be 8 bytes. We somewhat-arbitrarily capitalize the first letter
inline char* Base40ToChar(u64 base40, char* str) {
  base40 &= CLU(0x00000000ffffffff);	// Just low 32 bits
  memset(str, 0, 8);
  bool first_letter = true;
  // First character went in last, comes out first
  int i = 0;
  while (base40 > 0) {
    u64 n40 = base40 % 40;
    str[i] = kFromBase40[n40];
    base40 /= 40;
    if (first_letter && (1 <= n40) && (n40 <= 26)) {
      str[i] &= ~0x20; 		// Uppercase it
      first_letter = false;
    }
    ++i;
  }
  return str;
}

// Pack six characters into 32 bits. Only use a-zA-Z0-9.-/
inline u64 CharToBase40(const char* str) {
  int len = strlen(str);
  // If longer than 6 characters, take only the first 6
  if (len > 6) {len = 6;}
  u64 base40 = 0;
  // First character goes in last, comes out first
  for (int i = len - 1; i >= 0; -- i) {
    base40 = (base40 * 40) + kToBase40[str[i]];
  }
  return base40;
}

}  // End anonymous namespace

inline bool kutrace::test() {return KutraceInternal::TestModule();}
inline void kutrace::go(const char* process_name) {KutraceInternal::DoReset(0); KutraceInternal::DoInit(process_name); KutraceInternal::DoOn();}
inline void kutrace::goipc(const char* process_name) {KutraceInternal::DoReset(DO_IPC); KutraceInternal::DoInit(process_name); KutraceInternal::DoOn();}
inline void kutrace::goipcwrap(const char* process_name) {KutraceInternal::DoReset(DO_IPC | DO_WRAP); KutraceInternal::DoInit(process_name); KutraceInternal::DoOn();}
inline void kutrace::stop(const char* fname) {KutraceInternal::DoOff(); KutraceInternal::DoFlush(); KutraceInternal::DoDump(fname); KutraceInternal::DoQuit();}
inline void kutrace::mark_a(const char* label) {KutraceInternal::DoMark(KUTRACE_MARKA, KutraceInternal::CharToBase40(label));}
inline void kutrace::mark_b(const char* label) {KutraceInternal::DoMark(KUTRACE_MARKB, KutraceInternal::CharToBase40(label));}
inline void kutrace::mark_c(const char* label) {KutraceInternal::DoMark(KUTRACE_MARKC, KutraceInternal::CharToBase40(label));}
inline void kutrace::mark_d(uint64 n) {KutraceInternal::DoMark(KUTRACE_MARKD, n);}
inline void kutrace::mark_e(const char* label, u64 num = 0xFFFFFFFF) {KutraceInternal::InsertVariableEntry(label, KUTRACE_MARKE_NAME, num);}

// Returns number of words inserted 1..8, or
//   0 if tracing is off, negative if module is not not loaded 
inline u64 kutrace::addevent(uint64 eventnum, uint64 arg) {return KutraceInternal::DoEvent(eventnum, arg);}

inline void kutrace::addname(uint64 eventnum, uint64 number, const char* name) {KutraceInternal::addname(eventnum, number, name);}

inline void kutrace::msleep(int msec) {KutraceInternal::msleep(msec);}
int64 kutrace::readtime() {return KutraceInternal::ku_get_cycles();}

// Go ahead and expose all the routines
const char* kutrace::Base40ToChar(u64 base40, char* str) {return KutraceInternal::Base40ToChar(base40, str);}
inline u64 kutrace::CharToBase40(const char* str) {return KutraceInternal::CharToBase40(str);}

inline u64 kutrace::DoControl(u64 command, u64 arg) {
  return KutraceInternal::DoControl(command, arg);
}
inline void kutrace::DoDump(const char* fname) {KutraceInternal::DoDump(fname);}
inline u64 kutrace::DoEvent(u64 eventnum, u64 arg) {return KutraceInternal::DoEvent(eventnum, arg);}
inline void kutrace::DoFlush() {KutraceInternal::DoFlush();}
inline void kutrace::DoInit(const char* process_name) {KutraceInternal::DoInit(process_name);}
inline void kutrace::DoMark(u64 n, u64 arg) {KutraceInternal::DoMark(n, arg);}
inline bool kutrace::DoTest() {return KutraceInternal::DoTest();}
inline bool kutrace::DoOff() {return KutraceInternal::DoOff();}
inline bool kutrace::DoOn() {return KutraceInternal::DoOn();}
inline void kutrace::DoQuit() {KutraceInternal::DoQuit();}
inline void kutrace::DoReset(u64 doing_ipc){KutraceInternal::DoReset(doing_ipc);}
inline void kutrace::DoStat(u64 control_flags) {KutraceInternal::DoStat(control_flags);}
inline void kutrace::EmitNames(const NumNamePair* ipair, u64 n) {KutraceInternal::EmitNames(ipair, n);}
inline u64 kutrace::GetUsec() {return KutraceInternal::GetUsec();}
inline const char* kutrace::MakeTraceFileName(const char* name, char* str) {
  return KutraceInternal::MakeTraceFileName(name, str);
}
inline bool kutrace::TestModule() {return KutraceInternal::TestModule();}

#endif	// __KUTRACE_LIB_H__


