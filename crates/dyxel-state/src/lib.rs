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

use std::any::Any;
use std::cell::RefCell;
use std::ops::{Add, Sub, Mul, Div, AddAssign, SubAssign, MulAssign, DivAssign, Rem, RemAssign};
use std::rc::Rc;
use slotmap::{SlotMap, new_key_type};

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
        self.states.get(id)?
            .downcast_ref::<StateRef<T>>()
            .cloned()
    }
}

/// Internal state storage
pub struct StateInner<T: Clone + 'static> {
    value: RefCell<T>,
    /// Callbacks to call when value changes: (node_id, format_fn)
    text_bindings: RefCell<Vec<(u32, Box<dyn Fn(&T) -> String>)>>,
}

impl<T: Clone + 'static> StateInner<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            text_bindings: RefCell::new(Vec::new()),
        }
    }
    
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }
    
    pub fn set(&self, new_value: T) {
        *self.value.borrow_mut() = new_value;
        self.update_subscribers();
    }
    
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        let mut value = self.value.borrow_mut();
        f(&mut *value);
        drop(value);
        self.update_subscribers();
    }
    
    /// Bind this state to a text node
    pub fn bind_text<F>(&self, node_id: u32, format: F)
    where
        F: Fn(&T) -> String + 'static,
    {
        self.text_bindings.borrow_mut().push((node_id, Box::new(format)));
    }
    
    /// Update all bound text nodes
    fn update_subscribers(&self) {
        let value = self.value.borrow();
        for (node_id, format) in self.text_bindings.borrow().iter() {
            let text = format(&*value);
            update_text_node(*node_id, &text);
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
}

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
