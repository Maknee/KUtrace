#include <iostream>
#include <vector>
#include <chrono>
#include <iomanip>
#include <numeric>
#include <algorithm>
#include <functional>
#include <string>
#include <string_view>
#include <cstdint>
#include <cstring>
#include <thread>
#include <fstream>
#include <unistd.h>
#include <sched.h>
#include <pthread.h>
#include <atomic>
#include <mutex>
#include <memory>
#include <random>
#include <future>
#include <utility>
#include <sstream>

// Include original implementations
#include "../kutrace_userlib/kutrace_lib.h"
#include "../kutrace_userlib/kutrace_rpc_logger.h"

// Include optimized implementation with Quill
#include "temp.h"

// Simple benchmarking class
class Benchmark {
private:
    using Clock = std::chrono::steady_clock;
    using TimePoint = std::chrono::time_point<Clock>;
    using Duration = std::chrono::nanoseconds;
    
    std::string name;
    int iterations;
    int warmup_iterations;
    std::vector<Duration> durations;
    bool numa_aware;
    int cpu_pin;

public:
    Benchmark(const std::string& name, int iterations = 100000, int warmup_iterations = 1000)
        : name(name), iterations(iterations), warmup_iterations(warmup_iterations), 
          numa_aware(false), cpu_pin(-1) {}
    
    // Enable NUMA awareness by pinning to a specific CPU
    Benchmark& pin_to_cpu(int cpu_id) {
        numa_aware = true;
        cpu_pin = cpu_id;
        return *this;
    }
    
    template<typename Func>
    void run(Func&& func) {
        // Run some warmup iterations to stabilize CPU frequency and caches
        for (int i = 0; i < warmup_iterations; ++i) {
            func();
        }
        
        durations.clear();
        durations.reserve(iterations);
        
        // If NUMA aware, pin the thread to specified CPU
        if (numa_aware) {
            pin_thread_to_cpu(cpu_pin);
        }
        
        // Run the actual benchmark
        for (int i = 0; i < iterations; ++i) {
            auto start = Clock::now();
            func();
            auto end = Clock::now();
            durations.push_back(std::chrono::duration_cast<Duration>(end - start));
        }
        
        // If NUMA aware, unpin the thread
        if (numa_aware) {
            unpin_thread();
        }
        
        report_results();
    }
    
private:
    void pin_thread_to_cpu(int cpu_id) {
#ifdef __linux__
        cpu_set_t cpuset;
        CPU_ZERO(&cpuset);
        CPU_SET(cpu_id, &cpuset);
        pthread_t current_thread = pthread_self();
        pthread_setaffinity_np(current_thread, sizeof(cpu_set_t), &cpuset);
#else
        std::cerr << "CPU pinning only supported on Linux" << std::endl;
#endif
    }
    
    void unpin_thread() {
#ifdef __linux__
        cpu_set_t cpuset;
        CPU_ZERO(&cpuset);
        for (int i = 0; i < 256; ++i) {  // Set all possible CPUs
            CPU_SET(i, &cpuset);
        }
        pthread_t current_thread = pthread_self();
        pthread_setaffinity_np(current_thread, sizeof(cpu_set_t), &cpuset);
#endif
    }
    
