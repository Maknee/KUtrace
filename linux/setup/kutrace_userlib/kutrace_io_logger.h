#ifndef KUTRACE_IO_LOGGER_H
#define KUTRACE_IO_LOGGER_H

#include <vector>
#include <string>
#include <thread>
#include <chrono>
#include <fstream>
#include <atomic>
#include <sstream>
#include <unordered_map>
#include <algorithm>
#include <mutex>
#include <span>
#include <functional>
#include <cstring>
#include <iomanip>

/**
 * Lightweight thread-safe header-only I/O operation logger
 * Designed to work with KUTrace and support various I/O systems
 */
class KUTraceIoLogger {
public:
    // Operation types
    enum class op_type : uint8_t {
        read = 0,
        write = 1,
        fsync = 2,
        other = 255
    };

    // Single log KUTraceIOEntry representing an I/O operation
    struct KUTraceIOEntry {
        uint64_t timestamp_ns;  // nanoseconds since epoch or start
        op_type type;
        uint64_t req_id;
        int fd;
        off_t offset;
        size_t size;
        int result;
        int flags;
        std::thread::id thread_id;
        
        // Default constructor
        KUTraceIOEntry() : timestamp_ns(0), type(op_type::other), req_id(0), 
                 fd(-1), offset(0), size(0), result(0), flags(0) {}
        
        // Detailed constructor
        KUTraceIOEntry(op_type t, uint64_t id, int file_desc, off_t off, size_t sz, 
             int res = 0, int flg = 0) 
            : type(t), req_id(id), fd(file_desc), offset(off), size(sz), 
              result(res), flags(flg), thread_id(std::this_thread::get_id()) {
            
            // Get current time in nanoseconds
            auto now = std::chrono::high_resolution_clock::now().time_since_epoch();
            timestamp_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(now).count();
        }
    };

private:
    // Local storage for entries
    std::vector<KUTraceIOEntry> thread_entries;
    
    // Track all threads' entries for merging
    std::mutex merge_mutex;
    std::unordered_map<std::thread::id, std::vector<KUTraceIOEntry>> all_entries;
    
    // Request ID counter
    std::atomic<uint64_t> next_req_id;
    
    // Timing reference point
    std::chrono::high_resolution_clock::time_point start_time;
    
    // Optional thread name mapping for better output
    std::unordered_map<std::thread::id, std::string> thread_names;
    
    // FD to name mapping
    std::unordered_map<int, std::string> fd_names;

public:
    KUTraceIoLogger() : next_req_id(0), start_time(std::chrono::high_resolution_clock::now()) {
        // Pre-allocate space in thread_entries to avoid reallocations
        thread_entries.reserve(1024);
    }
    
    ~KUTraceIoLogger() = default;
    
    // Merge multiple loggers into a new logger (static utility function)
    inline static KUTraceIoLogger merge(std::span<const KUTraceIoLogger*> loggers) {
        KUTraceIoLogger merged_logger;
        
        // Process each input logger
        for (const KUTraceIoLogger* logger : loggers) {
            if (!logger) continue;
            
            // Need to lock the other logger to safely access its entries
            std::lock_guard<std::mutex> other_lock(logger->merge_mutex);
            
            // Copy thread names
            for (const auto& [tid, name] : logger->thread_names) {
                merged_logger.thread_names[tid] = name;
            }
            
            // Copy fd names
            for (const auto& [fd, name] : logger->fd_names) {
                merged_logger.fd_names[fd] = name;
            }
            
            // Copy thread entries
            for (const auto& [tid, entries] : logger->all_entries) {
                auto& target = merged_logger.all_entries[tid];
                target.insert(target.end(), entries.begin(), entries.end());
            }
            
            // Special handling for local thread entries
            {
                std::thread::id current_tid = std::this_thread::get_id();
                merged_logger.all_entries[current_tid].insert(
                    merged_logger.all_entries[current_tid].end(),
                    logger->thread_entries.begin(), 
                    logger->thread_entries.end()
                );
            }
        }
        
        // Set next request ID to max of all loggers + 1
        uint64_t max_req_id = 0;
        for (const KUTraceIoLogger* logger : loggers) {
            if (logger && logger->next_req_id > max_req_id) {
                max_req_id = logger->next_req_id;
            }
        }
        merged_logger.next_req_id = max_req_id;
        
        return merged_logger;
    }
    
    // Get the next request ID (thread-safe)
    uint64_t get_next_req_id() {
        return next_req_id.fetch_add(1, std::memory_order_relaxed);
    }
    
    // Log a read operation
    uint64_t log_read(int fd, off_t offset, size_t size, int flags = 0, int result = 0) {
        uint64_t req_id = get_next_req_id();
        thread_entries.emplace_back(op_type::read, req_id, fd, offset, size, result, flags);
        return req_id;
    }
    
