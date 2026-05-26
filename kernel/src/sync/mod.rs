use spin::Mutex;

pub mod mutex;

pub struct SpinLock<T>(Mutex<T>);

impl<T> SpinLock<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn lock(&self) -> spin::MutexGuard<'_, T> {
        self.0.lock()
    }
}
