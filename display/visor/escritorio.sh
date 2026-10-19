#!/usr/bin/env bash
set -euo pipefail
# Compatibility entry point for the native desktop; experimental viewer is opt-in.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec bash "$DIR/../shell/start-native.sh" "$@"
