//! Procedural macros for dyxel-app

use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, ItemFn};

/// Convert CamelCase to snake_case
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_was_upper = false;
    
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !prev_was_upper {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_was_upper = true;
        } else {
            result.push(c);
            prev_was_upper = false;
        }
    }
    
    result
}

/// #[app] macro - transforms app function into Dyxel app
///
/// # Example
/// ```rust,ignore
/// use dyxel_app_macro::app;
/// use dyxel_state::use_state;
///
/// #[app]
/// fn counter() {
///     let count = use_state(|| 0);
///     // ...
/// }
/// ```
#[proc_macro_attribute]
pub fn app(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);
    
    let fn_name = &input_fn.sig.ident;
    let fn_block = &input_fn.block;
    let fn_vis = &input_fn.vis;
    
    // Convert function name to snake_case for generated identifiers
    let fn_name_snake = to_snake_case(&fn_name.to_string());
    
    // Generate internal function names (snake_case)
    let user_fn_name = format_ident!("_{}_user_app", fn_name_snake);
    let init_fn_name = format_ident!("_{}_init", fn_name_snake);
    let tick_fn_name = format_ident!("_{}_tick", fn_name_snake);
    // Static variable should be SCREAMING_SNAKE_CASE
    let init_flag_name = format_ident!("_{}_INITIALIZED", fn_name_snake.to_uppercase());
    
    // Generate the transformed code - flat structure, no module wrapper
    let expanded = quote! {
        /// The user's app function (renamed to avoid conflict)
        #fn_vis fn #user_fn_name() -> impl ::dyxel_view::BaseView {
            #fn_block
        }
        
        /// Storage for init state
        static mut #init_flag_name: bool = false;
        
        /// Initialize the app - called once on startup
        #fn_vis fn #init_fn_name() {
            unsafe {
                if #init_flag_name { return; }
                #init_flag_name = true;
            }
            
            // Set up panic hook
            ::dyxel_view::init_panic_hook();
            
            // Initialize state system
            ::dyxel_app::init_state_system();
            
            // Run user's app setup
            let _view = #user_fn_name();
            
            // Force initial layout
            ::dyxel_view::force_layout();
        }
        
        /// Per-frame tick - called every frame
        #fn_vis fn #tick_fn_name() {
            // Process any pending state updates
            ::dyxel_view::dyxel_view_tick();
        }
        
        // Export main() for Host - calls init()
        #[cfg(not(test))]
        #[unsafe(no_mangle)]
        #fn_vis extern "C" fn main() {
            #init_fn_name();
        }
        
        // Export guest_tick() for Host - calls tick()
        #[cfg(not(test))]
        #[unsafe(no_mangle)]
        #fn_vis extern "C" fn guest_tick() {
            #tick_fn_name();
        }
    };
    
    expanded.into()
}
