// rpc_csv_logger.h
// Simple header-only RPC logging API to track client-server communications
// Using Quill's native CSV writing functionality

#pragma once

#include <string>
#include <string_view>
#include <cstdint>
#include <sys/time.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <cstdlib>
#include <sstream>
#include <memory>

#include "quill/Backend.h"
#include "quill/CsvWriter.h"
#include "quill/core/FrontendOptions.h"

// Enable with environment variable
#define KUTRACE_RPC_ENABLE

namespace kutrace {

// RPC types
enum class CSVRPCType : uint16_t {
    ReqSend = 0,
    ReqRcv,
    RespSend,
    RespRcv,
    Text,
    NumType
};

// RPC status
enum class CSVRPCStatus : uint32_t {
    Success = 0,
    Fail,
    TooBusy,
    NumStatus
};

// Simple structure to represent an IP address and port
struct CSVNetworkAddress {
    uint32_t ip;   // IPv4 address in network byte order
    uint16_t port; // Port number
    
    CSVNetworkAddress() : ip(0), port(0) {}
    
    CSVNetworkAddress(uint32_t ip_addr, uint16_t port_num) : ip(ip_addr), port(port_num) {}
    
    CSVNetworkAddress(const std::string& ip_str, uint16_t port_num) : port(port_num) {
        struct in_addr addr;
        inet_aton(ip_str.c_str(), &addr);
        ip = addr.s_addr;
    }
    
    std::string to_string() const {
        struct in_addr addr;
        addr.s_addr = ip;
        std::stringstream ss;
        ss << inet_ntoa(addr) << ":" << port;
        return ss.str();
    }
    
    // Just the IP part as a string
    std::string ip_to_string() const {
        struct in_addr addr;
        addr.s_addr = ip;
        return std::string(inet_ntoa(addr));
    }
};

// Message data structure for simplifying API calls
struct CSVRPCMessage {
    const uint8_t* data;
    uint32_t length;
    
    CSVRPCMessage() : data(nullptr), length(0) {}
    
    CSVRPCMessage(const uint8_t* data_ptr, uint32_t data_len) : data(data_ptr), length(data_len) {}
    
    CSVRPCMessage(std::string_view str) 
        : data(reinterpret_cast<const uint8_t*>(str.data())), length(str.length()) {}
    
    CSVRPCMessage(const std::string& str) 
        : data(reinterpret_cast<const uint8_t*>(str.data())), length(str.length()) {}
    
    CSVRPCMessage(const char* str) 
        : data(reinterpret_cast<const uint8_t*>(str)), length(strlen(str)) {}
};

// Define our CSV schema for RPC logging
struct RPCCsvSchema {
    static constexpr char const* header = "rpc_id,req_send_timestamp,req_rcv_timestamp,resp_send_timestamp,resp_rcv_timestamp,client_ip,server_ip,client_port,server_port,request_len,response_len,type,method";
    static constexpr char const* format = "{},{},{},{},{},{},{},{},{},{},{},{},{}";
};

struct CSVFrontendOptions
{
//   static constexpr QueueType queue_type = QueueType::UnboundedBlocking;
//   static constexpr size_t initial_queue_capacity = 128u * 1024u; // 128 KiB
//   static constexpr uint32_t blocking_queue_retry_interval_ns = 800;
//   static constexpr size_t unbounded_queue_max_capacity = 2ull * 1024u * 1024u * 1024u; // 2 GiB
//   static constexpr HugePagesPolicy huge_pages_policy = HugePagesPolicy::Never;

  static constexpr quill::QueueType queue_type = quill::QueueType::UnboundedBlocking;
//   static constexpr quill::QueueType queue_type = quill::QueueType::UnboundedDropping;
  static constexpr size_t initial_queue_capacity = 2 * 1024u * 1024u; // 128MB
  static constexpr uint32_t blocking_queue_retry_interval_ns = 10; // 10ns
  static constexpr size_t unbounded_queue_max_capacity = 16ull * 1024u * 1024u * 1024u; // 2 GiB
  static constexpr quill::HugePagesPolicy huge_pages_policy = quill::HugePagesPolicy::Try;
};

// RPC Logger class using Quill's native CSV writing functionality
class CSVRPCLogger {
public:
    CSVRPCLogger() = default;

    // Constructor - opens log file
    explicit CSVRPCLogger(const std::string& filename) {
        open(filename);
    }

    // ~CSVRPCLogger() {
    //     if (quill::Backend::is_running()) {
    //         quill::Backend::stop();
    //     }
    // }

