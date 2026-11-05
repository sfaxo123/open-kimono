//! Key/value storage behind `kv.get` / `kv.set`. The host passes the
//! service's own id as the namespace; a guest cannot name another namespace.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

pub trait KvBackend {
    fn get(&self, namespace: &str, key: &str) -> Option<Vec<u8>>;
    fn set(&mut self, namespace: &str, key: &str, value: &[u8]);
    /// Bytes (keys + values) stored in `namespace`, for quota checks.
    fn used_bytes(&self, namespace: &str) -> usize;
}

/// In-memory backend; one map per namespace.
#[derive(Debug, Default)]
pub struct MemKv {
    spaces: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
}

impl MemKv {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keys of one namespace (for inspection and tests).
    pub fn keys(&self, namespace: &str) -> Vec<String> {
        self.spaces.get(namespace).map_or_else(Vec::new, |m| m.keys().cloned().collect())
    }
}

impl KvBackend for MemKv {
    fn get(&self, namespace: &str, key: &str) -> Option<Vec<u8>> {
        self.spaces.get(namespace)?.get(key).cloned()
    }

    fn set(&mut self, namespace: &str, key: &str, value: &[u8]) {
        self.spaces
            .entry(String::from(namespace))
            .or_default()
            .insert(String::from(key), value.to_vec());
    }

    fn used_bytes(&self, namespace: &str) -> usize {
        self.spaces
            .get(namespace)
            .map_or(0, |m| m.iter().map(|(k, v)| k.len() + v.len()).sum())
    }
}

/// Lets several services share one backend (each still sees only its namespace).
impl<B: KvBackend> KvBackend for Rc<RefCell<B>> {
    fn get(&self, namespace: &str, key: &str) -> Option<Vec<u8>> {
        self.borrow().get(namespace, key)
    }

    fn set(&mut self, namespace: &str, key: &str, value: &[u8]) {
        self.borrow_mut().set(namespace, key, value)
    }

    fn used_bytes(&self, namespace: &str) -> usize {
        self.borrow().used_bytes(namespace)
    }
}
