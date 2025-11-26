use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SpscQueue<T> {
    buf: Vec<UnsafeCell<Option<T>>>,
    cap: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

unsafe impl<T: Send> Send for SpscQueue<T> {}
unsafe impl<T: Send> Sync for SpscQueue<T> {}

impl<T> SpscQueue<T> {
    pub fn new(cap: usize) -> Self {
        let mut buf = Vec::with_capacity(cap);
        for _ in 0..cap {
            buf.push(UnsafeCell::new(None));
        }

        Self {
            buf,
            cap,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn try_push(&self, value: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);

        if (head + 1) % self.cap == tail {
            return Err(value);
        }

        unsafe {
            *self.buf[head].get() = Some(value);
        }

        self.head.store((head + 1) % self.cap, Ordering::Release);
        Ok(())
    }

    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        if head == tail {
            return None;
        }

        let val = unsafe {
            (*self.buf[tail].get()).take()
        };

        self.tail.store((tail + 1) % self.cap, Ordering::Release);
        val
    }
}
