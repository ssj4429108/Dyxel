//! Dyxel State System - React/Dioxus-style state management
//!
//! Direct command-based updates to shared memory (no virtual DOM).
//!
//! # Example
//! ```rust,ignore
//! use dyxel_state::use_state;
//!
//! let mut count = use_state(|| 0);
//! count.set(5);
//! assert_eq!(count.get(), 5);
//! ```

use dyxel_shared::SizeUnit;
use futures_signals::signal::Signal;
use slotmap::{SlotMap, new_key_type};
use std::any::Any;
use std::cell::RefCell;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Rem, RemAssign, Sub, SubAssign};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

new_key_type! {
    /// Unique identifier for a state
    pub struct StateId;
}

/// Internal state storage - shared via Rc
type StateRef<T> = Rc<StateInner<T>>;

thread_local! {
    static STATE_MANAGER: RefCell<StateManager> = RefCell::new(StateManager::new());
}

/// Manager for all states
pub struct StateManager {
    states: SlotMap<StateId, Box<dyn Any>>,
}

impl StateManager {
    pub fn new() -> Self {
        Self {
            states: SlotMap::with_key(),
        }
    }

    /// Insert a new state
    pub fn insert<T: Clone + 'static>(&mut self, inner: StateRef<T>) -> StateId {
        self.states.insert(Box::new(inner))
    }

    /// Get a state by ID
    pub fn get<T: Clone + 'static>(&self, id: StateId) -> Option<StateRef<T>> {
        self.states.get(id)?.downcast_ref::<StateRef<T>>().cloned()
    }
}

/// Callback for signal-based subscriptions
type SignalCallback<T> = Box<dyn FnMut(&T)>;

/// Version counter for tracking state changes
static VERSION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Internal state storage
pub struct StateInner<T: Clone + 'static> {
    value: RefCell<T>,
    /// Callbacks to call when value changes: (node_id, format_fn)
    text_bindings: RefCell<Vec<(u32, Box<dyn Fn(&T) -> String>)>>,
    /// Signal-based subscriptions for reactive binding
    signal_subscribers: RefCell<Vec<SignalCallback<T>>>,
    /// Version counter for change detection
    version: RefCell<u64>,
}

impl<T: Clone + 'static> StateInner<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            text_bindings: RefCell::new(Vec::new()),
            signal_subscribers: RefCell::new(Vec::new()),
            version: RefCell::new(
                VERSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            ),
        }
    }

    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    pub fn set(&self, new_value: T) {
        *self.value.borrow_mut() = new_value;
        let new_version = VERSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.version.borrow_mut() = new_version;
        self.update_subscribers();
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        let mut value = self.value.borrow_mut();
        f(&mut *value);
        drop(value);
        *self.version.borrow_mut() =
            VERSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.update_subscribers();
    }

    /// Bind this state to a text node
    pub fn bind_text<F>(&self, node_id: u32, format: F)
    where
        F: Fn(&T) -> String + 'static,
    {
        self.text_bindings
            .borrow_mut()
            .push((node_id, Box::new(format)));
    }

    /// Subscribe to state changes via Signal
    pub fn subscribe_signal<F>(&self, callback: F)
    where
        F: FnMut(&T) + 'static,
    {
        self.signal_subscribers
            .borrow_mut()
            .push(Box::new(callback));
    }

    /// Get current version
    pub fn get_version(&self) -> u64 {
        *self.version.borrow()
    }

    /// Update all bound text nodes and signal subscribers
    fn update_subscribers(&self) {
        let value = self.value.borrow();
        for (node_id, format) in self.text_bindings.borrow().iter() {
            let text = format(&*value);
            update_text_node(*node_id, &text);
        }
        for callback in self.signal_subscribers.borrow_mut().iter_mut() {
            callback(&*value);
        }
    }
}

/// Hook to update text node - will be provided by dyxel-view
static mut TEXT_UPDATE_HOOK: Option<fn(u32, &str)> = None;

/// Register the text update hook (called by dyxel-view)
pub fn register_text_update_hook(hook: fn(u32, &str)) {
    unsafe {
        TEXT_UPDATE_HOOK = Some(hook);
    }
}

fn update_text_node(node_id: u32, text: &str) {
    unsafe {
        if let Some(hook) = TEXT_UPDATE_HOOK {
            hook(node_id, text);
        }
    }
}

/// A reactive state handle (Copy)
#[derive(Clone)]
pub struct State<T: Clone + 'static> {
    inner: StateRef<T>,
}

impl<T: Clone + 'static> State<T> {
    /// Create a new state (internal use, use use_state instead)
    pub fn new<F>(init: F) -> Self
    where
        F: FnOnce() -> T,
    {
        let inner = Rc::new(StateInner::new(init()));
        let id = STATE_MANAGER.with(|m| m.borrow_mut().insert(inner.clone()));
        // Store ID for potential future use
        let _ = id;

        Self { inner }
    }

    /// Get the current value
    pub fn get(&self) -> T {
        self.inner.get()
    }

    /// Set a new value
    pub fn set(&self, value: T) {
        self.inner.set(value);
    }

    /// Update value with a function
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.inner.update(f);
    }

    /// Bind this state to a text node with formatter
    pub fn bind_text<F>(&self, node_id: u32, format: F)
    where
        F: Fn(&T) -> String + 'static,
    {
        self.inner.bind_text(node_id, format);
    }

    /// Bind with default Display format
    pub fn bind_display(&self, node_id: u32)
    where
        T: std::fmt::Display,
    {
        self.bind_text(node_id, |v| v.to_string());
    }

    /// Convert this State into a Signal for reactive binding
    ///
    /// This allows State to be used with `.sig()` method in RSX:
    /// ```rust,ignore
    /// let width = use_state(|| 100.0f32);
    /// rsx! {
    ///     View { width: {width.sig()} }
    /// }
    /// ```
    pub fn sig(&self) -> StateSignal<T> {
        StateSignal {
            state: self.inner.clone(),
            last_version: RefCell::new(self.inner.get_version()),
            is_first_poll: RefCell::new(true),
        }
    }
}

