// dumplogfile4.cc cloned from dumplogfile.cc 2018.04.16
// Little program to dump a binary log file
// Copyright 2021 Richard L. Sites
//
// compile with g++ -O2 dumplogfile4.cc dclab_log.cc -o dumplogfile4
//
// expect filename(s) to be of form
//   client4_20180416_151126_dclab-1_3162.log
//
// Hex dump a log file:
//   od -Ax -tx4z -w32 foo.log
//

// ./dumplogfile4 "Write 1MB" client.bin > client.json
// ./makeself show_rpc.html client.json client.html

#include <time.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>

const int kMaxLogDataSize = 0;
const int kMaxMethodNameSize = 24;

// RPC types
enum class RPCType : uint16_t {
    ReqSend = 0,
    ReqRcv,
    RespSend,
    RespRcv,
    Text,
    NumType
};

// RPC status
enum class RPCStatus : uint32_t {
    Success = 0,
    Fail,
    TooBusy,
    NumStatus
};

// Binary log record structure (96 bytes)
typedef struct {
    uint32_t rpcid;
    uint32_t parent;
    int64_t req_send_timestamp;  // usec since the epoch, client clock
    int64_t req_rcv_timestamp;   // usec since the epoch, server clock
    int64_t resp_send_timestamp; // usec since the epoch, server clock
    int64_t resp_rcv_timestamp;  // usec since the epoch, client clock
    // 40 bytes

    uint32_t client_ip;
    uint32_t server_ip;
    uint16_t client_port;
    uint16_t server_port;
    uint8_t lglen1;              // 10 * lg(request data length in bytes)
    uint8_t lglen2;              // 10 * lg(response data length in bytes)
    uint16_t type;               // An RPCType
    // 16 bytes

    char method[kMaxMethodNameSize];
    // 64 bytes

    uint32_t status;             // 0 = success, other = error code
    uint32_t datalength;         // full length transmitted
    // 72 bytes to here

    uint8_t data[kMaxLogDataSize]; // truncated, zero filled
    // 96 bytes
} BinaryLogRecord;


// Assumed Ethernet speed in gigabits per second
static const int64_t kGbs = 1;

// Assumed RPC message overhead, in addition to pure data
static const int64_t kMsgOverheadBytes = 100;

// Assumed time for missing transmission or server time, in usec
static const int kMissingTime = 2;

static char gTempBuffer[24];

// 2**0.0 through 2** 0.9
static const double kPowerTwoTenths[10] = {
  1.0000, 1.0718, 1.1487, 1.2311, 1.3195, 
  1.4142, 1.5157, 1.6245, 1.7411, 1.8661
};

int64_t imax(int64_t a, int64_t b) {return (a >= b) ? a : b;}

// return 2 * (x/10)
int64_t ExpTenths(uint8_t x) {
  int64_t powertwo = x / 10; 
  int64_t fraction = x % 10;
  int64_t retval = 1l << powertwo;
  retval *= kPowerTwoTenths[fraction];
  return retval;
}


// Return sec to transmit x bytes at y Gb/s, where 1 Gb/s = 125000000 B/sec
// but we assume we only get about 90% of this for real data, so 110 B/usec
int64_t BytesToUsec(int64_t x) {
  int64_t retval = x * kGbs / 110;
  return retval;
}

int64_t RpcMsglglenToUsec(uint8_t lglen) {
  return BytesToUsec(ExpTenths(lglen) + kMsgOverheadBytes);
}


// Turn seconds since the epoch into yyyy-mm-dd_hh:mm:ss
// Not valid after January 19, 2038
const char* FormatSecondsDateTimeLong(int64_t sec) {
  // if (sec == 0) {return "unknown";}  // Longer spelling: caller expecting date
  time_t tt = sec;
  struct tm* t = localtime(&tt);
  sprintf(gTempBuffer, "%04d-%02d-%02d_%02d:%02d:%02d", 
         t->tm_year + 1900, t->tm_mon + 1, t->tm_mday, 
         t->tm_hour, t->tm_min, t->tm_sec);
  return gTempBuffer;
}


//
// Formatting for printing
//

// These all use a single static buffer. In real production code, these would 
// all be std::string values, or something else at least as safe.
static const int kMaxDateTimeBuffer = 32;
static char gTempDateTimeBuffer[kMaxDateTimeBuffer];

