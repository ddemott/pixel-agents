#![allow(dead_code)]
// Pre-app input queue: raw bytes captured during capability probing
// that must be replayed before reading fresh stdin.

use bytes::{Bytes, BytesMut};

pub struct InputQueue {
    buf: BytesMut,
}

impl InputQueue {
    pub fn new() -> Self {
        Self { buf: BytesMut::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn drain(&mut self) -> Bytes {
        self.buf.split().freeze()
    }
}

impl Default for InputQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_on_new() {
        let mut q = InputQueue::new();
        assert!(q.is_empty());
        assert_eq!(q.drain().len(), 0);
    }

    #[test]
    fn push_and_drain() {
        let mut q = InputQueue::new();
        q.push(b"hello");
        q.push(b" world");
        assert!(!q.is_empty());
        let out = q.drain();
        assert_eq!(&out[..], b"hello world");
        assert!(q.is_empty());
    }

    #[test]
    fn drain_clears() {
        let mut q = InputQueue::new();
        q.push(b"abc");
        let _ = q.drain();
        q.push(b"def");
        assert_eq!(&q.drain()[..], b"def");
    }
}