/// SizeUnit Signal for f32 State - wraps StateSignal and converts f32 to SizeUnit
pub struct SizeUnitSignal {
    inner: StateSignal<f32>,
}

impl Signal for SizeUnitSignal {
    type Item = crate::SizeUnit;

    fn poll_change(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Pin project to inner
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.inner) };
        match inner.poll_change(cx) {
            Poll::Ready(Some(v)) => Poll::Ready(Some(crate::SizeUnit::Lp(v))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Unpin for SizeUnitSignal {}

impl State<f32> {
    /// Convert f32 State to SizeUnit Signal (for width/height binding)
    pub fn sig_size(&self) -> SizeUnitSignal {
        SizeUnitSignal { inner: self.sig() }
    }
}

impl State<(u32, u32, u32, u32)> {
    /// Convert color State to color Signal (for color binding)
    pub fn sig_color(&self) -> StateSignal<(u32, u32, u32, u32)> {
        self.sig()
    }
}

/// A Signal implementation for State<T>
pub struct StateSignal<T: Clone + 'static> {
    state: StateRef<T>,
    last_version: RefCell<u64>,
    // Track if this is the first poll - we should always emit initial value
    is_first_poll: RefCell<bool>,
}

// StateSignal is Unpin because all fields are Unpin
impl<T: Clone + 'static> Unpin for StateSignal<T> {}

impl<T: Clone + 'static> Clone for StateSignal<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            last_version: RefCell::new(*self.last_version.borrow()),
            // Preserve is_first_poll state to ensure consistent behavior after clone
            is_first_poll: RefCell::new(*self.is_first_poll.borrow()),
        }
    }
}

impl<T: Clone + 'static> Signal for StateSignal<T> {
    type Item = T;

    fn poll_change(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Option<Self::Item>> {
        let current_version = self.state.get_version();
        let last_version = *self.last_version.borrow();
        let is_first = *self.is_first_poll.borrow();

        // Only emit value on first poll or when version has changed
        if is_first || current_version != last_version {
            *self.is_first_poll.borrow_mut() = false;
            *self.last_version.borrow_mut() = current_version;
            let value = self.state.get();
            Poll::Ready(Some(value))
        } else {
            Poll::Pending
        }
    }
}

// ===== Signal extension for State =====

/// Extension trait for converting State to Signal
pub trait StateSignalExt<T: Clone + 'static> {
    /// Convert State to Signal for reactive property binding
    fn sig(&self) -> StateSignal<T>;
}

impl<T: Clone + 'static> StateSignalExt<T> for State<T> {
    fn sig(&self) -> StateSignal<T> {
        State::sig(self)
    }
}

// Re-export futures-signals types for convenience
pub use futures_signals::signal::{Signal as FsSignal, SignalExt as FsSignalExt};

// ===== Operator Overloads =====

impl<T> AddAssign<T> for State<T>
where
    T: Clone + Add<Output = T> + 'static,
{
    fn add_assign(&mut self, rhs: T) {
        let current = self.get();
        self.set(current + rhs);
    }
}

impl<T> SubAssign<T> for State<T>
where
    T: Clone + Sub<Output = T> + 'static,
{
    fn sub_assign(&mut self, rhs: T) {
        let current = self.get();
        self.set(current - rhs);
    }
}

impl<T> MulAssign<T> for State<T>
where
    T: Clone + Mul<Output = T> + 'static,
{
    fn mul_assign(&mut self, rhs: T) {
        let current = self.get();
        self.set(current * rhs);
    }
}

impl<T> DivAssign<T> for State<T>
where
    T: Clone + Div<Output = T> + 'static,
{
    fn div_assign(&mut self, rhs: T) {
        let current = self.get();
        self.set(current / rhs);
    }
}

impl<T> RemAssign<T> for State<T>
where
    T: Clone + Rem<Output = T> + 'static,
{
    fn rem_assign(&mut self, rhs: T) {
        let current = self.get();
        self.set(current % rhs);
    }
}

// ===== Public API =====

/// Create a new state
///
/// # Example
/// ```rust
/// use dyxel_state::use_state;
///
/// let count = use_state(|| 0);
/// assert_eq!(count.get(), 0);
/// count.set(5);
/// assert_eq!(count.get(), 5);
/// ```
pub fn use_state<T, F>(init: F) -> State<T>
where
    T: Clone + 'static,
    F: FnOnce() -> T,
{
    State::new(init)
}

/// Create a computed state (memoized)
///
/// Recomputes when dependencies change (if we had dependency tracking).
/// For now, just a simple wrapper.
pub fn use_memo<T, F>(compute: F) -> State<T>
where
    T: Clone + 'static,
    F: Fn() -> T,
{
    State::new(&compute)
}

/// Create a side effect
///
/// Currently just executes immediately.
/// In the future, could track signal dependencies and re-run.
pub fn use_effect<F>(_f: F)
where
    F: Fn() + 'static,
{
    // For now, just run once
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_use_state() {
        let count = use_state(|| 0);
        assert_eq!(count.get(), 0);

        count.set(5);
        assert_eq!(count.get(), 5);
    }

    #[test]
    fn test_state_add_assign() {
        let mut count = use_state(|| 10);
        count += 5;
        assert_eq!(count.get(), 15);
    }

    #[test]
    fn test_state_update() {
        let count = use_state(|| 5);
        count.update(|v| *v *= 2);
        assert_eq!(count.get(), 10);
    }
}
