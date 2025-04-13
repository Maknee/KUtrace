// dumpcsv2json.cc
// Program to convert CSV RPC logs to JSON format for visualization
// Based on dumplogfile4.cc
//
// compile with g++ -O2 csv_dump_log.cc -o csv_dump_log
//
// Usage: ./csv_dump_log "Title" <csv_file_name>

#include <cmath>
#include <time.h>
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <string>
#include <vector>
#include <sstream>
#include <fstream>
#include <iostream>
#include <algorithm>

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

// CSV Log Record structure
struct CSVLogRecord {
    uint32_t rpcid;
    uint32_t parent;
    int64_t req_send_timestamp;  // usec since the epoch, client clock
    int64_t req_rcv_timestamp;   // usec since the epoch, server clock
    int64_t resp_send_timestamp; // usec since the epoch, server clock
    int64_t resp_rcv_timestamp;  // usec since the epoch, client clock
    
    uint32_t client_ip;
    uint32_t server_ip;
    uint16_t client_port;
    uint16_t server_port;
    uint8_t lglen1;              // 10 * lg(request data length in bytes)
    uint8_t lglen2;              // 10 * lg(response data length in bytes)
    uint16_t type;               // An RPCType
    
    std::string method;
    
    uint32_t status;             // 0 = success, other = error code
    uint32_t datalength;         // full length transmitted
    
    std::string data;            // Message data (truncated)
};

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

int64_t imax(int64_t a, int64_t b) { return (a >= b) ? a : b; }

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
  time_t tt = sec;
  struct tm* t = localtime(&tt);
  sprintf(gTempDateTimeBuffer, "%04d%02d%02d_%02d%02d%02d", 
         t->tm_year + 1900, t->tm_mon + 1, t->tm_mday, 
         t->tm_hour, t->tm_min, t->tm_sec);
  return gTempDateTimeBuffer;
}

// Turn usec since the epoch into yyyymmdd_hhmmss.usec
const char* FormatUsecDateTime(int64_t us) {
  int32_t seconds = us / 1000000;
  int32_t usec = us - (seconds * 1000000);
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%s.%06d", 
           FormatSecondsDateTime(seconds), usec);
  return gTempPrintBuffer;
}

// Turn usec since the epoch into ss.usec (no date)
// Note: initial 3d needed for sort of JSON file to but times in order
const char* FormatUsecTime(int64_t us) {
  int32_t seconds = us / 1000000;
  int32_t usec = us - (seconds * 1000000);
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%3d.%06d", seconds, usec);
  return gTempPrintBuffer;
}

// Format IP address
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
  if (type < (uint32_t)RPCType::NumType) {
    return kRPCTypeName[type];
  }
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "Type_%d", type);
  return gTempPrintBuffer;
}

// TenLg length
const char* FormatLglen(uint8_t len) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%d.%d", len / 10, len % 10);
  return gTempPrintBuffer;
}

// Just an rpcid as decimal
const char* FormatRPCIDint(uint32_t rpcid) {
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "%u", rpcid);
  return gTempPrintBuffer;
}

// Turn status into meaningful name or leave as number
const char* FormatStatus(uint32_t status) {
  if (status < (uint32_t)RPCStatus::NumStatus) {return kRPCStatusName[status];}
  // Unknown status values
  snprintf(gTempPrintBuffer, kMaxPrintBuffer, "ERROR_%d", status);
  return gTempPrintBuffer;
}

// Parse a string IP address into uint32_t
uint32_t ParseIpAddress(const std::string& ip_str) {
    uint32_t a, b, c, d;
    if (sscanf(ip_str.c_str(), "%u.%u.%u.%u", &a, &b, &c, &d) != 4) {
        return 0;
    }
    return (a << 24) | (b << 16) | (c << 8) | d;
}