    void report_results() {
        // Calculate statistics
        std::sort(durations.begin(), durations.end());
        
        Duration min_duration = durations.front();
        Duration max_duration = durations.back();
        
        Duration sum = std::accumulate(durations.begin(), durations.end(), Duration(0));
        double avg_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(sum).count() / 
                        static_cast<double>(iterations);
        
        // Calculate median
        Duration median;
        if (iterations % 2 == 0) {
            median = (durations[iterations/2 - 1] + durations[iterations/2]) / 2;
        } else {
            median = durations[iterations/2];
        }
        
        // Calculate 90th, 95th, and 99th percentiles
        Duration p90 = durations[static_cast<size_t>(iterations * 0.9)];
        Duration p95 = durations[static_cast<size_t>(iterations * 0.95)];
        Duration p99 = durations[static_cast<size_t>(iterations * 0.99)];
        
        // Output results
        std::cout << "=== Benchmark: " << name << " ===" << std::endl;
        std::cout << std::fixed << std::setprecision(2);
        std::cout << "Iterations: " << iterations << std::endl;
        std::cout << "Min time:   " << std::chrono::duration_cast<std::chrono::nanoseconds>(min_duration).count() << " ns" << std::endl;
        std::cout << "Max time:   " << std::chrono::duration_cast<std::chrono::nanoseconds>(max_duration).count() << " ns" << std::endl;
        std::cout << "Average:    " << avg_ns << " ns" << std::endl;
        std::cout << "Median:     " << std::chrono::duration_cast<std::chrono::nanoseconds>(median).count() << " ns" << std::endl;
        std::cout << "P90:        " << std::chrono::duration_cast<std::chrono::nanoseconds>(p90).count() << " ns" << std::endl;
        std::cout << "P95:        " << std::chrono::duration_cast<std::chrono::nanoseconds>(p95).count() << " ns" << std::endl;
        std::cout << "P99:        " << std::chrono::duration_cast<std::chrono::nanoseconds>(p99).count() << " ns" << std::endl;
        std::cout << "Throughput: " << 1e9 / avg_ns << " ops/sec" << std::endl;
        std::cout << std::endl;
    }
};

// Helper to get a random rpcid
uint32_t get_random_rpcid() {
    static std::random_device rd;
    static std::mt19937 gen(rd());
    static std::uniform_int_distribution<uint32_t> dist(1, UINT32_MAX);
    return dist(gen);
}

// Helper function to create a dummy RPC message for benchmarking
kutrace::RPCMessage create_dummy_message(size_t size) {
    static std::string dummy_data(size, 'X');
    return kutrace::RPCMessage(dummy_data);
}

// Helper functions to create network addresses for benchmarking
kutrace::NetworkAddress create_client_address() {
    return kutrace::NetworkAddress("192.168.1.10", 12345);
}

kutrace::NetworkAddress create_server_address() {
    return kutrace::NetworkAddress("192.168.1.20", 8080);
}

// Benchmark original RPC Logger with different message sizes
void benchmark_original_rpc_logger(const std::vector<size_t>& message_sizes) {
    for (size_t size : message_sizes) {
        // Set up logger (writing to /dev/null to avoid I/O overhead affecting benchmarks)
        kutrace::RPCLogger logger("/dev/null");
        
        kutrace::NetworkAddress client = create_client_address();
        kutrace::NetworkAddress server = create_server_address();
        kutrace::RPCMessage message = create_dummy_message(size);
        
        uint32_t rpcid = get_random_rpcid();
        uint32_t parent = 0;
        std::string_view method = "TestMethod";
        
        int64_t req_send_time = kutrace::RPCLogger::get_current_usec();
        int64_t req_rcv_time = req_send_time + 1000; // 1ms later
        int64_t resp_send_time = req_rcv_time + 5000; // 5ms later
        
        // Create benchmark name using stringstream instead of fmt
        std::stringstream ss;
        ss << "Original::log_client_send (size=" << size << " bytes)";
        std::string benchmark_name = ss.str();
        
        // Benchmark client_send
        Benchmark(benchmark_name)
            .run([&]() {
                logger.log_client_send(rpcid, parent, client, server, method, message);
            });
        
        // Create benchmark name using stringstream instead of fmt
        ss.str("");
        ss << "Original::log_client_recv (size=" << size << " bytes)";
        benchmark_name = ss.str();
        
        // Benchmark client_recv
        Benchmark(benchmark_name)
            .run([&]() {
                logger.log_client_recv(
                    rpcid, parent, client, server, method, 
                    kutrace::RPCStatus::Success, message,
                    req_send_time, req_rcv_time, resp_send_time
                );
            });
    }
}

