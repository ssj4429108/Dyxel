// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! RSX Macro - String Interpolation with Dynamic State Binding

use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Literal, Span, TokenTree};
use quote::{quote, quote_spanned};

/// RSX 宏 - 支持字符串插值和动态 State 绑定
///
/// 属性命名使用小驼峰 (camelCase)，例如:
/// - `fontSize` 而不是 `font_size`
/// - `backgroundColor` 而不是 `background_color`
/// - `onTap` 而不是 `on_tap`
///
/// 宏会自动将 camelCase 转换为 Rust 方法名的 snake_case。
#[proc_macro]
pub fn rsx(input: TokenStream) -> TokenStream {
    let input2 = proc_macro2::TokenStream::from(input);

    let first_token = input2.clone().into_iter().next();
    let input_span = first_token
        .as_ref()
        .map(|t| t.span())
        .unwrap_or_else(Span::call_site);

    match parse_rsx_element(input2) {
        Ok(node) => {
            let expanded = expand_node(&node);
            TokenStream::from(expanded)
        }
        Err(e) => {
            let err = syn::Error::new(input_span, e);
            TokenStream::from(err.to_compile_error())
        }
    }
}

#[derive(Debug, Clone)]
struct RsxNode {
    node_type: String,
    type_span: Span,
    props: Vec<(String, Vec<TokenTree>, Span)>,
    children: Vec<RsxNode>,
    is_var_ref: bool,
    var_span: Option<Span>,
}

fn parse_rsx_element(input: proc_macro2::TokenStream) -> Result<RsxNode, String> {
    let tokens: Vec<_> = input.into_iter().collect();
    let mut pos = 0;
    parse_element_with_span(&tokens, &mut pos)
}

