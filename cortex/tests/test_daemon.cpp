#include "archie/cortex/contracts.hpp"
#include "archie/cortex/ipc.hpp"

#include <cstdlib>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#if defined(__unix__)
#include <cerrno>
#include <chrono>
#include <csignal>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <unistd.h>
#endif

namespace {

#if defined(__unix__)

using namespace archie::cortex;

GraphSnapshot snapshot() {
    GraphSnapshot graph;
    graph.snapshot_id = "snapshot-e2e";
    graph.captured_at_ms = 1'700'000'000'000;
    graph.nodes = {
        GraphNode{.id = "person:rafita",
                  .kind = NodeKind::Person,
                  .summary = "Rafita owns the robot arm project.",
                  .aliases = {},
                  .provenance_ref = "source:e2e",
                  .sensitivity = Sensitivity::LocalRestricted,
                  .observed_at_ms = 1'700'000'000'000,
                  .version = 1},
    };
    return graph;
}

ContextRequest request() {
    return ContextRequest{.request_id = "request-e2e",
                          .question = "What project does Rafita own?",
                          .active_project_id = {},
                          .graph_snapshot_id = "snapshot-e2e",
                          .permitted_sensitivities = {Sensitivity::LocalRestricted},
                          .token_budget = 256,
                          .maximum_items = 8,
                          .maximum_expansion_hops = 1,
                          .requested_at_ms = 1'700'000'001'000};
}

bool write_all(int fd, const std::uint8_t* data, std::size_t size) {
    std::size_t offset = 0;
    while (offset < size) {
        const auto count = ::write(fd, data + offset, size - offset);
        if (count < 0) {
            if (errno == EINTR) continue;
            return false;
        }
        offset += static_cast<std::size_t>(count);
    }
    return true;
}

bool read_all(int fd, std::uint8_t* data, std::size_t size) {
    std::size_t offset = 0;
    while (offset < size) {
        const auto count = ::read(fd, data + offset, size - offset);
        if (count <= 0) {
            if (count < 0 && errno == EINTR) continue;
            return false;
        }
        offset += static_cast<std::size_t>(count);
    }
    return true;
}

bool write_frame(int fd, const std::vector<std::uint8_t>& payload) {
    const auto size = static_cast<std::uint32_t>(payload.size());
    const std::uint8_t header[] = {
        static_cast<std::uint8_t>((size >> 24U) & 0xffU),
        static_cast<std::uint8_t>((size >> 16U) & 0xffU),
        static_cast<std::uint8_t>((size >> 8U) & 0xffU),
        static_cast<std::uint8_t>(size & 0xffU),
    };
    return write_all(fd, header, sizeof(header)) && write_all(fd, payload.data(), payload.size());
}

std::optional<std::vector<std::uint8_t>> read_frame(int fd) {
    std::uint8_t header[4]{};
    if (!read_all(fd, header, sizeof(header))) return std::nullopt;
    const auto size = (static_cast<std::uint32_t>(header[0]) << 24U) |
                      (static_cast<std::uint32_t>(header[1]) << 16U) |
                      (static_cast<std::uint32_t>(header[2]) << 8U) |
                      static_cast<std::uint32_t>(header[3]);
    if (size == 0U || size > ipc::kMaximumFrameBytes) return std::nullopt;
    std::vector<std::uint8_t> payload(size);
    if (!read_all(fd, payload.data(), payload.size())) return std::nullopt;
    return payload;
}

int connect_to(const std::string& socket_path) {
    const int fd = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    if (socket_path.size() >= sizeof(address.sun_path)) {
        (void)::close(fd);
        return -1;
    }
    std::snprintf(address.sun_path, sizeof(address.sun_path), "%s", socket_path.c_str());
    if (::connect(fd, reinterpret_cast<const sockaddr*>(&address), sizeof(address)) != 0) {
        (void)::close(fd);
        return -1;
    }
    const timeval timeout{.tv_sec = 5, .tv_usec = 0};
    (void)::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    (void)::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
    return fd;
}

bool bundle_contains(const ContextBundle& bundle, std::string_view id) {
    for (const auto& item : bundle.facts) {
        if (item.node_id == id) return true;
    }
    return false;
}

int run() {
    char directory_template[] = "/tmp/archie-cortex-e2e.XXXXXX";
    char* directory = ::mkdtemp(directory_template);
    if (directory == nullptr) return EXIT_FAILURE;
    const std::string root{directory};
    const std::string graph_path = root + "/current.acg";
    const std::string socket_path = root + "/cortex.sock";
    {
        std::ofstream graph_file(graph_path, std::ios::binary | std::ios::trunc);
        const auto bytes = ipc::encode(snapshot());
        graph_file.write(reinterpret_cast<const char*>(bytes.data()),
                         static_cast<std::streamsize>(bytes.size()));
        if (!graph_file.good()) return EXIT_FAILURE;
    }

    const auto child = ::fork();
    if (child < 0) return EXIT_FAILURE;
    if (child == 0) {
        ::execl(ARCHIE_CORTEXD_PATH, ARCHIE_CORTEXD_PATH, "--socket", socket_path.c_str(),
                "--graph-snapshot", graph_path.c_str(), static_cast<char*>(nullptr));
        _exit(127);
    }

    int fd = -1;
    for (int attempt = 0; attempt < 100; ++attempt) {
        fd = connect_to(socket_path);
        if (fd >= 0) break;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    bool passed = fd >= 0;
    if (passed) {
        passed = write_frame(fd, ipc::encode(request()));
        const auto response = passed ? read_frame(fd) : std::nullopt;
        const auto bundle = response ? ipc::decode_context_response(*response) : std::nullopt;
        passed = bundle.has_value() && bundle_contains(*bundle, "person:rafita") &&
                 bundle->metrics.token_count <= 256U;
        (void)::close(fd);
    }
    if (passed) {
        fd = connect_to(socket_path);
        auto focused = request();
        focused.question = "no lexical overlap";
        FocusState focus;
        focus.focus_id = "focus:e2e";
        focus.activations = {{"person:rafita", 1.0, focused.requested_at_ms, true}};
        focused.focus = focus;
        passed = fd >= 0 && write_frame(fd, ipc::encode(focused));
        const auto response = passed ? read_frame(fd) : std::nullopt;
        const auto bundle = response ? ipc::decode_context_response(*response) : std::nullopt;
        passed = bundle.has_value() && bundle_contains(*bundle, "person:rafita");
        if (fd >= 0) (void)::close(fd);
    }
    if (passed) {
        fd = connect_to(socket_path);
        auto prospective = request();
        prospective.retrieval_cues = {{"person:rafita","future meeting"}};
        prospective.token_budget = 1024;
        passed = fd >= 0 && write_frame(fd, ipc::encode(prospective));
        const auto response = passed ? read_frame(fd) : std::nullopt;
        const auto bundle = response ? ipc::decode_context_response(*response) : std::nullopt;
        passed = bundle && bundle->selector_version == "prospective-v1" && bundle_contains(*bundle,"person:rafita");
        if (fd >= 0) (void)::close(fd);
    }
    (void)::kill(child, SIGTERM);
    int status = 0;
    bool exited = false;
    for (int attempt = 0; attempt < 200; ++attempt) {
        if (::waitpid(child, &status, WNOHANG) == child) { exited = true; break; }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    if (!exited) {
        (void)::kill(child, SIGKILL);
        (void)::waitpid(child, &status, 0);
        std::cerr << "archie-cortexd failed to stop after SIGTERM\n";
    }
    passed = passed && exited && WIFEXITED(status) && WEXITSTATUS(status) == 0;
    (void)::unlink(socket_path.c_str());
    (void)::unlink(graph_path.c_str());
    (void)::rmdir(root.c_str());
    if (!passed) {
        std::cerr << "archie-cortexd did not return a valid ContextBundle over AF_UNIX\n";
    }
    return passed ? EXIT_SUCCESS : EXIT_FAILURE;
}

#endif

}  // namespace

int main() {
#if defined(__unix__)
    return run();
#else
    std::cout << "skipped: AF_UNIX daemon E2E requires the production platform\n";
    return 77;
#endif
}