static const int kMaxPrintBuffer = 256;
static char gTempPrintBuffer[kMaxPrintBuffer];


// Turn seconds since the epoch into yyyymmdd_hhmmss
// Not valid after January 19, 2038
const char* FormatSecondsDateTime(int32_t sec) {
  // if (sec == 0) {return "unknown";}  // Longer spelling: caller expecting date
  time_t tt = sec;
  struct tm* t = localtime(&tt);
  sprintf(gTempDateTimeBuffer, "%04d%02d%02d_%02d%02d%02d", 
         t->tm_year + 1900, t->tm_mon + 1, t->tm_mday, 
         t->tm_hour, t->tm_min, t->tm_sec);
  return gTempDateTimeBuffer;
}

// Turn seconds since the epoch into hhmmss (no date)
// Not valid after January 19, 2038
const char* FormatSecondsTime(int32_t sec) {
  // if (sec == 0) {return "unk";}  // Shorter spelling: caller expecting no date
  time_t tt = sec;
  struct tm* t = localtime(&tt);
  sprintf(gTempDateTimeBuffer, "%02d%02d%02d", 
         t->tm_hour, t->tm_min, t->tm_sec);
  return gTempDateTimeBuffer;
}

// Turn usec since the epoch into yyyymmdd_hhmmss.usec
const char* FormatUsecDateTime(int64_t us) {
  // if (us == 0) {return "unknown";}  // Longer spelling: caller expecting date
  int32_t seconds = us / 1000000;
  int32_t usec = us - (seconds * 1000000);
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%s.%06d", 
           FormatSecondsDateTime(seconds), usec);
  return gTempPrintBuffer;
}

// Turn usec since the epoch into ss.usec (no date)
// Note: initial 3d needed for sort of JSON file to but times in order
const char* FormatUsecTime(int64_t us) {
  // if (us == 0) {return "unk";}
  int32_t seconds = us / 1000000;
  int32_t usec = us - (seconds * 1000000);
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%3d.%06d", seconds, usec);
  return gTempPrintBuffer;
}

// TODO: map into a human-meaningful name
const char* FormatIpPort(uint32_t ip, uint16_t port) {
  if (ip == 0) {return "unk:unk";}
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%d.%d.%d.%d:%d", 
           (ip >> 24) & 0xff, (ip >> 16) & 0xff, 
           (ip >> 8) & 0xff, (ip >> 0) & 0xff, port);
  return gTempPrintBuffer;
}

// TODO: map into a human-meaningful name
const char* FormatIp(uint32_t ip) {
  if (ip == 0) {return "unk:unk";}
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%d.%d.%d.%d", 
           (ip >> 24) & 0xff, (ip >> 16) & 0xff, 
           (ip >> 8) & 0xff, (ip >> 0) & 0xff);
  return gTempPrintBuffer;
}

static const char* const kRPCTypeName[] = {
  "ReqSend ", "ReqRcv  ", "RespSend", "RespRcv ", "Text    "
};

// Padded to 8 characters for printing
static const char* const kRPCStatusName[] = {
  "Success ", "Fail    ",  "TooBusy ", 
};


// Turn RPC type enum into a meaningful name
const char* FormatType(uint32_t type) {
  return kRPCTypeName[type];
}

// TenLg length
const char* FormatLglen(uint8_t len) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%d.%d", len / 10, len % 10);
  return gTempPrintBuffer;
}

// Just an rpcid as hex
const char* FormatRPCID(uint32_t rpcid) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%08x", rpcid);
  return gTempPrintBuffer;
}

// Just an rpcid as decimal
const char* FormatRPCIDint(uint32_t rpcid) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%u", rpcid);
  return gTempPrintBuffer;
}

// Method as C string with trailing '\0'
const char* FormatMethod(const char* method) {
  if (method[0] == '\0') {return "unknown";}
  memcpy(gTempPrintBuffer, method, 8);
  gTempPrintBuffer[8] = '\0';
  return gTempPrintBuffer;
}

// Turn status into meaningful name or leave as number
const char* FormatStatus(uint32_t status) {
  if (status < (uint32_t)RPCStatus::NumStatus) {return kRPCStatusName[status];}
  // Unknown status values
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "ERROR_%d", status);
  return gTempPrintBuffer;
}

