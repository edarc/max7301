//! Provides mutual exclusion for various environments.

/// Any type that can implement `IOMutex` can be used as a mutex for synchronizing access to the
/// I/O device (either pins in an expander, or multiple expanders on a bus).
///
/// If the `std` feature is enabled, then `IOMutex` is implemented for `std::sync::Mutex`. If
/// `cortexm` is enabled, then `IOMutex` is implemented for
/// `cortex_m::interrupt::Mutex<core::cell::RefCell>` (the `RefCell` is needed to add mutability
/// which the mutex does not provide for some reason).
///
/// If either of these features is enabled, then the type alias [`DefaultMutex<T>`] will point to
/// the corresponding mutex type to use.
pub trait IOMutex<T> {
    /// Construct a new instance of this mutex containing the value `v`.
    fn new(v: T) -> Self;

    /// Lock the mutex and call the closure `f` as a critical section, passing a mutable reference
    /// to the owned value. Returns the value returned by `f`.
    fn lock<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> R;
}

#[cfg(feature = "std")]
pub type DefaultMutex<T> = std::sync::Mutex<T>;

#[cfg(feature = "cortexm")]
pub type DefaultMutex<T> = cortex_m::interrupt::Mutex<core::cell::RefCell<T>>;

#[cfg(feature = "std")]
impl<T> IOMutex<T> for std::sync::Mutex<T> {
    fn new(v: T) -> Self {
        std::sync::Mutex::new(v)
    }
    fn lock<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> R {
        let mut v = self.lock().unwrap();
        f(&mut v)
    }
}

#[cfg(feature = "cortexm")]
impl<T> IOMutex<T> for cortex_m::interrupt::Mutex<core::cell::RefCell<T>> {
    fn new(v: T) -> Self {
        cortex_m::interrupt::Mutex::new(core::cell::RefCell::new(v))
    }
    fn lock<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> R {
        cortex_m::interrupt::free(|cs| {
            let mut v = self.borrow(cs).borrow_mut();
            f(&mut v)
        })
    }
}
