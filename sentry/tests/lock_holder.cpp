// Subprocess fixture for proving kernel-owned graph locks survive neither exit nor kill.
#include "sentry/store/graph_store.hpp"
#include <cstdio>

int main(int argc, char** argv) {
    if (argc != 2) { return 64; }
    auto transaction = sentry::store::GraphTransaction::acquire(argv[1]);
    if (!transaction) { return 65; }
    std::puts("locked");
    std::fflush(stdout);
    std::getchar();
    return 0;
}