// Benchmark CSV RPC Logger with different message sizes
void benchmark_csv_rpc_logger(const std::vector<size_t>& message_sizes) {
    for (size_t size : message_sizes) {
        // Set up logger (writing to /dev/null to avoid I/O overhead affecting benchmarks)
        kutrace::CSVRPCLogger logger("/dev/null");
        
        kutrace::NetworkAddress client = create_client_address();
        kutrace::NetworkAddress server = create_server_address();
        kutrace::RPCMessage message = create_dummy_message(size);
        
        uint32_t rpcid = get_random_rpcid();
        std::string_view method = "TestMethod";
        
        int64_t req_send_time = kutrace::CSVRPCLogger::get_current_usec();
        int64_t req_rcv_time = req_send_time + 1000; // 1ms later
        int64_t resp_send_time = req_rcv_time + 5000; // 5ms later
        
        // Create benchmark name using stringstream instead of fmt
        std::stringstream ss;
        ss << "CSV::log_client_send (size=" << size << " bytes)";
        std::string benchmark_name = ss.str();
        
        // Benchmark client_send
        Benchmark(benchmark_name)
            .run([&]() {
                logger.log_client_send(rpcid, client, server, method, message);
            });
        
        // Create benchmark name using stringstream instead of fmt
        ss.str("");
        ss << "CSV::log_client_recv (size=" << size << " bytes)";
        benchmark_name = ss.str();
        
        // Benchmark client_recv
        Benchmark(benchmark_name)
            .run([&]() {
                logger.log_client_recv(
                    rpcid, client, server, method, 
                    kutrace::RPCStatus::Success, message,
                    req_send_time, req_rcv_time, resp_send_time
                );
            });
    }
}

