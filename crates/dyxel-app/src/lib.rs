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
pub use dyxel_state::{State, StateSignalExt, use_effect, use_memo, use_state};

// Re-export view components
pub use dyxel_view::{
    AlignItems, BaseView, Button, FlexDirection, FlexWrap, JustifyContent, Text, View,
    force_layout, rsx, set_text,
};

// Re-export flex components
pub use dyxel_view::components::{
    ButtonSize, ButtonVariant, Color, Column, CrossAxisAlignment, Divider, MainAxisAlignment,
    Padding, Row, Spacer,
};

// Re-export shared types
pub use dyxel_shared::{LpExt, PxExt, SizeUnit, lp, px};

/// Prelude module - import everything you need
pub mod prelude {
    pub use crate::{
        AlignItems, BaseView, Button, FlexDirection, FlexWrap, JustifyContent, Text, View,
        force_layout, rsx, set_text,
    };
    pub use crate::{
        ButtonSize, ButtonVariant, Color, Column, CrossAxisAlignment, Divider, MainAxisAlignment,
        Padding, Row, Spacer,
    };
    pub use crate::{State, StateSignalExt, app, use_effect, use_memo, use_state};
    pub use dyxel_shared::{LpExt, PxExt, SizeUnit, lp, px};
}

/// Initialize the text update hook from dyxel-view
pub fn init_state_system() {
    dyxel_state::register_text_update_hook(set_text_wrapper);
}

fn set_text_wrapper(node_id: u32, text: &str) {
    set_text(node_id, text);
}
