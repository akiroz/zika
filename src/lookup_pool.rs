use lru::LruCache;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::num::NonZeroUsize;

pub struct LookupPool<A, B, I> {
    iterator: I,
    pool: VecDeque<B>,
    forward: LruCache<A, B>,
    reverse: HashMap<B, A>,
}

impl<A, B, I> LookupPool<A, B, I>
where
    A: Eq + Hash + Clone,
    B: Eq + Hash + Copy,
    I: ExactSizeIterator<Item = B>,
{
    pub fn new(iterator: I) -> Self {
        let length = iterator.len() - 1;
        Self {
            iterator,
            pool: VecDeque::new(),
            forward: LruCache::new(
                NonZeroUsize::new(length).expect("non-zero range for lookup pool"),
            ),
            reverse: HashMap::new(),
        }
    }

    pub fn get_forward(&mut self, a: &A) -> B {
        match self.forward.get(a) {
            Some(b) => *b,
            None => {
                let b = self
                    .pool
                    .pop_front()
                    .unwrap_or_else(|| self.iterator.next().unwrap());
                if let Some((.., v)) = self.forward.push(a.clone(), b) {
                    if v != b {
                        self.reverse.remove(&v);
                        self.pool.push_back(v);
                    }
                }
                self.reverse.insert(b, a.clone());
                b
            }
        }
    }

    pub fn get_reverse(&self, b: &B) -> Option<&A> {
        self.reverse.get(b)
    }

    pub fn cap(&self) -> usize {
        self.forward.cap().get()
    }

    // pub fn resize(&mut self, range: Range<B>) {
    //     let size = range.len() - 1;
    //     self.range = range;
    //     self.pool.clear();
    //     self.reverse.clear();
    //     self.forward.clear();
    //     self.forward.resize(NonZeroUsize::new(size).unwrap());
    // }
}
