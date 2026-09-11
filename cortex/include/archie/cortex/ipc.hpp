// Bounded binary IPC for the unprivileged Cortex daemon.
//
// Unix socket ownership is enforced by systemd. This protocol carries no command,
// credential, lease or graph-mutation message: it accepts a policy-filtered immutable
// snapshot and returns a ContextBundle or a typed refusal.

#pragma once

#include <cstdint>
#include <optional>
#include <vector>

#include "archie/cortex/contracts.hpp"

namespace archie::cortex::ipc {

constexpr std::uint8_t kProtocolVersion = 1;
constexpr std::uint32_t kMaximumFrameBytes = 1U << 20U;
constexpr std::uint32_t kMaximumSnapshotBytes = 32U << 20U;

enum class MessageType : std::uint8_t {
    GraphSnapshot = 1,
    ContextRequest = 2,
    ContextResponse = 3,
    Refusal = 4,
};

struct Refusal {
    ContractRefusal code = ContractRefusal::InvalidModelManifest;
};

[[nodiscard]] std::vector<std::uint8_t> encode(const GraphSnapshot& snapshot);
[[nodiscard]] std::vector<std::uint8_t> encode(const ContextRequest& request);
[[nodiscard]] std::vector<std::uint8_t> encode(const ContextBundle& bundle);
[[nodiscard]] std::vector<std::uint8_t> encode(const Refusal& refusal);

[[nodiscard]] std::optional<GraphSnapshot> decode_graph_snapshot(
    const std::vector<std::uint8_t>& bytes) noexcept;
[[nodiscard]] std::optional<ContextRequest> decode_context_request(
    const std::vector<std::uint8_t>& bytes) noexcept;
[[nodiscard]] std::optional<ContextBundle> decode_context_response(
    const std::vector<std::uint8_t>& bytes) noexcept;
[[nodiscard]] std::optional<Refusal> decode_refusal(
    const std::vector<std::uint8_t>& bytes) noexcept;

}  // namespace archie::cortex::ipc
