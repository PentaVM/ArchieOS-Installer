#include "sentry/store/graph_store.hpp"

#include <utility>
#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <cerrno>
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>
#endif

namespace sentry::store {

std::expected<GraphTransaction, StoreError> GraphTransaction::acquire(
    const std::filesystem::path& graph_path) {
    // Resolve relative paths and existing symlinks so aliases of one graph use one lock.
    std::error_code ec;
    auto path = std::filesystem::weakly_canonical(std::filesystem::absolute(graph_path, ec), ec);
    if (ec || path.empty()) {
        return std::unexpected(StoreError::LockUnavailable);
    }
    path += ".lock";
#if defined(_WIN32)
    const HANDLE handle = CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE,
                                     FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_ALWAYS,
                                     FILE_ATTRIBUTE_NORMAL, nullptr);
    if (handle == INVALID_HANDLE_VALUE) {
        return std::unexpected(StoreError::LockUnavailable);
    }
    OVERLAPPED position{};
    if (!LockFileEx(handle, LOCKFILE_EXCLUSIVE_LOCK, 0, 1, 0, &position)) {
        CloseHandle(handle);
        return std::unexpected(StoreError::LockUnavailable);
    }
    return GraphTransaction{reinterpret_cast<std::intptr_t>(handle)};
#else
    const int handle = ::open(path.c_str(), O_CREAT | O_RDWR | O_CLOEXEC, 0600);
    if (handle < 0) {
        return std::unexpected(StoreError::LockUnavailable);
    }
    int locked;
    do {
        locked = ::flock(handle, LOCK_EX);
    } while (locked < 0 && errno == EINTR);
    if (locked < 0) {
        ::close(handle);
        return std::unexpected(StoreError::LockUnavailable);
    }
    return GraphTransaction{handle};
#endif
}

GraphTransaction::GraphTransaction(GraphTransaction&& other) noexcept
    : handle_(std::exchange(other.handle_, -1)) {}

GraphTransaction& GraphTransaction::operator=(GraphTransaction&& other) noexcept {
    if (this != &other) {
        release();
        handle_ = std::exchange(other.handle_, -1);
    }
    return *this;
}

GraphTransaction::~GraphTransaction() { release(); }

void GraphTransaction::release() noexcept {
    if (handle_ == -1) {
        return;
    }
#if defined(_WIN32)
    OVERLAPPED position{};
    const auto handle = reinterpret_cast<HANDLE>(handle_);
    UnlockFileEx(handle, 0, 1, 0, &position);
    CloseHandle(handle);
#else
    // close releases flock even after an error/exception in the caller.
    ::close(static_cast<int>(handle_));
#endif
    handle_ = -1;
}

}  // namespace sentry::store