// Parse CSV line into CSVLogRecord
bool ParseCSVLine(const std::string& line, CSVLogRecord& record) {
    std::istringstream ss(line);
    std::string token;
    std::vector<std::string> tokens;
    
    // Split the line by commas
    while (std::getline(ss, token, ',')) {
        tokens.push_back(token);
    }
    
    // Check if we have enough tokens
    if (tokens.size() < 12) {
        return false;
    }
    
    try {
        int idx = 0;
        record.rpcid = std::stoul(tokens[idx++]);
        record.parent = 0;  // Default parent to 0 if not present in CSV
        
        record.req_send_timestamp = std::stoull(tokens[idx++]);
        record.req_rcv_timestamp = tokens[idx] == "0" ? 0 : std::stoull(tokens[idx]); idx++;
        record.resp_send_timestamp = tokens[idx] == "0" ? 0 : std::stoull(tokens[idx]); idx++;
        record.resp_rcv_timestamp = tokens[idx] == "0" ? 0 : std::stoull(tokens[idx]); idx++;
        
        record.client_ip = ParseIpAddress(tokens[idx++]);
        record.server_ip = ParseIpAddress(tokens[idx++]);
        record.client_port = std::stoul(tokens[idx++]);
        record.server_port = std::stoul(tokens[idx++]);
        
        record.type = std::stoul(tokens[idx++]);
        record.method = tokens[idx++];
        record.status = std::stoul(tokens[idx++]);
        
        // Parse data length and calculate lglen values
        if (idx < tokens.size()) {
            record.datalength = std::stoul(tokens[idx++]);
            
            // Calculate lglen from actual data length
            // lglen = 10 * log2(length)
            double log2_len = 0;
            if (record.datalength > 0) {
                log2_len = log2(record.datalength);
            }
            record.lglen1 = static_cast<uint8_t>(log2_len * 10);
            record.lglen2 = record.lglen1;  // Use same value for response if not specified
        } else {
            record.datalength = 0;
            record.lglen1 = 0;
            record.lglen2 = 0;
        }
        
        // Get data if available
        if (idx < tokens.size()) {
            record.data = tokens[idx++];
        } else {
            record.data = "";
        }
        
        return true;
    } catch (const std::exception& e) {
        std::cerr << "Error parsing CSV line: " << e.what() << std::endl;
        return false;
    }
}

// Print one CSV log record to file f in JSON format
void PrintLogRecordAsJson(FILE* f, const CSVLogRecord& lr, uint64_t basetime_usec) {
    fprintf(f, "[");
    fprintf(f, "%s, ", FormatUsecTime(lr.req_send_timestamp - basetime_usec));
    fprintf(f, "%s, ", FormatUsecTime(lr.req_rcv_timestamp - basetime_usec));
    fprintf(f, "%s, ", FormatUsecTime(lr.resp_send_timestamp - basetime_usec));
    fprintf(f, "%s, ", FormatUsecTime(lr.resp_rcv_timestamp - basetime_usec));
    
    fprintf(f, "\"%s\", ", FormatIp(lr.client_ip));
    fprintf(f, "\"%s\", ", FormatIp(lr.server_ip));
    
    fprintf(f, "%s, ", FormatLglen(lr.lglen1));
    fprintf(f, "%s, ", FormatLglen(lr.lglen2));
    fprintf(f, "%s, ", FormatRPCIDint(lr.rpcid));
    fprintf(f, "%s, ", FormatRPCIDint(lr.parent));
    
    fprintf(f, "\"%s\", ", FormatType(lr.type));
    fprintf(f, "\"%s\", ", lr.method.c_str());
    fprintf(f, "\"%s\", ", FormatStatus(lr.status));
    
    fprintf(f, "%u, ", lr.datalength);
    fprintf(f, "\"%s\"", lr.data.c_str());
    fprintf(f, "],\n");
}

void PrintJsonHeader(FILE* f, int64_t basetime, const char* title) {
    // Convert usec to sec and format date_time
    const char* base_char = FormatSecondsDateTimeLong(basetime / 1000000);
    // Leading spaces force header lines to sort to front
    fprintf(f, "  {\n");
    fprintf(f, " \"Comment\" : \"V4 flat RPCs from CSV\",\n");
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
    fprintf(stderr, "Usage: dumpcsv2json [-all] [-req] \"title\" <csv_file_name>\n");
    fprintf(stderr, "       By default, only complete (client type RespRcv) transactions are dumped.\n");
    fprintf(stderr, "       Use -all to see incomplete transactions (server side are all incomplete).\n");
    fprintf(stderr, "       Use -req to see request transactions (client side ReqSend).\n");
}