fn parse_element_with_span(tokens: &[TokenTree], pos: &mut usize) -> Result<RsxNode, String> {
    let (node_type, type_span) = match tokens.get(*pos) {
        Some(TokenTree::Ident(ident)) => {
            let span = ident.span();
            let name = ident.to_string();
            *pos += 1;
            (name, span)
        }
        _ => return Err("Expected element type".to_string()),
    };

    // Check for variable reference
    if let Some(next) = tokens.get(*pos) {
        let is_group = matches!(next, TokenTree::Group(g) 
            if g.delimiter() == Delimiter::Parenthesis || g.delimiter() == Delimiter::Brace);

        if !is_group {
            return Ok(RsxNode {
                node_type,
                type_span,
                props: Vec::new(),
                children: Vec::new(),
                is_var_ref: true,
                var_span: Some(type_span),
            });
        }
    }

    // Parse () group
    let mut props = Vec::new();
    if let Some(TokenTree::Group(group)) = tokens.get(*pos) {
        if group.delimiter() == Delimiter::Parenthesis {
            *pos += 1;
            let args: Vec<_> = group.stream().into_iter().collect();

            if (node_type == "Text" || node_type == "Button") && !args.is_empty() {
                let first_span = args
                    .first()
                    .map(|t| t.span())
                    .unwrap_or_else(Span::call_site);
                if let Some(first) = args.first() {
                    let s = first.to_string();
                    if node_type == "Text"
                        && s.starts_with('"')
                        && s.contains('{')
                        && s.contains('}')
                    {
                        props.push(("value_dynamic".to_string(), args, first_span));
                    } else if node_type == "Button" {
                        // Button("Label") - label is the first argument
                        props.push(("label".to_string(), args, first_span));
                    } else {
                        props.push(("value".to_string(), args, first_span));
                    }
                }
            }
        }
    }

    // Parse {} block
    let mut children = Vec::new();
    if let Some(TokenTree::Group(group)) = tokens.get(*pos) {
        if group.delimiter() == Delimiter::Brace {
            *pos += 1;
            let inner: Vec<_> = group.stream().into_iter().collect();
            let mut inner_pos = 0;

            while inner_pos < inner.len() {
                while let Some(TokenTree::Punct(p)) = inner.get(inner_pos) {
                    if p.as_char() == ',' {
                        inner_pos += 1;
                    } else {
                        break;
                    }
                }

                match inner.get(inner_pos) {
                    None => break,
                    Some(TokenTree::Ident(ident)) => {
                        let name = ident.to_string();
                        let name_span = ident.span();

                        let is_property = matches!(
                            inner.get(inner_pos + 1),
                            Some(TokenTree::Punct(p)) if p.as_char() == ':'
                        );

                        if is_property {
                            inner_pos += 1; // skip property name
                                            // Skip the colon
                            if let Some(TokenTree::Punct(p)) = inner.get(inner_pos) {
                                if p.as_char() == ':' {
                                    inner_pos += 1;
                                }
                            }

                            // Collect value tokens
                            let mut value_tokens = Vec::new();
                            let brace_depth = 0;
                            let paren_depth = 0;
                            let bracket_depth = 0;
                            let mut pipe_depth = 0;
                            let mut in_closure_params = false;

                            while inner_pos < inner.len() {
                                match inner.get(inner_pos) {
                                    Some(TokenTree::Punct(p))
                                        if p.as_char() == ','
                                            && brace_depth == 0
                                            && paren_depth == 0
                                            && bracket_depth == 0
                                            && pipe_depth == 0 =>
                                    {
                                        inner_pos += 1;
                                        break;
                                    }
                                    Some(TokenTree::Punct(p)) if p.as_char() == '|' => {
                                        // Track closure pipe boundaries
                                        if pipe_depth == 0 {
                                            in_closure_params = !in_closure_params;
                                            pipe_depth = if in_closure_params { 1 } else { 0 };
                                        } else {
                                            pipe_depth = 0;
                                            in_closure_params = false;
                                        }
                                        value_tokens.push(inner[inner_pos].clone());
                                        inner_pos += 1;
                                    }
                                    Some(TokenTree::Group(_)) => {
                                        // Group is atomic, just push it
                                        value_tokens.push(inner[inner_pos].clone());
                                        inner_pos += 1;
                                    }
                                    Some(t) => {
                                        value_tokens.push(t.clone());
                                        inner_pos += 1;
                                    }
                                    None => break,
                                }
                            }

                            // Check if value is dynamic string
                            if value_tokens.len() == 1 {
                                if let Some(TokenTree::Literal(lit)) = value_tokens.first() {
                                    let s = lit.to_string();
                                    if s.starts_with('"') && s.contains('{') && s.contains('}') {
                                        props.push((
                                            format!("{}_dynamic", name),
                                            value_tokens,
                                            name_span,
                                        ));
                                        continue;
                                    }
                                }
                            }

                            props.push((name, value_tokens, name_span));
                        } else if is_flag_property(&name) {
                            // Flag property without value: expanded, disabled, etc.
                            // Treat as property with empty value
                            props.push((name, vec![], name_span));
                            inner_pos += 1; // skip the identifier
                        } else {
                            let child = parse_element_with_span(&inner, &mut inner_pos)?;
                            children.push(child);
                        }
                    }
                    Some(_) => {
                        inner_pos += 1;
                    }
                }
            }
        }
    }

    Ok(RsxNode {
        node_type,
        type_span,
        props,
        children,
        is_var_ref: false,
        var_span: None,
    })
}

