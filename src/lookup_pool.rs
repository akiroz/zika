use std::hash::Hash;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::collections::{HashMap, VecDeque};
use lru::LruCache;

pub struct LookupPool<A, B> {
    range: Range<B>,
    pool: VecDeque<B>,
    forward: LruCache<A, B>,
    reverse: HashMap<B, A>,
}

impl <A, B> LookupPool<A, B> where
    A: Eq + Hash + Clone,
    B: Eq + Hash,
    Range<B>: ExactSizeIterator<Item = B>,
{
    pub fn new(range: Range<B>) -> Self {
        Self {
            range: range,
            pool: VecDeque::new(),
            forward: LruCache::new(NonZeroUsize::new(range.len() - 1).unwrap()),
            reverse: HashMap::new(),
        }
    }

    pub fn get_forward(&mut self, a: &A) -> B {
        if let Some(&b) = self.forward.get(a) {
            return b;
        }
        let b = self.pool.pop_front().unwrap_or(self.range.next().unwrap());
        if let Some((.., v)) = self.forward.push(a.clone(), b) {
            if v != b {
                self.reverse.remove(&v);
                self.pool.push_back(v);
            }
        }
        self.reverse.insert(b, a.clone());
        b
    }

    pub fn get_reverse(&self, b: &B) -> Option<&A> {
        self.reverse.get(b)
    }

    pub fn cap(&self) -> usize {
        self.forward.cap().get()
    } 

    pub fn resize(&mut self, range: Range<B>) {
        self.range = range;
        self.pool.clear();
        self.reverse.clear();
        self.forward.clear();
        self.forward.resize(NonZeroUsize::new(range.len() - 1).unwrap());
    }
}