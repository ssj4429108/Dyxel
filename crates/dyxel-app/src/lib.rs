//! Dyxel App Framework - Dioxus-style declarative UI
//!
//! Direct command-based updates to shared memory (no virtual DOM).
//!
//! # Example
//! ```rust
//! #[app]
//! fn Counter() {
//!     let mut count = use_state(|| 0);
//!
//!     rsx! {
//!         View {
//!             Text("Count: {count}")
//!             Button("+") {
//!                 on_tap: move || count += 1
//!             }
//!         }
//!     }
//! }
//! ```

// Re-export macro
pub use dyxel_app_macro::app;

// Re-export state system
pub use dyxel_state::{use_state, use_memo, use_effect, State, StateSignalExt};

// Re-export view components
pub use dyxel_view::{
    rsx, View, Text, Button, Column, Row,
    FlexDirection, JustifyContent, AlignItems,
    BaseView, set_text, force_layout,
};

// Re-export shared types
pub use dyxel_shared::{SizeUnit, px, lp, PxExt, LpExt};

/// Prelude module - import everything you need
pub mod prelude {
    pub use crate::{use_state, use_memo, use_effect, State, StateSignalExt, app};
    pub use crate::{
        rsx, View, Text, Button, Column, Row,
        FlexDirection, JustifyContent, AlignItems,
        BaseView, set_text, force_layout,
    };
    pub use dyxel_shared::{SizeUnit, px, lp, PxExt, LpExt};
}

/// Initialize the text update hook from dyxel-view
pub fn init_state_system() {
    dyxel_state::register_text_update_hook(set_text_wrapper);
}

fn set_text_wrapper(node_id: u32, text: &str) {
    set_text(node_id, text);
}
