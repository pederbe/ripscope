//! Hash maps and sets keyed by names, paths and ids the process itself reads. They use a
//! fast hasher instead of the standard library's DoS-resistant one: the keys come from
//! the user's own source tree, and hashing them dominated several lookups.

pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;
pub(crate) type HashSet<K> = std::collections::HashSet<K, foldhash::fast::RandomState>;
