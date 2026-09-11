// archie-cortexd — unprivileged local ContextBundle compiler.
//
// This daemon receives a policy-filtered request over a systemd-owned Unix socket, reads
// a trusted immutable graph snapshot, and returns a bounded ContextBundle. It has no TCP
// listener, command execution, graph mutation, lease, credential or policy interface.

#include "archie/cortex/contracts.hpp"
#include "archie/cortex/ipc.hpp"

#include <cerrno>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#if defined(__unix__)
#include <fcntl.h>
#include <poll.h>
#include <pwd.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <unistd.h>
#endif

namespace {

constexpr int kUnavailable = 69;
constexpr int kDataError = 65;

struct Options {
    std::string graph_snapshot{"/var/lib/archie/knowledge/graph/current.acg"};
    std::optional<std::string> direct_socket{};
};

[[nodiscard]] std::optional<Options> parse_options(int argc, char* argv[]) {
    Options options;
    for (int index = 1; index < argc; ++index) {
        const std::string_view argument{argv[index]};
        if (argument == "--graph-snapshot" || argument == "--socket") {
            if (++index >= argc || std::string_view{argv[index]}.empty()) return std::nullopt;
            if (argument == "--graph-snapshot") {
                options.graph_snapshot = argv[index];
            } else {
                options.direct_socket = argv[index];
            }
            continue;
        }
        return std::nullopt;
    }
    return options;
}

[[nodiscard]] bool self_check() {
    archie::cortex::GraphSnapshot snapshot{
        .snapshot_id = "self-check", .captured_at_ms = 1, .nodes = {}, .edges = {}};
    return !archie::cortex::validate(snapshot).has_value();
}

#if defined(__unix__)

volatile std::sig_atomic_t keep_running = 1;
extern "C" void stop(int) { keep_running = 0; }

[[nodiscard]] bool read_exact(int fd, void* destination, std::size_t size) {
    auto* bytes = static_cast<std::uint8_t*>(destination);
    std::size_t consumed = 0;
    while (consumed < size) {
        const auto count = ::read(fd, bytes + consumed, size - consumed);
        if (count == 0) return false;
        if (count < 0) {
            if (errno == EINTR) continue;
            return false;
        }
        consumed += static_cast<std::size_t>(count);
    }
    return true;
}

[[nodiscard]] bool write_exact(int fd, const void* source, std::size_t size) {
    const auto* bytes = static_cast<const std::uint8_t*>(source);
    std::size_t written = 0;
    while (written < size) {
        const auto count = ::write(fd, bytes + written, size - written);
        if (count < 0) {
            if (errno == EINTR) continue;
            return false;
        }
        written += static_cast<std::size_t>(count);
    }
    return true;
}

[[nodiscard]] std::optional<std::vector<std::uint8_t>> read_frame(int fd) {
    std::uint8_t length_bytes[4]{};
    if (!read_exact(fd, length_bytes, sizeof(length_bytes))) return std::nullopt;
    const auto length = (static_cast<std::uint32_t>(length_bytes[0]) << 24U) |
                        (static_cast<std::uint32_t>(length_bytes[1]) << 16U) |
                        (static_cast<std::uint32_t>(length_bytes[2]) << 8U) |
                        static_cast<std::uint32_t>(length_bytes[3]);
    if (length == 0U || length > archie::cortex::ipc::kMaximumFrameBytes) return std::nullopt;
    std::vector<std::uint8_t> payload(length);
    if (!read_exact(fd, payload.data(), payload.size())) return std::nullopt;
    return payload;
}

[[nodiscard]] bool write_frame(int fd, const std::vector<std::uint8_t>& payload) {
    if (payload.empty() || payload.size() > archie::cortex::ipc::kMaximumFrameBytes) return false;
    const auto length = static_cast<std::uint32_t>(payload.size());
    const std::uint8_t header[4] = {
        static_cast<std::uint8_t>((length >> 24U) & 0xffU),
        static_cast<std::uint8_t>((length >> 16U) & 0xffU),
        static_cast<std::uint8_t>((length >> 8U) & 0xffU),
        static_cast<std::uint8_t>(length & 0xffU),
    };
    return write_exact(fd, header, sizeof(header)) && write_exact(fd, payload.data(), payload.size());
}

[[nodiscard]] std::optional<archie::cortex::GraphSnapshot> load_snapshot(
    const std::string& path) {
    std::ifstream file(path, std::ios::binary | std::ios::ate);
    if (!file.is_open()) return std::nullopt;
    const auto size = file.tellg();
    if (size <= 0 || size > static_cast<std::streamoff>(archie::cortex::ipc::kMaximumSnapshotBytes)) {
        return std::nullopt;
    }
    const auto byte_count = static_cast<std::streamsize>(size);
    std::vector<std::uint8_t> bytes(static_cast<std::size_t>(byte_count));
    file.seekg(0);
    if (!file.read(reinterpret_cast<char*>(bytes.data()), byte_count)) return std::nullopt;
    return archie::cortex::ipc::decode_graph_snapshot(bytes);
}

[[nodiscard]] bool peer_is_permitted(int client, bool activated_socket) {
#if defined(SO_PEERCRED)
    ucred peer{};
    socklen_t peer_size = sizeof(peer);
    if (::getsockopt(client, SOL_SOCKET, SO_PEERCRED, &peer, &peer_size) != 0) return false;
    if (!activated_socket) return peer.uid == ::geteuid();
    const passwd* archied = ::getpwnam("archie");
    return archied != nullptr && peer.uid == archied->pw_uid;
#else
    // ArchieOS targets Linux and requires SO_PEERCRED. Other Unix targets must not
    // silently weaken this boundary merely to make a development build convenient.
    (void)client;
    (void)activated_socket;
    return false;
#endif
}

void serve_client(int client, const std::string& snapshot_path, bool activated_socket) {
    if (!peer_is_permitted(client, activated_socket)) {
        return;
    }
    // One unresponsive peer must not hold the single bounded accept loop indefinitely.
    const timeval timeout{.tv_sec = 5, .tv_usec = 0};
    (void)::setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    (void)::setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
    const auto frame = read_frame(client);
    if (!frame) return;
    const auto request = archie::cortex::ipc::decode_context_request(*frame);
    if (!request) {
        (void)write_frame(client, archie::cortex::ipc::encode(
            archie::cortex::ipc::Refusal{.code = archie::cortex::ContractRefusal::MalformedIpcFrame}));
        return;
    }
    const auto snapshot = load_snapshot(snapshot_path);
    if (!snapshot) {
        (void)write_frame(client, archie::cortex::ipc::encode(
            archie::cortex::ipc::Refusal{.code = archie::cortex::ContractRefusal::SnapshotUnavailable}));
        return;
    }
    archie::cortex::ContextBundle bundle;
    if (const auto refusal = archie::cortex::DeterministicContextCompiler{}.compile(
            *snapshot, *request, &bundle)) {
        (void)write_frame(client, archie::cortex::ipc::encode(
            archie::cortex::ipc::Refusal{.code = *refusal}));
        return;
    }
    (void)write_frame(client, archie::cortex::ipc::encode(bundle));
}

[[nodiscard]] int systemd_socket() {
    const char* pid_text = std::getenv("LISTEN_PID");
    const char* fds_text = std::getenv("LISTEN_FDS");
    if (pid_text == nullptr || fds_text == nullptr) return -1;
    char* end = nullptr;
    const auto pid = std::strtol(pid_text, &end, 10);
    if (end == pid_text || *end != '\0' || pid != static_cast<long>(::getpid())) return -1;
    end = nullptr;
    const auto fds = std::strtol(fds_text, &end, 10);
    if (end == fds_text || *end != '\0' || fds < 1L) return -1;
    constexpr int first_systemd_fd = 3;
    int type = 0;
    socklen_t type_size = sizeof(type);
    if (::getsockopt(first_systemd_fd, SOL_SOCKET, SO_TYPE, &type, &type_size) != 0 ||
        type != SOCK_STREAM) return -1;
    return first_systemd_fd;
}

[[nodiscard]] int bind_direct_socket(const std::string& path) {
    if (path.empty() || path.size() >= sizeof(sockaddr_un::sun_path)) return -1;
    struct stat existing {};
    if (::lstat(path.c_str(), &existing) == 0) {
        // Never remove an ordinary file, directory, symlink or special file.
        if (!S_ISSOCK(existing.st_mode) || ::unlink(path.c_str()) != 0) return -1;
    } else if (errno != ENOENT) {
        return -1;
    }
    const int fd = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    sockaddr_un address{};
    address.sun_family = AF_UNIX;
    std::strncpy(address.sun_path, path.c_str(), sizeof(address.sun_path) - 1U);
    if (::bind(fd, reinterpret_cast<const sockaddr*>(&address), sizeof(address)) != 0 ||
        ::chmod(path.c_str(), 0660) != 0 || ::listen(fd, 16) != 0) {
        (void)::close(fd);
        (void)::unlink(path.c_str());
        return -1;
    }
    return fd;
}

[[nodiscard]] int serve(const Options& options) {
    const int activated = systemd_socket();
    const bool activated_socket = activated >= 0;
    const int socket = activated_socket ? activated
                                      : (options.direct_socket ? bind_direct_socket(*options.direct_socket)
                                                               : -1);
    if (socket < 0) {
        std::cerr << "archie-cortexd: requires a systemd AF_UNIX socket or --socket PATH\n";
        return kUnavailable;
    }
    const int flags = ::fcntl(socket, F_GETFL, 0);
    if (flags < 0 || ::fcntl(socket, F_SETFL, flags | O_NONBLOCK) != 0) return kDataError;
    struct sigaction action {};
    action.sa_handler = stop;
    ::sigemptyset(&action.sa_mask);
    if (::sigaction(SIGTERM, &action, nullptr) != 0 ||
        ::sigaction(SIGINT, &action, nullptr) != 0) return kDataError;
    while (keep_running != 0) {
        // Bound the wait even if SIGTERM arrives between the loop check and poll.
        // std::signal can restart a blocking accept, leaving shutdown stuck forever.
        pollfd ready{.fd = socket, .events = POLLIN, .revents = 0};
        const int available = ::poll(&ready, 1, 250);
        if (keep_running == 0) break;
        if (available == 0 || (available < 0 && errno == EINTR)) continue;
        if (available < 0 || (ready.revents & POLLIN) == 0) return kDataError;
        const int client = ::accept4(socket, nullptr, nullptr, SOCK_CLOEXEC);
        if (client < 0) {
            if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK) continue;
            std::cerr << "archie-cortexd: accept failed: " << std::strerror(errno) << '\n';
            return kDataError;
        }
        serve_client(client, options.graph_snapshot, activated_socket);
        (void)::close(client);
    }
    return EXIT_SUCCESS;
}

#endif

}  // namespace

int main(int argc, char* argv[]) {
    if (argc == 2 && std::string_view{argv[1]} == "--version") {
        std::cout << "archie-cortexd deterministic-v1\n";
        return EXIT_SUCCESS;
    }
    if (argc == 2 && std::string_view{argv[1]} == "--self-check") {
        return self_check() ? EXIT_SUCCESS : EXIT_FAILURE;
    }
    const auto options = parse_options(argc, argv);
    if (!options) {
        std::cerr << "usage: archie-cortexd [--graph-snapshot PATH] [--socket PATH]\n";
        return kDataError;
    }
#if defined(__unix__)
    return serve(*options);
#else
    (void)options;
    std::cerr << "archie-cortexd: Unix-domain IPC is required on the production image\n";
    return kUnavailable;
#endif
}