// Test file operations performance
void benchmark_file_ops() {
    std::cout << "Testing file operations performance...\n" << std::endl;
    
    // Create temporary files
    char temp_filename1[] = "/tmp/original_rpc_XXXXXX";
    char temp_filename2[] = "/tmp/csv_rpc_XXXXXX";
    int fd1 = mkstemp(temp_filename1);
    int fd2 = mkstemp(temp_filename2);
    close(fd1);
    close(fd2);
    
    // Set up test data
    const int NUM_RECORDS = 1000000;
    kutrace::NetworkAddress client = create_client_address();
    kutrace::NetworkAddress server = create_server_address();
    kutrace::RPCMessage message = create_dummy_message(64);
    std::string method = "TestMethod";
    
    // Helper function to read CPU cycles
    auto read_cpu_cycles = []() -> uint64_t {
        #if defined(__x86_64__) || defined(_M_X64)
            uint32_t lo, hi;
            __asm__ __volatile__ ("rdtsc" : "=a" (lo), "=d" (hi));
            return ((uint64_t)hi << 32) | lo;
        #else
            // For non-x86 architectures, return 0 or implement alternative
            return 0;
        #endif
    };
    
    // Test original logger
    {
        auto start = std::chrono::high_resolution_clock::now();
        uint64_t start_cycles = read_cpu_cycles();
        
        kutrace::RPCLogger logger(temp_filename1);
        for (int i = 0; i < NUM_RECORDS; i++) {
            uint32_t rpcid = i;
            logger.log_client_send(rpcid, 0, client, server, method, message);
        }
        
        uint64_t end_cycles = read_cpu_cycles();
        auto end = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start);
        
        double total_time_ms = duration.count() / 1e6;
        double ns_per_op = duration.count() / static_cast<double>(NUM_RECORDS);
        uint64_t cycles_per_op = (end_cycles - start_cycles) / NUM_RECORDS;
        
        std::cout << "Original RPC Logger file operations (" << NUM_RECORDS << " records):" << std::endl;
        std::cout << "  Total time: " << total_time_ms << " ms" << std::endl;
        std::cout << "  Throughput: " << (NUM_RECORDS * 1000.0) / total_time_ms << " ops/sec" << std::endl;
        std::cout << "  Time per operation: " << ns_per_op << " ns/op" << std::endl;
        std::cout << "  Cycles per operation: " << cycles_per_op << " cycles/op" << std::endl;
        
        // Get file size
        std::ifstream file(temp_filename1, std::ios::binary | std::ios::ate);
        std::streamsize file_size = file.tellg();
        file.close();
        std::cout << "  Log file size: " << file_size << " bytes" << std::endl;
        std::cout << std::endl;
    }
    
    // Test CSV logger
    {
        auto start = std::chrono::high_resolution_clock::now();
        uint64_t start_cycles = read_cpu_cycles();
        
        kutrace::CSVRPCLogger logger(temp_filename2);
        for (int i = 0; i < NUM_RECORDS; i++) {
            uint32_t rpcid = i;
            logger.log_client_send(rpcid, client, server, method, message);
        }
        logger.flush(); // Ensure all data is flushed
        
        uint64_t end_cycles = read_cpu_cycles();
        auto end = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end - start);
        
        double total_time_ms = duration.count() / 1e6;
        double ns_per_op = duration.count() / static_cast<double>(NUM_RECORDS);
        uint64_t cycles_per_op = (end_cycles - start_cycles) / NUM_RECORDS;
        
        std::cout << "CSV RPC Logger file operations (" << NUM_RECORDS << " records):" << std::endl;
        std::cout << "  Total time: " << total_time_ms << " ms" << std::endl;
        std::cout << "  Throughput: " << (NUM_RECORDS * 1000.0) / total_time_ms << " ops/sec" << std::endl;
        std::cout << "  Time per operation: " << ns_per_op << " ns/op" << std::endl;
        std::cout << "  Cycles per operation: " << cycles_per_op << " cycles/op" << std::endl;
        
        // Get file size
        std::ifstream file(temp_filename2, std::ios::binary | std::ios::ate);
        std::streamsize file_size = file.tellg();
        file.close();
        std::cout << "  Log file size: " << file_size << " bytes" << std::endl;
        std::cout << std::endl;
    }
    
    // Clean up
    unlink(temp_filename1);
    unlink(temp_filename2);
}