// Just show length in decimal
const char* FormatLength(uint32_t length) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%d", length);
  return gTempPrintBuffer;
}

// Turn fixed-field-width data into C string with trailing '\0'
// We expect a delimited string with 4-byte length on front
// We only do the first of possibly two strings
const char* FormatData(const uint8_t* data, int fixed_width) {
  int trunclen = (fixed_width >= kMaxLogDataSize) ? kMaxLogDataSize : fixed_width;
  for (int i = 0; i < trunclen; ++i) {
    uint8_t c = data[i];
    if (c <= ' ') {c = '.';}	// Turn any bytes of delimited length into dots
    gTempPrintBuffer[i] = c;
  }
  gTempPrintBuffer[trunclen] = '\0';

#if 1
  // Suppress trailing spaces
  for (int i = trunclen - 1; i >= 0; --i) {
    if (gTempPrintBuffer[i] == ' ') {
      gTempPrintBuffer[i] = '\0';
    } else {
      break;
    }
  }
#endif
  return gTempPrintBuffer;
}

// Print one binary log record to file f
void PrintLogRecordAsJson(FILE* f, const BinaryLogRecord* lr, uint64_t basetime_usec) {
  fprintf(f, "[");
  fprintf(f, "%s, ", FormatUsecTime(lr->req_send_timestamp - basetime_usec));
  fprintf(f, "%s, ", FormatUsecTime(lr->req_rcv_timestamp - basetime_usec));
  fprintf(f, "%s, ", FormatUsecTime(lr->resp_send_timestamp - basetime_usec));
  fprintf(f, "%s, ", FormatUsecTime(lr->resp_rcv_timestamp - basetime_usec));

  //fprintf(f, "\"%s\", ", FormatIpPort(lr->client_ip, lr->client_port));
  //fprintf(f, "\"%s\", ", FormatIpPort(lr->server_ip, lr->server_port));
  fprintf(f, "\"%s\", ", FormatIp(lr->client_ip));
  fprintf(f, "\"%s\", ", FormatIp(lr->server_ip));

  fprintf(f, "%s, ", FormatLglen(lr->lglen1));
  fprintf(f, "%s, ", FormatLglen(lr->lglen2));
  fprintf(f, "%s, ", FormatRPCIDint(lr->rpcid));
  fprintf(f, "%s, ", FormatRPCIDint(lr->parent));

  fprintf(f, "\"%s\", ", FormatType(lr->type));
  fprintf(f, "\"%s\", ", FormatMethod(lr->method));
  fprintf(f, "\"%s\", ", FormatStatus(lr->status));

  fprintf(f, "%s, ", FormatLength(lr->datalength));
  fprintf(f, "\"%s\"", FormatData(lr->data, kMaxLogDataSize));
  fprintf(f, "],\n");
}

void PrintJsonHeader(FILE* f, int64_t basetime, const char* title) {
  // Convert usec to sec and format date_time
  const char* base_char = FormatSecondsDateTimeLong(basetime / 1000000);
  // Leading spaces force header lines to sort to front
  fprintf(f, "  {\n");
  fprintf(f, " \"Comment\" : \"V4 flat RPCs\",\n");
  fprintf(f, " \"axisLabelX\" : \"Time (sec)\",\n");
  fprintf(f, " \"axisLabelY\" : \"RPC Number\",\n");
  fprintf(f, " \"deltaT23\" : 0,\n");
  fprintf(f, " \"flags\" : 0,\n");
  fprintf(f, " \"gbs\" : 1,\n");
  fprintf(f, " \"shortMulX\" : 1,\n");
  fprintf(f, " \"shortUnitsX\" : \"s\",\n");
  fprintf(f, " \"thousandsX\" : 1000,\n");
  fprintf(f, " \"title\" : \"%s\",\n", title);
  fprintf(f, " \"tracebase\" : \"%s\",\n", base_char);
  fprintf(f, " \"version\" : 4,\n");
  fprintf(f, "\"events\" : [\n");
}

void PrintJsonFooter(FILE* f) {
  fprintf(f, "[999.0, 0.0, 0.0, 0.0, \"\", \"\", 0.0, 0.0, 0, 0, \"\", \"\", \"\", 0, \"\"]\n");
  fprintf(f, "]}\n");
}

