//! Compact property storage for stored nodes and relationships.
//!
//! A `BTreeMap<String, Value>` allocates a whole tree leaf (room for 11
//! entries, ~650 bytes) even for one property, and each node owns a copy of
//! every key. [`Properties`] keeps the entries in one exact-size boxed slice,
//! sorted by key (so iteration order matches `PropertyMap`), and the keys are
//! `Arc<str>` shared by every node through a [`KeyInterner`].

use std::collections::HashSet;
use std::sync::Arc;

use crate::value::{PropertyMap, Value};

/// Shares one allocation per distinct property key.
#[derive(Debug, Clone, Default)]
pub struct KeyInterner {
    keys: HashSet<Arc<str>>,
}

impl KeyInterner {
    pub fn intern(&mut self, key: &str) -> Arc<str> {
        if let Some(k) = self.keys.get(key) {
            return Arc::clone(k);
        }
        let k: Arc<str> = Arc::from(key);
        self.keys.insert(Arc::clone(&k));
        k
    }
}

/// The properties of a stored node or relationship, sorted by key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Properties {
    entries: Box<[(Arc<str>, Value)]>,
}

impl Properties {
    /// Builds from a map whose `null` values were already dropped.
    pub fn from_map(map: PropertyMap, keys: &mut KeyInterner) -> Self {
        // BTreeMap iterates in key order, so the slice comes out sorted.
        Properties { entries: map.into_iter().map(|(k, v)| (keys.intern(&k), v)).collect() }
    }

    fn find(&self, key: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|(k, _)| (**k).cmp(key))
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.find(key).ok().map(|i| &self.entries[i].1)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.find(key).is_ok()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> + '_ {
        self.entries.iter().map(|(k, v)| (&**k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.iter().map(|(k, _)| &**k)
    }

    pub fn to_map(&self) -> PropertyMap {
        self.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    pub fn into_map(self) -> PropertyMap {
        self.entries.into_vec().into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    /// Sets `key` and returns the previous value.
    pub fn insert(&mut self, key: &str, value: Value, keys: &mut KeyInterner) -> Option<Value> {
        match self.find(key) {
            Ok(i) => Some(std::mem::replace(&mut self.entries[i].1, value)),
            Err(i) => {
                let mut v = std::mem::take(&mut self.entries).into_vec();
                v.insert(i, (keys.intern(key), value));
                self.entries = v.into_boxed_slice();
                None
            }
        }
    }

    /// Removes `key` and returns its value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.find(key).ok()?;
        let mut v = std::mem::take(&mut self.entries).into_vec();
        let (_, old) = v.remove(i);
        self.entries = v.into_boxed_slice();
        Some(old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_insert_remove_and_shared_keys() {
        let mut keys = KeyInterner::default();
        let map = PropertyMap::from([("b".into(), Value::from(2)), ("a".into(), Value::from(1))]);
        let mut p = Properties::from_map(map.clone(), &mut keys);
        assert_eq!(p.keys().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(p.get("b"), Some(&Value::from(2)));
        assert_eq!(p.get("c"), None);

        assert_eq!(p.insert("c", Value::from(3), &mut keys), None);
        assert_eq!(p.insert("a", Value::from(10), &mut keys), Some(Value::from(1)));
        assert_eq!(p.keys().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(p.remove("b"), Some(Value::from(2)));
        assert_eq!(p.remove("b"), None);
        assert_eq!(p.to_map(), PropertyMap::from([("a".into(), Value::from(10)), ("c".into(), Value::from(3))]));

        let q = Properties::from_map(map, &mut keys);
        assert!(Arc::ptr_eq(&p.entries[0].0, &q.entries[0].0), "keys are shared");
        assert!(Properties::default().is_empty());
    }
}
