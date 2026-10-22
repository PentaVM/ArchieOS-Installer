//! Words the LAN inventory, the context feeder and Core's unified view share about how a
//! device was observed.

/// Sources that read a cache rather than hear the device: the system's neighbour table
/// and Sentry's own observation history. A device seen only by these is a candidate, not
/// a device seen now.
pub const CACHE_SOURCES: [&str; 2] = ["neighbor-cache", "history-cache"];

/// Whether every source behind a device only read a cache. An empty list is not
/// cache-only: it says nothing either way.
pub fn cache_only<S: AsRef<str>>(seen_by: &[S]) -> bool {
    !seen_by.is_empty()
        && seen_by
            .iter()
            .all(|source| CACHE_SOURCES.contains(&source.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::cache_only;

    #[test]
    fn only_a_list_of_cache_reads_is_cache_only() {
        assert!(cache_only(&["neighbor-cache"]));
        assert!(cache_only(&["neighbor-cache", "history-cache"]));
        assert!(!cache_only(&["neighbor-cache", "ssdp"]));
        assert!(!cache_only::<&str>(&[]));
    }
}