void usage() {
  fprintf(stderr, "Usage: dumplogfile4 [-all] [-req] \"title\" <binary file name(s)>\n");
  fprintf(stderr, "       By default, only complete (client type RespRcv) transactions are dumped.\n");
  fprintf(stderr, "       Use -all to see incomplete transactions (server side are all incomlete).\n");
}

static const int kMaxFileNames = 100;

int main(int argc, const char** argv) {
  bool dump_raw = false;
  bool dump_all = false;
  bool dump_req = false;
  int next_fname = 0;
  const char* fname[kMaxFileNames];
  const char* title = NULL;

  // Pick up arguments
  for (int i = 1; i < argc; ++i) {
    if (argv[i][0] != '-') {
      if (title == NULL) {
        title = argv[i];
      } else {
        fname[next_fname++] = argv[i];
        if (next_fname >= kMaxFileNames) {
          fprintf(stderr, "More than %d file names.\n", kMaxFileNames);
          return 0;
        }
      }
    } else if (strcmp(argv[i], "-raw") == 0) {
      dump_raw = true;
    } else if (strcmp(argv[i], "-all") == 0) {
      dump_all = true;
    } else if (strcmp(argv[i], "-req") == 0) {
      dump_req = true;
    } else {
      usage();
      return 0;
    }
  }

  if (next_fname == 0) {
    usage();
    return 0;
  }

  if (title == NULL) {title = "Placeholder title";}

  FILE* logfile;
  BinaryLogRecord lr;
  int64_t basetime = 0;	// In usec
  // Process log files in order presented
  for (int i = 0; i < next_fname; ++i) {
    logfile = fopen(fname[i], "rb");
    if (logfile == NULL) {
      fprintf(stderr, "%s did not open\n", fname[i]);
      return 0;
    }

    // Always dump complete transactions, from client-side logs RespRcvType
    // If -req, dump completed server-side requests RespSendType
    // If -all, dump all log records
    while(fread(&lr, sizeof(BinaryLogRecord), 1, logfile) != 0) {
      bool dumpme = false;
      if (dump_all) {dumpme = true;}
      if (dump_req && lr.type == (uint16_t)RPCType::ReqSend) {dumpme = true;}
      if (lr.type == (uint16_t)RPCType::RespRcv) {dumpme = true;}
      if (!dumpme) {continue;}

      // Pick off base time at first RPC
      if ((basetime == 0) && (lr.req_send_timestamp != 0)) {
        // Round down usec time to multiple of one minute
        basetime = (lr.req_send_timestamp / 60000000) * 60000000;
        PrintJsonHeader(stdout, basetime, title);
      }

      // Estimated network transmission times
      int64_t est_req_usec = RpcMsglglenToUsec(lr.lglen1);
      int64_t est_resp_usec = RpcMsglglenToUsec(lr.lglen2);

      if (!dump_raw) {
        // Fill in any missing times (incomlete RPCs)
        // Missing t2 etc. must include estimated transmission time
        // Times in usec
        if (lr.req_rcv_timestamp == 0) {
          lr.req_rcv_timestamp = lr.req_send_timestamp + est_req_usec + kMissingTime;
        }
        if (lr.resp_send_timestamp == 0) {
          lr.resp_send_timestamp = lr.req_rcv_timestamp + kMissingTime;
        }
        if (lr.resp_rcv_timestamp == 0) {
          lr.resp_rcv_timestamp = lr.req_send_timestamp + 
            (lr.resp_send_timestamp - lr.req_rcv_timestamp) + 
            est_req_usec + kMissingTime + est_resp_usec + kMissingTime;
        }

        // Enforce that nonzero times are non-decreasing
        if (lr.req_rcv_timestamp != 0) {
          lr.req_rcv_timestamp   = imax(lr.req_rcv_timestamp, lr.req_send_timestamp);
        }
        if (lr.resp_send_timestamp != 0) {
          lr.resp_send_timestamp = imax(lr.resp_send_timestamp, lr.req_rcv_timestamp);
        }
        if (lr.resp_rcv_timestamp != 0) {
          lr.resp_rcv_timestamp  = imax(lr.resp_rcv_timestamp, lr.resp_send_timestamp);
        }
      }

      PrintLogRecordAsJson(stdout, &lr, basetime);
    }
    fclose(logfile);
  }
  PrintJsonFooter(stdout);

  return 0;
}

