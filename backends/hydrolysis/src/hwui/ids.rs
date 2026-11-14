//! Dense `u32` ids with free-list reuse.
//!
//! The replayer keeps every resource kind in an array indexed by id, so ids
//! stay dense: a released id is handed out again before the pool grows.

use super::HwuiError;
use super::protocol::NONE;

/// One resource kind's id space.
#[derive(Debug)]
pub struct IdPool {
    kind: &'static str,
    next: u32,
    free: Vec<u32>,
}

impl IdPool {
    /// An empty pool naming `kind` in its exhaustion error.
    pub const fn new(kind: &'static str) -> Self {
        Self {
            kind,
            next: 0,
            free: Vec::new(),
        }
    }

    /// The most recently released id, else the next fresh one.
    ///
    /// # Errors
    ///
    /// [`HwuiError::IdsExhausted`] when every id below [`NONE`] is live.
    pub fn acquire(&mut self) -> Result<u32, HwuiError> {
        if let Some(id) = self.free.pop() {
            return Ok(id);
        }
        if self.next == NONE {
            return Err(HwuiError::IdsExhausted { kind: self.kind });
        }
        let id = self.next;
        self.next += 1;
        Ok(id)
    }

    /// Returns `id` for reuse.
    pub fn release(&mut self, id: u32) {
        debug_assert!(id < self.next && !self.free.contains(&id));
        self.free.push(id);
    }

    /// Live ids.
    #[cfg(test)]
    pub const fn live(&self) -> usize {
        self.next as usize - self.free.len()
    }
}

#[cfg(test)]
mod tests {
    use super::IdPool;

    #[test]
    fn released_ids_are_reused_before_the_pool_grows() {
        let mut pool = IdPool::new("node");
        let a = pool.acquire().unwrap();
        let b = pool.acquire().unwrap();
        assert_eq!((a, b), (0, 1));
        pool.release(a);
        assert_eq!(pool.acquire().unwrap(), 0);
        assert_eq!(pool.acquire().unwrap(), 2);
        assert_eq!(pool.live(), 3);
    }
}
