#pragma once

#include "sentry/store/graph_store.hpp"

namespace sentry::store {

inline constexpr std::size_t kMaxProjectionBytes = 8 * 1024 * 1024;
inline constexpr std::size_t kMaxProjectionNodes = 4096;
inline constexpr std::size_t kMaxProjectionEdges = 16384;
inline constexpr std::size_t kMaxProjectionIdBytes = 192;

enum class ProjectionError { TooLarge, InvalidGraph, InvalidUtf8 };
[[nodiscard]] std::string_view describe(ProjectionError error) noexcept;

// Read-only schema_version 1 projection. The graph must come from GraphStore::load or
// parse, whose refusal rules remain the trust boundary. IDs/endpoints/UTF-8 and output
// limits are checked here before a complete JSON value is returned. No partial output.
// Tags are the existing wanda.tag traits, with the same provenance as ordinary traits.
// Sensitive values AND their source notes are hidden unless reveal_sensitive is explicit.
[[nodiscard]] std::expected<std::string, ProjectionError> project_wanda_json(
    const PersonGraph& graph, bool reveal_sensitive = false);

}  // namespace sentry::store