    // Log a write operation
    uint64_t log_write(int fd, off_t offset, size_t size, int flags = 0, int result = 0) {
        uint64_t req_id = get_next_req_id();
        thread_entries.emplace_back(op_type::write, req_id, fd, offset, size, result, flags);
        return req_id;
    }
    
    // Log any operation type with a given request ID
    void log_operation(op_type type, uint64_t req_id, int fd, off_t offset, 
                      size_t size, int flags = 0, int result = 0) {
        thread_entries.emplace_back(type, req_id, fd, offset, size, result, flags);
    }
    
    // Update the result of the latest operation
    void update_result(int result) {
        if (!thread_entries.empty()) {
            thread_entries.back().result = result;
        }
    }
    
    // Update result for a specific request ID
    void update_result(uint64_t req_id, int result) {
        // Search backwards as the req_id is likely recent
        for (auto it = thread_entries.rbegin(); it != thread_entries.rend(); ++it) {
            if (it->req_id == req_id) {
                it->result = result;
                break;
            }
        }
    }
    
    // Register current thread's entries for merging
    void register_thread(const std::string& thread_name = "") {
        std::lock_guard<std::mutex> lock(merge_mutex);
        std::thread::id tid = std::this_thread::get_id();
        
        // Store thread name if provided
        if (!thread_name.empty()) {
            thread_names[tid] = thread_name;
        }
        
        // Store current thread entries
        all_entries[tid] = thread_entries;
    }
    
    // Register a file descriptor with a name
    void register_fd(int fd, const std::string& name) {
        std::lock_guard<std::mutex> lock(merge_mutex);
        fd_names[fd] = name;
    }
    
    // Clear entries for current thread
    void clear_thread_entries() {
        thread_entries.clear();
    }
    
    // Clear all entries from all threads
    void clear_all_entries() {
        std::lock_guard<std::mutex> lock(merge_mutex);
        all_entries.clear();
        clear_thread_entries();
    }
    
    // Get a copy of the current thread's entries
    std::vector<KUTraceIOEntry> get_thread_entries() const {
        return thread_entries;
    }
    
    // Get a span of the current thread's entries (non-owning view)
    std::span<const KUTraceIOEntry> get_thread_entries_span() const {
        return std::span<const KUTraceIOEntry>(thread_entries);
    }
    
    // Get all entries from all threads
    std::vector<KUTraceIOEntry> get_all_entries() {
        register_thread(); // Make sure current thread is included
        
        std::lock_guard<std::mutex> lock(merge_mutex);
        std::vector<KUTraceIOEntry> merged;
        
        // Calculate total size
        size_t total_size = 0;
        for (const auto& [_, entries] : all_entries) {
            total_size += entries.size();
        }
        
        merged.reserve(total_size);
        
        // Merge all entries
        for (const auto& [_, entries] : all_entries) {
            merged.insert(merged.end(), entries.begin(), entries.end());
        }
        
        // Sort by timestamp
        std::sort(merged.begin(), merged.end(), 
            [](const KUTraceIOEntry& a, const KUTraceIOEntry& b) {
                return a.timestamp_ns < b.timestamp_ns;
            });
        
        return merged;
    }
    
    // Dump all entries to a simple JSON file
    bool dump_to_json(const std::string& filename) {
        // Make sure to collect entries from current thread
        register_thread();
        
        std::vector<KUTraceIOEntry> merged = get_all_entries();
        
        std::ofstream file(filename);
        if (!file.is_open()) {
            return false;
        }
        
        // Calculate base timestamp for relative times
        uint64_t base_timestamp = 0;
        if (!merged.empty()) {
            base_timestamp = merged.front().timestamp_ns;
        }
        
        // Write JSON header
        file << "{\n";
        file << "  \"entries\": [\n";
        
        // Write entries
        for (size_t i = 0; i < merged.size(); ++i) {
            const auto& e = merged[i];
            
            // Convert thread ID to string
            std::ostringstream tid_str;
            tid_str << e.thread_id;
            std::string thread_display = tid_str.str();
            
            // Use thread name if available
            auto thread_name_it = thread_names.find(e.thread_id);
            if (thread_name_it != thread_names.end()) {
                thread_display = thread_name_it->second;
            }
            
            // Get file descriptor name if available
            std::string fd_display = std::to_string(e.fd);
            auto fd_name_it = fd_names.find(e.fd);
            if (fd_name_it != fd_names.end()) {
                fd_display += " (" + fd_name_it->second + ")";
            }
            
            // Convert operation type to string
            std::string op_str;
            switch (e.type) {
                case op_type::read: op_str = "read"; break;
                case op_type::write: op_str = "write"; break;
                case op_type::fsync: op_str = "fsync"; break;
                default: op_str = "other"; break;
            }
            
            // Calculate relative timestamp in milliseconds
            double rel_time_ms = (e.timestamp_ns - base_timestamp) / 1000000.0;
            
            file << "    {\n";
            file << "      \"timestamp_ms\": " << std::fixed << std::setprecision(3) << rel_time_ms << ",\n";
            file << "      \"op_type\": \"" << op_str << "\",\n";
            file << "      \"req_id\": " << e.req_id << ",\n";
            file << "      \"fd\": \"" << fd_display << "\",\n";
            file << "      \"offset\": " << e.offset << ",\n";
            file << "      \"size\": " << e.size << ",\n";
            file << "      \"result\": " << e.result << ",\n";
            file << "      \"flags\": " << e.flags << ",\n";
            file << "      \"thread\": \"" << thread_display << "\"\n";
            file << "    }" << (i < merged.size() - 1 ? "," : "") << "\n";
        }
        
        // Write JSON footer
        file << "  ]\n";
        file << "}\n";
        
        return true;
    }
    