fn expand_node(node: &RsxNode) -> proc_macro2::TokenStream {
    if node.is_var_ref {
        let var_name = proc_macro2::Ident::new(
            &node.node_type,
            node.var_span.unwrap_or_else(Span::call_site),
        );
        return quote! { #var_name };
    }

    let type_span = node.type_span;
    let node_type = proc_macro2::Ident::new(&node.node_type, type_span);
    let node_var = proc_macro2::Ident::new(
        &format!("_{}_node", node.node_type.to_lowercase()),
        Span::call_site(),
    );

    // Separate static props from dynamic bindings
    let mut static_props = Vec::new();
    let mut dynamic_bindings = Vec::new();

    for (name, value_tokens, name_span) in &node.props {
        if name.ends_with("_dynamic") || name == "value_dynamic" {
            let real_name = if name == "value_dynamic" {
                "value"
            } else {
                name.trim_end_matches("_dynamic")
            };
            dynamic_bindings.push((real_name.to_string(), value_tokens.clone(), *name_span));
        } else {
            static_props.push((name.clone(), value_tokens.clone(), *name_span));
        }
    }

    // Static property assignments
    let static_assignments: Vec<_> = static_props.iter().map(|(name, value_tokens, name_span)| {
        let value = tokens_to_stream(value_tokens);

        // Convert camelCase property names to snake_case for method calls
        // This allows RSX to use familiar camelCase while internal Rust code uses snake_case
        let name = if name.contains(|c: char| c.is_uppercase()) {
            camel_to_snake(name)
        } else {
            name.to_string()
        };

        // Special handling for gesture DSL
        if name == "gesture" {
            return quote_spanned! { *name_span =>
                .gesture(#value)
            };
        }
        
        // Check if value is a code block (for dynamic State binding)
        let is_code_block = value_tokens.len() == 1 && 
            matches!(value_tokens.first(), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace);
        
        // Check for State::get() pattern - either in code block or direct
        // Pattern: identifier . get ( )
        let tokens_to_check: Vec<_> = if is_code_block {
            match value_tokens.first() {
                Some(TokenTree::Group(g)) => g.stream().into_iter().collect(),
                _ => value_tokens.to_vec(),
            }
        } else {
            value_tokens.to_vec()
        };
        

        // Pattern: identifier . get ( )  - that's 4 tokens total
        let is_simple_state_get = tokens_to_check.len() == 4 &&
            matches!(tokens_to_check.get(0), Some(TokenTree::Ident(_))) &&
            matches!(tokens_to_check.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '.') &&
            matches!(tokens_to_check.get(2), Some(TokenTree::Ident(i)) if i.to_string() == "get") &&
            matches!(tokens_to_check.get(3), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis);
        
        // Pattern: single identifier (e.g., box_width) - treat as State
        // Exclude boolean keywords (true, false) which are Idents but not State variables
        let is_single_ident = tokens_to_check.len() == 1 &&
            matches!(tokens_to_check.get(0), Some(TokenTree::Ident(i)) if i.to_string() != "true" && i.to_string() != "false");
        
        if is_simple_state_get || is_single_ident {
            let state_ident = if is_simple_state_get {
                tokens_to_check.get(0).and_then(|t| match t { TokenTree::Ident(i) => Some(i.clone()), _ => None })
            } else {
                tokens_to_check.get(0).and_then(|t| match t { TokenTree::Ident(i) => Some(i.clone()), _ => None })
            };
            
            if let Some(state_ident) = state_ident {
                let method_ident = proc_macro2::Ident::new(&name, *name_span);
                
                // Choose appropriate signal method based on property name
                // width/height need sig_size() for f32 -> SizeUnit conversion
                // color needs sig_color() for (u32,u32,u32,u32) -> (u32,u32,u32,u32)
                // other properties use sig()
                let sig_method = if name == "width" || name == "height" {
                    quote! { sig_size }
                } else if name == "color" {
                    quote! { sig_color }
                } else {
                    quote! { sig }
                };
                
                // Generate dynamic binding using State's signal method
                return quote_spanned! { *name_span =>
                    .#method_ident(#state_ident.#sig_method())
                };
            }
        }
        
        // Special handling for flag properties (no value) - call method without arguments
        // e.g., expanded -> .expanded()
        if value_tokens.is_empty() {
            let method_ident = proc_macro2::Ident::new(&name, *name_span);
            return quote_spanned! { *name_span =>
                .#method_ident()
            };
        }

        // Special handling for unit value () - call method without arguments
        // e.g., expanded: () -> .expanded()
        if value_tokens.len() == 1 {
            if let Some(TokenTree::Group(g)) = value_tokens.first() {
                if g.delimiter() == Delimiter::Parenthesis && g.stream().is_empty() {
                    let method_ident = proc_macro2::Ident::new(&name, *name_span);
                    return quote_spanned! { *name_span =>
                        .#method_ident()
                    };
                }
            }
        }

        // Special handling for gestures
        // Simple versions (no event parameter): onTap, onDoubleTap, onLongPress, onPanSimple
        // Full versions (with GestureEvent): onTapEvent, onDoubleTapEvent, onLongPressEvent, onPan
        if name == "onTap" {
            return quote_spanned! { *name_span =>
                .on_tap(#value)
            };
        }
        if name == "onTapEvent" {
            return quote_spanned! { *name_span =>
                .on_tap(#value)
            };
        }
        if name == "onDoubleTap" {
            return quote_spanned! { *name_span =>
                .on_double_tap(#value)
            };
        }
        if name == "onDoubleTapEvent" {
            return quote_spanned! { *name_span =>
                .on_double_tap(#value)
            };
        }
        if name == "onLongPress" {
            return quote_spanned! { *name_span =>
                .on_long_press(#value)
            };
        }
        if name == "onLongPressEvent" {
            return quote_spanned! { *name_span =>
                .on_long_press(#value)
            };
        }
        if name == "onPan" {
            return quote_spanned! { *name_span =>
                .on_pan(#value)
            };
        }
        if name == "onPanSimple" {
            return quote_spanned! { *name_span =>
                .on_pan_simple(#value)
            };
        }

        if is_code_block {
            // Regular code block - pass directly
            let method_ident = proc_macro2::Ident::new(&name, *name_span);

            return quote_spanned! { *name_span =>
                .#method_ident(#value)
            };
        }

        let method_ident = proc_macro2::Ident::new(&name, *name_span);
        
        quote_spanned! { *name_span =>
            .#method_ident(#value)
        }
    }).collect();

    // Dynamic bindings - use bind_text for automatic updates
    let dynamic_assignments: Vec<_> = dynamic_bindings
        .iter()
        .map(|(name, value_tokens, name_span)| {
            // Convert camelCase to snake_case for method names
            let name = if name.contains(|c: char| c.is_uppercase()) {
                camel_to_snake(name)
            } else {
                name.to_string()
            };

            let lit = tokens_to_stream(value_tokens);
            let lit_str = lit.to_string();

            let (format_str, vars) = parse_interpolation(&lit_str);

            if vars.is_empty() {
                // No interpolation, treat as static
                let method_ident = proc_macro2::Ident::new(&name, *name_span);
                return quote_spanned! { *name_span =>
                    .#method_ident(#lit)
                };
            }

            // Generate dynamic binding
            let format_lit = Literal::string(&format_str);

            // Build format arguments with proper type handling based on format spec
            let format_args: Vec<_> = vars
                .iter()
                .map(|var| {
                    let var_ident = proc_macro2::Ident::new(&var.name, Span::call_site());
                    if var.format_spec.is_empty() {
                        // No format spec - use to_string() for backward compatibility
                        quote! { #var_ident.get().to_string() }
                    } else {
                        // Has format spec - pass value directly to format!
                        // The format spec (e.g., .1, .2$) is already in the format string
                        quote! { #var_ident.get() }
                    }
                })
                .collect();

            quote_spanned! { *name_span =>
                .value({
                    let __initial = format!(#format_lit, #(#format_args),*);
                    __initial
                })
            }
        })
        .collect();

    // Generate post-creation bindings for dynamic text
    let binding_code: Vec<_> = dynamic_bindings
        .iter()
        .filter_map(|(name, value_tokens, _)| {
            if name != "value" {
                return None; // Only support text value for now
            }

            let lit = tokens_to_stream(value_tokens);
            let lit_str = lit.to_string();
            let (_format_str, vars) = parse_interpolation(&lit_str);

            if vars.is_empty() {
                return None;
            }

            // Generate bind_text calls - format spec only affects initial value
            // Updates use to_string for simplicity
            let binds: Vec<_> = vars
                .iter()
                .map(|var| {
                    let var_ident = proc_macro2::Ident::new(&var.name, Span::call_site());
                    quote! {
                        #var_ident.bind_text(#node_var.node_id(), |v| v.to_string());
                    }
                })
                .collect();

            Some(quote! { #(#binds)* })
        })
        .collect();

    // Children
    let child_assignments: Vec<_> = node
        .children
        .iter()
        .map(|child| {
            let child_expr = expand_node(child);
            quote! {
                let __child = #child_expr;
                #node_var = ::dyxel_view::BaseView::child(#node_var, __child.node_id());
            }
        })
        .collect();

    // Determine the module path for the component
    // Flex components are in ::dyxel_view::flex module
    // Button and TextInput are in ::dyxel_view::components module
    let is_flex_component = matches!(
        node.node_type.as_str(),
        "Column" | "Row" | "Spacer" | "Divider"
    );
    let is_button = node.node_type == "Button";
    let is_text_input = node.node_type == "TextInput";

    // Extract label from props if Button
    let label_expr = if is_button {
        node.props
            .iter()
            .find(|(name, _, _)| name == "label")
            .map(|(_, tokens, _)| tokens_to_stream(tokens))
    } else {
        None
    };

    let new_expr = if is_flex_component {
        quote! {
            ::dyxel_view::flex::#node_type::new()
        }
    } else if is_button {
        // Button::new("label") - label is required first argument
        if let Some(label) = label_expr {
            quote! {
                ::dyxel_view::components::button::#node_type::new(#label)
            }
        } else {
            // Fallback: use empty label
            quote! {
                ::dyxel_view::components::button::#node_type::new("")
            }
        }
    } else if is_text_input {
        quote! {
            ::dyxel_view::components::text_input::#node_type::new()
        }
    } else {
        quote! {
            ::dyxel_view::#node_type::new()
        }
    };

    quote_spanned! { type_span =>
        {
            let mut #node_var = #new_expr
                #(#static_assignments)*
                #(#dynamic_assignments)*;
            #(#binding_code)*
            #(#child_assignments)*
            #node_var
        }
    }
}

/// Parsed variable with optional format spec
#[derive(Debug)]
struct InterpVar {
    name: String,
    format_spec: String, // e.g., ".1", ":.2$", etc.
}

/// Parse "Count: {count}" -> ("Count: {}", ["count"])
/// Parse "Scale: {scale:.1}x" -> ("Scale: {:.1}x", [("scale", ".1")])
fn parse_interpolation(lit: &str) -> (String, Vec<InterpVar>) {
    let mut format_str = String::new();
    let mut vars = Vec::new();

    let content = if lit.starts_with('"') && lit.ends_with('"') {
        &lit[1..lit.len() - 1]
    } else {
        lit
    };

    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                format_str.push('{');
                continue;
            }

            let mut var_name = String::new();
            let mut format_spec = String::new();
            let mut in_format_spec = false;

            while let Some(&ch) = chars.peek() {
                if ch == '}' {
                    chars.next();
                    break;
                }
                if ch == ':' && !in_format_spec {
                    in_format_spec = true;
                    chars.next();
                    continue;
                }
                if in_format_spec {
                    format_spec.push(ch);
                } else {
                    var_name.push(ch);
                }
                chars.next();
            }

            if !var_name.is_empty() {
                if format_spec.is_empty() {
                    format_str.push_str("{}");
                } else {
                    format_str.push_str("{:");
                    format_str.push_str(&format_spec);
                    format_str.push('}');
                }
                vars.push(InterpVar {
                    name: var_name,
                    format_spec,
                });
            }
        } else if c == '}' {
            if chars.peek() == Some(&'}') {
                chars.next();
                format_str.push('}');
                continue;
            }
            format_str.push(c);
        } else if c == '\\' {
            if let Some(&next) = chars.peek() {
                format_str.push(c);
                format_str.push(next);
                chars.next();
            }
        } else {
            format_str.push(c);
        }
    }

    (format_str, vars)
}

fn tokens_to_stream(tokens: &[TokenTree]) -> proc_macro2::TokenStream {
    tokens.iter().cloned().collect()
}

/// Check if a property name is a flag property (no value needed)
/// These are properties like `expanded`, `disabled`, `clipToBounds` that
/// can be written as just the name without `: ()` or `: true`
///
/// Supports both camelCase (e.g., `clipToBounds`) and lowercase (e.g., `cliptobounds`)
fn is_flag_property(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "expanded" | "disabled" | "cliptobounds" | "hidden" | "enabled"
    )
}

fn camel_to_snake(camel: &str) -> String {
    let mut result = String::new();
    let mut prev_was_upper = false;

    for (i, c) in camel.chars().enumerate() {
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