    void open(const std::string& filename) {
        // Initialize Quill if not already started
        if (!quill::Backend::is_running()) {
            quill::BackendOptions backend_options;
            backend_options.error_notifier = {};
            quill::Backend::start(backend_options);
            
            // Silence internal log messages
            // quill::Backend::set_internal_log_level(quill::LogLevel::Error);
        }
        
        // Create our CSV writer
        m_csv_writer = std::make_unique<quill::CsvWriter<RPCCsvSchema, CSVFrontendOptions>>(filename);
    }
    
    // Client-side logging
    uint64_t log_client_send(
        uint32_t rpc_id,
        const CSVNetworkAddress& client,
        const CSVNetworkAddress& server,
        std::string_view method,
        const CSVRPCMessage& message
    ) {
        int64_t timestamp = get_current_usec();
        
        if (m_csv_writer) {
            m_csv_writer->append_row(
                rpc_id,
                timestamp, 0, 0, 0,
                client.ip,
                server.ip,
                client.port,
                server.port,
                message.length, 0,
                static_cast<uint16_t>(CSVRPCType::ReqSend),
                method
            );
        }
        
        return timestamp;
    }
    
    void log_client_recv(
        uint32_t rpc_id,
        const CSVNetworkAddress& client,
        const CSVNetworkAddress& server,
        std::string_view method,
        CSVRPCStatus status,
        const CSVRPCMessage& message,
        int64_t req_send_time,
        int64_t req_rcv_time,
        int64_t resp_send_time
    ) {
        int64_t timestamp = get_current_usec();
        
        if (m_csv_writer) {
            m_csv_writer->append_row(
                rpc_id,
                req_send_time,
                req_rcv_time,
                resp_send_time,
                timestamp,
                client.ip,
                server.ip,
                client.port,
                server.port,
                0, message.length,
                static_cast<uint16_t>(CSVRPCType::RespRcv),
                method
            );
        }
    }
    
    // Server-side logging
    void log_server_recv(
        uint32_t rpc_id,
        const CSVNetworkAddress& client,
        const CSVNetworkAddress& server,
        std::string_view method,
        const CSVRPCMessage& message,
        int64_t req_send_time
    ) {
        int64_t timestamp = get_current_usec();
        
        if (m_csv_writer) {
            m_csv_writer->append_row(
                rpc_id,
                req_send_time,
                timestamp, 0, 0,
                client.ip_to_string(),
                server.ip_to_string(),
                client.port,
                server.port,
                message.length, 0,
                static_cast<uint16_t>(CSVRPCType::ReqRcv),
                method
            );
        }
    }

    void log_server_send(
        uint32_t rpc_id,
        const CSVNetworkAddress& client,
        const CSVNetworkAddress& server,
        std::string_view method,
        CSVRPCStatus status,
        const CSVRPCMessage& message,
        int64_t req_send_time,
        int64_t req_rcv_time
    ) {
        int64_t timestamp = get_current_usec();
        
        if (m_csv_writer) {
            m_csv_writer->append_row(
                rpc_id,
                req_send_time,
                req_rcv_time,
                timestamp, 0,
                client.ip_to_string(),
                server.ip_to_string(),
                client.port,
                server.port,
                0, message.length,
                static_cast<uint16_t>(CSVRPCType::RespSend),
                method
            );
        }
    }
    
    // Explicitly flush logs when requested
    void flush() {
        if (m_csv_writer) {
            // Force Quill to process all pending log messages
            m_csv_writer->flush();
            // quill::Backend::flush();
        }
    }
    
    // Helper to get current time in microseconds
    static int64_t get_current_usec() {
        struct timeval tv;
        gettimeofday(&tv, NULL);
        return (int64_t)tv.tv_sec * 1000000 + tv.tv_usec;
    }

#ifdef KUTRACE_RPC_ENABLE
    static CSVRPCLogger* get() {
        static CSVRPCLogger logger;
        const char* rpc_file = std::getenv("HF3FS_ENABLE_RPC_LOGGING_FILE");
        if (rpc_file && !logger.m_csv_writer) {
            logger.open(rpc_file);
            return &logger;
        } else if (logger.m_csv_writer) {
            return &logger;
        }
        return nullptr;
    }
#endif

private:
    std::unique_ptr<quill::CsvWriter<RPCCsvSchema, CSVFrontendOptions>> m_csv_writer;
};

}  // namespace kutrace