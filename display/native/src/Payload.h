#pragma once

// Qt JSON in, neutral tree out.
//
// The visual model refuses to know about Qt. That is the whole reason its decisions can
// be tested on a machine with no GPU, no event loop and no daemon, and it costs exactly
// this file: one total conversion from what the transport produced to what the model
// reads.
//
// Nothing is interpreted here. A number stays a number and an unexpected shape stays an
// unexpected shape, because deciding what a *valid* payload means is the model's job and
// `IngestReport` is where it says so. A converter that quietly dropped a malformed field
// would be answering a question the model exists to ask.

#include <QJsonValue>

#include "archie/visual/value.hpp"

namespace archie::native {

/// Convert decoded JSON into the model's neutral tree.
///
/// Total: every `QJsonValue` maps to some `Value`, including `Undefined`, which becomes
/// null. A conversion that could fail would put a second failure mode in front of the
/// one the model already reports.
archie::visual::Value fromJson(const QJsonValue &value);

}  // namespace archie::native