int main(int argc, const char** argv) {
    bool dump_all = false;
    bool dump_req = false;
    const char* fname = NULL;
    const char* title = NULL;

    // Pick up arguments
    for (int i = 1; i < argc; ++i) {
        if (argv[i][0] != '-') {
            if (title == NULL) {
                title = argv[i];
            } else {
                fname = argv[i];
            }
        } else if (strcmp(argv[i], "-all") == 0) {
            dump_all = true;
        } else if (strcmp(argv[i], "-req") == 0) {
            dump_req = true;
        } else {
            usage();
            return 0;
        }
    }

    if (fname == NULL) {
        usage();
        return 0;
    }

    if (title == NULL) {
        title = "CSV RPC Log";
    }

    // Open CSV file
    std::ifstream csv_file(fname);
    if (!csv_file.is_open()) {
        fprintf(stderr, "Failed to open CSV file: %s\n", fname);
        return 1;
    }

    std::string line;
    int64_t basetime = 0;
    bool header_printed = false;
    int line_count = 0;
    
    // Skip header line if present
    if (std::getline(csv_file, line)) {
        // Check if this looks like a header (contains column names)
        if (line.find("rpcid") != std::string::npos || 
            line.find("timestamp") != std::string::npos || 
            line.find("RPCID") != std::string::npos) {
            // Skip this line, it's a header
            line_count++;
        } else {
            // This is data, process it
            csv_file.seekg(0); // Reset to beginning
        }
    }

    // Process each line of the CSV file
    while (std::getline(csv_file, line)) {
        line_count++;
        
        // Skip empty lines
        if (line.empty()) {
            continue;
        }
        
        // Parse CSV line into record
        CSVLogRecord record;
        if (!ParseCSVLine(line, record)) {
            fprintf(stderr, "Failed to parse line %d: %s\n", line_count, line.c_str());
            continue;
        }
        
        // Check if we should dump this record
        bool dumpme = false;
        if (dump_all) {
            dumpme = true;
        } else if (dump_req && record.type == (uint16_t)RPCType::ReqSend) {
            dumpme = true;
        } else if (record.type == (uint16_t)RPCType::RespRcv) {
            dumpme = true;
        }
        
        if (!dumpme) {
            continue;
        }
        
        // Set base time from first valid record
        if (basetime == 0 && record.req_send_timestamp != 0) {
            // Round down usec time to multiple of one minute
            basetime = (record.req_send_timestamp / 60000000) * 60000000;
            PrintJsonHeader(stdout, basetime, title);
            header_printed = true;
        }
        
        // Fill in any missing timestamps with estimates
        int64_t est_req_usec = RpcMsglglenToUsec(record.lglen1);
        int64_t est_resp_usec = RpcMsglglenToUsec(record.lglen2);
        
        // Fill in any missing times (incomplete RPCs)
        if (record.req_rcv_timestamp == 0) {
            record.req_rcv_timestamp = record.req_send_timestamp + est_req_usec + kMissingTime;
        }
        if (record.resp_send_timestamp == 0) {
            record.resp_send_timestamp = record.req_rcv_timestamp + kMissingTime;
        }
        if (record.resp_rcv_timestamp == 0) {
            record.resp_rcv_timestamp = record.req_send_timestamp + 
                (record.resp_send_timestamp - record.req_rcv_timestamp) + 
                est_req_usec + kMissingTime + est_resp_usec + kMissingTime;
        }
        
        // Enforce that nonzero times are non-decreasing
        if (record.req_rcv_timestamp != 0) {
            record.req_rcv_timestamp = imax(record.req_rcv_timestamp, record.req_send_timestamp);
        }
        if (record.resp_send_timestamp != 0) {
            record.resp_send_timestamp = imax(record.resp_send_timestamp, record.req_rcv_timestamp);
        }
        if (record.resp_rcv_timestamp != 0) {
            record.resp_rcv_timestamp = imax(record.resp_rcv_timestamp, record.resp_send_timestamp);
        }
        
        // Output the record in JSON format
        if (header_printed) {
            PrintLogRecordAsJson(stdout, record, basetime);
        }
    }
    
    // If we didn't print any records, print the header now
    if (!header_printed) {
        // Use current time as base time if no records were found
        basetime = time(NULL) * 1000000;
        PrintJsonHeader(stdout, basetime, title);
    }
    
    PrintJsonFooter(stdout);
    
    csv_file.close();
    
    return 0;
}