    // Analyze and print IO statistics
    void print_stats() {
        register_thread(); // Make sure current thread is included
        
        std::lock_guard<std::mutex> lock(merge_mutex);
        
        std::map<op_type, size_t> op_counts;
        std::map<op_type, size_t> op_bytes;
        std::map<op_type, double> op_latency_total_ms;
        std::map<std::thread::id, size_t> thread_op_counts;
        
        size_t total_ops = 0;
        size_t success_ops = 0;
        size_t failed_ops = 0;
        
        // Process all entries
        for (const auto& [tid, entries] : all_entries) {
            for (size_t i = 0; i < entries.size(); i++) {
                const auto& e = entries[i];
                op_counts[e.type]++;
                op_bytes[e.type] += e.size;
                thread_op_counts[tid]++;
                total_ops++;
                
                // Count successful vs failed operations
                if (e.result >= 0) {
                    success_ops++;
                } else {
                    failed_ops++;
                }
                
                // Try to calculate latency for operations with matching req_ids
                // This assumes a start KUTraceIOEntry followed by a completion KUTraceIOEntry
                if (i < entries.size() - 1 && e.req_id == entries[i+1].req_id) {
                    double latency_ms = (entries[i+1].timestamp_ns - e.timestamp_ns) / 1000000.0;
                    op_latency_total_ms[e.type] += latency_ms;
                }
            }
        }
        
        // Print summary
        std::cout << "\n===== I/O Operation Statistics =====\n";
        std::cout << "Total operations: " << total_ops;
        if (total_ops > 0) {
            std::cout << " (" << success_ops << " successful, " 
                     << failed_ops << " failed, "
                     << (success_ops * 100.0 / total_ops) << "% success rate)";
        }
        std::cout << "\n";
        
        // Print by operation type
        std::cout << "\nOperation types:\n";
        std::cout << "  Read:  " << op_counts[op_type::read] << " ops, " 
                  << (op_bytes[op_type::read] / (1024*1024)) << " MB";
        if (op_counts[op_type::read] > 0) {
            std::cout << " (avg " << (op_bytes[op_type::read] / op_counts[op_type::read]) << " bytes/op)";
        }
        std::cout << "\n";
                  
        std::cout << "  Write: " << op_counts[op_type::write] << " ops, " 
                  << (op_bytes[op_type::write] / (1024*1024)) << " MB";
        if (op_counts[op_type::write] > 0) {
            std::cout << " (avg " << (op_bytes[op_type::write] / op_counts[op_type::write]) << " bytes/op)";
        }
        std::cout << "\n";
        
        std::cout << "  Fsync: " << op_counts[op_type::fsync] << " ops\n";
        std::cout << "  Other: " << op_counts[op_type::other] << " ops\n";
        
        // Print by thread
        std::cout << "\nThreads:\n";
        for (const auto& [tid, count] : thread_op_counts) {
            std::string thread_display;
            auto thread_name_it = thread_names.find(tid);
            if (thread_name_it != thread_names.end()) {
                thread_display = thread_name_it->second;
            } else {
                std::ostringstream tid_str;
                tid_str << tid;
                thread_display = tid_str.str();
            }
            
            std::cout << "  " << thread_display << ": " << count << " ops";
            double thread_percent = (count * 100.0) / total_ops;
            std::cout << " (" << std::fixed << std::setprecision(1) << thread_percent << "%)\n";
        }
        
        std::cout << "==================================\n\n";
    }
};

#endif // KUTRACE_IO_LOGGER_H