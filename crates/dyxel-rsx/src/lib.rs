// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! RSX Macro - String Interpolation with Dynamic State Binding

use proc_macro::TokenStream;
use proc_macro2::{TokenTree, Span, Delimiter, Literal};
use quote::{quote, quote_spanned};

/// RSX 宏 - 支持字符串插值和动态 State 绑定
#[proc_macro]
pub fn rsx(input: TokenStream) -> TokenStream {
    let input2 = proc_macro2::TokenStream::from(input);
    
    let first_token = input2.clone().into_iter().next();
    let input_span = first_token.as_ref().map(|t| t.span()).unwrap_or_else(Span::call_site);
    
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
            
            if node_type == "Text" && !args.is_empty() {
                let first_span = args.first().map(|t| t.span()).unwrap_or_else(Span::call_site);
                if let Some(first) = args.first() {
                    let s = first.to_string();
                    if s.starts_with('"') && s.contains('{') && s.contains('}') {
                        props.push(("value_dynamic".to_string(), args, first_span));
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
                                    Some(TokenTree::Punct(p)) if p.as_char() == ',' 
                                        && brace_depth == 0 
                                        && paren_depth == 0 
                                        && bracket_depth == 0
                                        && pipe_depth == 0 => {
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
                                        props.push((format!("{}_dynamic", name), value_tokens, name_span));
                                        continue;
                                    }
                                }
                            }
                            
                            props.push((name, value_tokens, name_span));
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
        let var_name = proc_macro2::Ident::new(&node.node_type, 
            node.var_span.unwrap_or_else(Span::call_site));
        return quote! { #var_name };
    }
    
    let type_span = node.type_span;
    let node_type = proc_macro2::Ident::new(&node.node_type, type_span);
    let node_var = proc_macro2::Ident::new(
        &format!("_{}_node", node.node_type.to_lowercase()),
        Span::call_site()
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
                let method_name = camel_to_snake(name);
                let method_ident = proc_macro2::Ident::new(&method_name, *name_span);
                
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
            let method_name = camel_to_snake(name);
            let method_ident = proc_macro2::Ident::new(&method_name, *name_span);
            
            return quote_spanned! { *name_span =>
                .#method_ident(#value)
            };
        }
        
        let method_name = camel_to_snake(name);
        let method_ident = proc_macro2::Ident::new(&method_name, *name_span);
        
        quote_spanned! { *name_span =>
            .#method_ident(#value)
        }
    }).collect();
    
    // Dynamic bindings - use bind_text for automatic updates
    let dynamic_assignments: Vec<_> = dynamic_bindings.iter().map(|(name, value_tokens, name_span)| {
        let lit = tokens_to_stream(value_tokens);
        let lit_str = lit.to_string();
        
        let (format_str, vars) = parse_interpolation(&lit_str);
        
        if vars.is_empty() {
            // No interpolation, treat as static
            let method_name = camel_to_snake(name);
            let method_ident = proc_macro2::Ident::new(&method_name, *name_span);
            return quote_spanned! { *name_span =>
                .#method_ident(#lit)
            };
        }
        
        // Generate dynamic binding
        let format_lit = Literal::string(&format_str);
        let var_idents: Vec<_> = vars.iter()
            .map(|v| proc_macro2::Ident::new(v, Span::call_site()))
            .collect();
        
        // Generate binding code with to_string() conversion
        quote_spanned! { *name_span =>
            .value({
                let __initial = format!(#format_lit, #(#var_idents.get().to_string()),*);
                __initial
            })
        }
    }).collect();
    
    // Generate post-creation bindings for dynamic text
    let binding_code: Vec<_> = dynamic_bindings.iter().filter_map(|(name, value_tokens, _)| {
        if name != "value" {
            return None; // Only support text value for now
        }
        
        let lit = tokens_to_stream(value_tokens);
        let lit_str = lit.to_string();
        let (_, vars) = parse_interpolation(&lit_str);
        
        if vars.is_empty() {
            return None;
        }
        
        let var_idents: Vec<_> = vars.iter()
            .map(|v| proc_macro2::Ident::new(v, Span::call_site()))
            .collect();
        
        // Generate bind_text calls with to_string conversion
        let binds = var_idents.iter().map(|var| {
            quote! {
                #var.bind_text(#node_var.node_id(), |v| v.to_string());
            }
        });
        Some(quote! { #(#binds)* })
    }).collect();
    
    // Children
    let child_assignments: Vec<_> = node.children.iter().map(|child| {
        let child_expr = expand_node(child);
        quote! {
            let __child = #child_expr;
            #node_var = ::dyxel_view::BaseView::child(#node_var, __child.node_id());
        }
    }).collect();
    
    quote_spanned! { type_span =>
        {
            let mut #node_var = ::dyxel_view::#node_type::new()
                #(#static_assignments)*
                #(#dynamic_assignments)*;
            #(#binding_code)*
            #(#child_assignments)*
            #node_var
        }
    }
}

/// Parse "Count: {count}" -> ("Count: {}", ["count"])
fn parse_interpolation(lit: &str) -> (String, Vec<String>) {
    let mut format_str = String::new();
    let mut vars = Vec::new();
    
    let content = if lit.starts_with('"') && lit.ends_with('"') {
        &lit[1..lit.len()-1]
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
            while let Some(&ch) = chars.peek() {
                if ch == '}' {
                    chars.next();
                    break;
                }
                var_name.push(ch);
                chars.next();
            }
            
            if !var_name.is_empty() {
                format_str.push_str("{}");
                vars.push(var_name);
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