// Run batch benchmark with multiple parallel threads to test scalability
void benchmark_multi_threaded(int num_threads, int ops_per_thread, size_t message_size) {
    std::cout << "\n=== Multi-threaded Benchmark (" << num_threads << " threads, "
              << ops_per_thread << " ops/thread, " << message_size << " bytes) ===\n" << std::endl;
    
    // Function to benchmark one implementation with multiple threads
    auto run_multi_threaded = [num_threads, ops_per_thread, message_size](
            const std::string& name, bool use_csv) {
        
        // Create a temporary file for the benchmark
        char temp_filename[] = "/tmp/rpc_benchmark_XXXXXX";
        int fd = mkstemp(temp_filename);
        close(fd);
        
        std::vector<std::thread> threads;
        std::atomic<int> ready_count(0);
        std::atomic<bool> start_flag(false);
        std::atomic<int> completed_count(0);
        
        // Create a shared mutex to synchronize logger access
        std::mutex logger_mutex;
        
        // Create a single shared logger instance for all threads
        std::unique_ptr<kutrace::RPCLogger> original_logger;
        std::unique_ptr<kutrace::CSVRPCLogger> csv_logger;
        
        if (use_csv) {
            csv_logger = std::make_unique<kutrace::CSVRPCLogger>(temp_filename);
        } else {
            original_logger = std::make_unique<kutrace::RPCLogger>(temp_filename);
        }
        
        auto thread_func = [&ready_count, &start_flag, &completed_count, 
                           &logger_mutex, &original_logger, &csv_logger,
                           use_csv, ops_per_thread, message_size]() {
            // Prepare data for this thread
            kutrace::NetworkAddress client = create_client_address();
            kutrace::NetworkAddress server = create_server_address();
            kutrace::RPCMessage message = create_dummy_message(message_size);
            std::string method = "TestMethod";
            
            // Signal that this thread is ready and wait for start signal
            ready_count.fetch_add(1, std::memory_order_release);
            while (!start_flag.load(std::memory_order_acquire)) {
                std::this_thread::yield();
            }
            
            // Perform operations
            for (int i = 0; i < ops_per_thread; i++) {
                uint32_t rpcid = get_random_rpcid();
                
                // Log client send with proper locking
                std::lock_guard<std::mutex> lock(logger_mutex);
                if (use_csv) {
                    csv_logger->log_client_send(rpcid, client, server, method, message);
                } else {
                    original_logger->log_client_send(rpcid, 0, client, server, method, message);
                }
            }
            
            // Signal completion
            completed_count.fetch_add(1, std::memory_order_release);
        };
        
        // Create all threads
        auto start_time = std::chrono::high_resolution_clock::now();
        
        for (int i = 0; i < num_threads; i++) {
            threads.emplace_back(thread_func);
        }
        
        // Wait for all threads to be ready
        while (ready_count.load(std::memory_order_acquire) < num_threads) {
            std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }
        
        // Start all threads simultaneously
        start_flag.store(true, std::memory_order_release);
        
        // Wait for all threads to finish
        while (completed_count.load(std::memory_order_acquire) < num_threads) {
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        
        // Flush the log to ensure all data is written
        if (use_csv && csv_logger) {
            csv_logger->flush();
        }
        
        // Record end time
        auto end_time = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::milliseconds>(end_time - start_time);
        
        // Join all threads
        for (auto& t : threads) {
            t.join();
        }
        
        // Calculate and report metrics
        double total_ops = num_threads * ops_per_thread;
        double seconds = duration.count() / 1000.0;
        double throughput = total_ops / seconds;
        
        // Check file size
        std::ifstream file(temp_filename, std::ios::binary | std::ios::ate);
        std::streamsize file_size = file.tellg();
        file.close();
        
        std::cout << name << ":\n";
        std::cout << "  Total time: " << seconds << " seconds\n";
        std::cout << "  Throughput: " << throughput << " ops/sec\n";
        std::cout << "  Log file size: " << file_size << " bytes\n";
        std::cout << std::endl;
        
        // Clean up
        unlink(temp_filename);
    };
    
    // Run tests with both implementations
    run_multi_threaded("Original RPC Logger", false);
    run_multi_threaded("CSV RPC Logger", true);
}

// Run variable thread count benchmark to test scalability - with shared logger
void benchmark_scaling(int max_threads, int ops_per_thread, size_t message_size) {
    std::cout << "\n=== Scalability Benchmark (1-" << max_threads << " threads, " 
              << ops_per_thread << " ops/thread, " << message_size << " bytes) ===\n" << std::endl;
    
    // Thread counts to test
    std::vector<int> thread_counts;
    // Test with single thread and powers of 2
    for (int t = 1; t <= max_threads; t = (t == 1) ? 2 : t * 2) {
        thread_counts.push_back(t);
    }
    // If max_threads is not a power of 2, include it too
    if (thread_counts.back() != max_threads) {
        thread_counts.push_back(max_threads);
    }
    
    // Results storage
    struct Result {
        int threads;
        double throughput_original;
        double throughput_csv;
        double ns_per_op_original;
        double ns_per_op_csv;
        uint64_t cycles_per_op_original;
        uint64_t cycles_per_op_csv;
    };
    std::vector<Result> results;
    
    // Helper function to read CPU cycles
    auto read_cpu_cycles = []() -> uint64_t {
        #if defined(__x86_64__) || defined(_M_X64)
            uint32_t lo, hi;
            __asm__ __volatile__ ("rdtsc" : "=a" (lo), "=d" (hi));
            return ((uint64_t)hi << 32) | lo;
        #else
            // For non-x86 architectures, return 0 or implement alternative
            return 0;
        #endif
    };
    
    // Benchmark function
    auto benchmark_with_threads = [ops_per_thread, message_size, &read_cpu_cycles]
        (int num_threads, bool use_csv) -> std::tuple<double, double, uint64_t> {
        // Create a temporary file for the benchmark
        char temp_filename[] = "/tmp/rpc_scaling_XXXXXX";
        int fd = mkstemp(temp_filename);
        close(fd);
        
        std::vector<std::thread> threads;
        std::atomic<int> ready_count(0);
        std::atomic<bool> start_flag(false);
        std::atomic<int> completed_count(0);
        std::atomic<uint64_t> total_cycles(0);
        
        // Create a mutex for synchronizing logger access
        std::mutex logger_mutex;
        
        // Create a single shared logger instance
        std::unique_ptr<kutrace::RPCLogger> original_logger;
        std::unique_ptr<kutrace::CSVRPCLogger> csv_logger;
        
        if (use_csv) {
            csv_logger = std::make_unique<kutrace::CSVRPCLogger>(temp_filename);
        } else {
            original_logger = std::make_unique<kutrace::RPCLogger>(temp_filename);
        }
        
        // Thread function
        auto thread_func = [&ready_count, &start_flag, &completed_count, &total_cycles,
                           &logger_mutex, &original_logger, &csv_logger, &read_cpu_cycles,
                           use_csv, ops_per_thread, message_size]() {
            // Prepare data for this thread
            kutrace::NetworkAddress client = create_client_address();
            kutrace::NetworkAddress server = create_server_address();
            kutrace::RPCMessage message = create_dummy_message(message_size);
            std::string method = "TestMethod";
            
            // Signal that this thread is ready and wait for start signal
            ready_count.fetch_add(1, std::memory_order_release);
            while (!start_flag.load(std::memory_order_acquire)) {
                std::this_thread::yield();
            }
            
            uint64_t thread_cycles = 0;
            
            // Perform operations
            for (int i = 0; i < ops_per_thread; i++) {
                uint32_t rpcid = get_random_rpcid();
                
                // Log client send with proper locking and measure cycles
                std::lock_guard<std::mutex> lock(logger_mutex);
                
                uint64_t start_cycles = read_cpu_cycles();
                
                if (use_csv) {
                    csv_logger->log_client_send(rpcid, client, server, method, message);
                } else {
                    original_logger->log_client_send(rpcid, 0, client, server, method, message);
                }
                
                uint64_t end_cycles = read_cpu_cycles();
                thread_cycles += (end_cycles - start_cycles);
            }
            
            // Add this thread's cycles to the total
            total_cycles.fetch_add(thread_cycles, std::memory_order_relaxed);
            
            // Signal completion
            completed_count.fetch_add(1, std::memory_order_release);
        };
        
        // Create all threads
        auto start_time = std::chrono::high_resolution_clock::now();
        
        for (int i = 0; i < num_threads; i++) {
            threads.emplace_back(thread_func);
        }
        
        // Wait for all threads to be ready
        while (ready_count.load(std::memory_order_acquire) < num_threads) {
            std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }
        
        // Start all threads simultaneously
        start_flag.store(true, std::memory_order_release);
        
        // Wait for all threads to finish
        while (completed_count.load(std::memory_order_acquire) < num_threads) {
            std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }
        
        // Record end time
        auto end_time = std::chrono::high_resolution_clock::now();
        auto duration = std::chrono::duration_cast<std::chrono::nanoseconds>(end_time - start_time);
        
        // Join all threads
        for (auto& t : threads) {
            t.join();
        }
        
        // Calculate metrics
        double total_ops = num_threads * ops_per_thread;
        double seconds = duration.count() / 1e9;
        double throughput = total_ops / seconds;
        double ns_per_op = duration.count() / total_ops;
        uint64_t cycles_per_op = total_cycles.load() / total_ops;
        
        // Clean up
        unlink(temp_filename);
        
        return {throughput, ns_per_op, cycles_per_op};
    };
    
    // Run tests for each thread count
    for (int threads : thread_counts) {
        std::cout << "Testing with " << threads << " threads..." << std::endl;
        
        // Original logger
        auto [throughput_original, ns_per_op_original, cycles_per_op_original] = 
            benchmark_with_threads(threads, false);
        
        // CSV logger
        auto [throughput_csv, ns_per_op_csv, cycles_per_op_csv] = 
            benchmark_with_threads(threads, true);
        
        // Store results
        results.push_back({
            threads, 
            throughput_original, 
            throughput_csv,
            ns_per_op_original,
            ns_per_op_csv,
            cycles_per_op_original,
            cycles_per_op_csv
        });
        
        std::cout << "  Original: " << throughput_original << " ops/sec, " 
                  << ns_per_op_original << " ns/op, " 
                  << cycles_per_op_original << " cycles/op" << std::endl;
        std::cout << "  CSV:      " << throughput_csv << " ops/sec, " 
                  << ns_per_op_csv << " ns/op, "
                  << cycles_per_op_csv << " cycles/op" << std::endl;
        std::cout << "  Speedup:  " << throughput_csv / throughput_original << "x" << std::endl;
        std::cout << std::endl;
    }
    
    // Print summary table
    std::cout << "=== Scalability Summary ===" << std::endl;
    std::cout << "Threads | Original (ops/sec) | CSV (ops/sec) | Speedup | Original (ns/op) | CSV (ns/op) | Original (cycles) | CSV (cycles)" << std::endl;
    std::cout << "--------|-------------------|---------------|---------|-----------------|-------------|-------------------|-------------" << std::endl;
    
    for (const auto& result : results) {
        std::cout << std::setw(7) << result.threads << " | "
                  << std::setw(18) << std::fixed << std::setprecision(2) << result.throughput_original << " | "
                  << std::setw(14) << result.throughput_csv << " | "
                  << std::setw(7) << std::setprecision(2) << result.throughput_csv / result.throughput_original << "x | "
                  << std::setw(16) << std::setprecision(2) << result.ns_per_op_original << " | "
                  << std::setw(12) << result.ns_per_op_csv << " | "
                  << std::setw(18) << result.cycles_per_op_original << " | "
                  << std::setw(12) << result.cycles_per_op_csv
                  << std::endl;
    }
    std::cout << std::endl;
}

int main() {
    std::cout << "Starting RPC Logger Benchmarks..." << std::endl;
    
    // Initialize KUTrace for testing
    kutrace::test();
    kutrace::go("benchmarks");

    // // Benchmark original RPC logger
    // std::vector<size_t> message_sizes = {16, 64, 256, 1024, 4096};
    // benchmark_original_rpc_logger(message_sizes);
    
    // // Benchmark CSV-based RPC logger
    // benchmark_csv_rpc_logger(message_sizes);
    
    // // Benchmark file operations
    benchmark_file_ops();
    
    // // Benchmark multi-threaded performance
    // benchmark_multi_threaded(4, 10000, 64);
    
    // Benchmark scalability
    benchmark_scaling(16, 50000, 8);
    
    // Create a variable to store the trace filename
    char trace_filename[256];
    kutrace::MakeTraceFileName("rpc_benchmarks", trace_filename);
    
    // Stop KUTrace and generate the trace file
    kutrace::stop(trace_filename);
    
    // Delete the trace file
    std::cout << "Deleting trace file: " << trace_filename << std::endl;
    if (unlink(trace_filename) == 0) {
        std::cout << "Trace file deleted successfully." << std::endl;
    } else {
        std::cerr << "Failed to delete trace file: " << strerror(errno) << std::endl;
    }
    
    return 0;
}