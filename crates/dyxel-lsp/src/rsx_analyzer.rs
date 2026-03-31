// RSX 语法分析器
// 提供语义分析、跳转定义、自动补全

use tower_lsp::lsp_types::*;
use std::collections::HashMap;

/// RSX 组件定义
#[derive(Clone, Debug)]
pub struct ComponentDef {
    pub name: String,
    pub module_path: String,
    pub file_path: String,
    pub line: u32,
    pub documentation: String,
    pub properties: Vec<PropertyDef>,
}

/// 属性定义
#[derive(Clone, Debug)]
pub struct PropertyDef {
    pub name: String,
    pub ty: String,
    pub documentation: String,
    pub required: bool,
}

/// 文档内容
#[derive(Debug)]
pub struct Document {
    pub uri: Url,
    pub text: String,
    pub version: i32,
}

/// State 变量定义
#[derive(Debug)]
struct StateVar {
    name: String,
    ty: String,
    line: usize,
}

/// RSX 分析器
#[derive(Debug)]
pub struct RsxAnalyzer {
    documents: HashMap<Url, Document>,
    /// 内置组件定义
    components: HashMap<String, ComponentDef>,
}

/// 将 UTF-16 偏移转换为字符索引
fn utf16_to_char_idx(s: &str, utf16_pos: usize) -> usize {
    let mut char_idx = 0;
    let mut utf16_count = 0;
    for c in s.chars() {
        if utf16_count >= utf16_pos {
            break;
        }
        utf16_count += c.len_utf16();
        char_idx += 1;
    }
    char_idx
}

/// 将字符索引转换为 UTF-16 偏移  
fn char_to_utf16_idx(s: &str, char_pos: usize) -> usize {
    let mut utf16_count = 0;
    for (i, c) in s.chars().enumerate() {
        if i >= char_pos {
            break;
        }
        utf16_count += c.len_utf16();
    }
    utf16_count
}

impl RsxAnalyzer {
    pub fn new() -> Self {
        let mut analyzer = Self {
            documents: HashMap::new(),
            components: HashMap::new(),
        };
        analyzer.init_builtin_components();
        analyzer
    }

    fn init_builtin_components(&mut self) {
        // View 组件
        self.components.insert(
            "View".to_string(),
            ComponentDef {
                name: "View".to_string(),
                module_path: "dyxel_view::View".to_string(),
                file_path: "crates/dyxel-view/src/lib.rs".to_string(),
                line: 471,
                documentation: "基础视图容器组件\n\n类似于 HTML 的 div，用于布局和容器".to_string(),
                properties: vec![
                    PropertyDef {
                        name: "width".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "宽度：\"100%\", 200.0, \"auto\"".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "height".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "高度：\"100%\", 200.0, \"auto\"".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "color".to_string(),
                        ty: "(u8, u8, u8)".to_string(),
                        documentation: "背景色 RGB，如 (255, 0, 0)".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "flexDirection".to_string(),
                        ty: "FlexDirection".to_string(),
                        documentation: "Flex 方向：Row | Column".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "justifyContent".to_string(),
                        ty: "JustifyContent".to_string(),
                        documentation: "主轴对齐：FlexStart | Center | FlexEnd | SpaceBetween | SpaceAround".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "alignItems".to_string(),
                        ty: "AlignItems".to_string(),
                        documentation: "交叉轴对齐：FlexStart | Center | FlexEnd | Stretch".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "flexWrap".to_string(),
                        ty: "FlexWrap".to_string(),
                        documentation: "换行：NoWrap | Wrap | WrapReverse".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "alignContent".to_string(),
                        ty: "AlignContent".to_string(),
                        documentation: "多行内容对齐：FlexStart | Center | FlexEnd | Stretch | SpaceBetween | SpaceAround".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "flexGrow".to_string(),
                        ty: "f32".to_string(),
                        documentation: "伸缩增长系数".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "zIndex".to_string(),
                        ty: "i32".to_string(),
                        documentation: "层叠顺序".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "padding".to_string(),
                        ty: "(f32, f32, f32, f32)".to_string(),
                        documentation: "内边距 (top, right, bottom, left)".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "margin".to_string(),
                        ty: "(f32, f32, f32, f32)".to_string(),
                        documentation: "外边距 (top, right, bottom, left)".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "borderRadius".to_string(),
                        ty: "f32".to_string(),
                        documentation: "圆角半径".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "onTap".to_string(),
                        ty: "Fn(f32, f32)".to_string(),
                        documentation: "点击事件回调".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "onLongPress".to_string(),
                        ty: "Fn(f32, f32)".to_string(),
                        documentation: "长按事件回调".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "onClick".to_string(),
                        ty: "Fn()".to_string(),
                        documentation: "点击事件回调（无坐标）".to_string(),
                        required: false,
                    },
                ],
            },
        );

        // Text 组件
        self.components.insert(
            "Text".to_string(),
            ComponentDef {
                name: "Text".to_string(),
                module_path: "dyxel_view::Text".to_string(),
                file_path: "crates/dyxel-view/src/lib.rs".to_string(),
                line: 497,
                documentation: "文本组件\n\n用于显示文字内容".to_string(),
                properties: vec![
                    PropertyDef {
                        name: "value".to_string(),
                        ty: "String".to_string(),
                        documentation: "文本内容".to_string(),
                        required: true,
                    },
                    PropertyDef {
                        name: "fontSize".to_string(),
                        ty: "f32".to_string(),
                        documentation: "字体大小（逻辑像素）".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "fontWeight".to_string(),
                        ty: "u16".to_string(),
                        documentation: "字重：400(normal), 700(bold)".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "textColor".to_string(),
                        ty: "(u8, u8, u8, u8)".to_string(),
                        documentation: "文字颜色 RGBA".to_string(),
                        required: false,
                    },
                    // Text 继承所有 View 的布局属性
                    PropertyDef {
                        name: "width".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "宽度".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "height".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "高度".to_string(),
                        required: false,
                    },
                ],
            },
        );

        // Button 组件
        self.components.insert(
            "Button".to_string(),
            ComponentDef {
                name: "Button".to_string(),
                module_path: "dyxel_view::Button".to_string(),
                file_path: "crates/dyxel-view/src/lib.rs".to_string(),
                line: 567,
                documentation: "按钮组件\n\n可点击的按钮，带默认样式".to_string(),
                properties: vec![
                    PropertyDef {
                        name: "onTap".to_string(),
                        ty: "Fn(f32, f32)".to_string(),
                        documentation: "点击事件回调".to_string(),
                        required: false,
                    },
                    // Button 继承 View 的所有属性
                    PropertyDef {
                        name: "width".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "宽度".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "height".to_string(),
                        ty: "SizeUnit | f32 | &str".to_string(),
                        documentation: "高度".to_string(),
                        required: false,
                    },
                    PropertyDef {
                        name: "color".to_string(),
                        ty: "(u8, u8, u8)".to_string(),
                        documentation: "背景色".to_string(),
                        required: false,
                    },
                ],
            },
        );
    }

    pub fn open_document(&mut self, uri: &Url, text: &str) {
        self.documents.insert(
            uri.clone(),
            Document {
                uri: uri.clone(),
                text: text.to_string(),
                version: 0,
            },
        );
    }

    pub fn update_document(&mut self, uri: &Url, text: &str) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.text = text.to_string();
            doc.version += 1;
        }
    }

    /// 分析文档并返回诊断信息
    pub fn analyze(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        
        if let Some(doc) = self.documents.get(uri) {
            // 简单的 RSX 语法检查
            diagnostics.extend(self.check_rsx_syntax(doc));
        }
        
        diagnostics
    }

    fn check_rsx_syntax(&self, doc: &Document) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let text = &doc.text;
        
        // 检查未闭合的 rsx! 块
        let _open_count = text.matches("rsx!").count();
        let brace_open = text.matches('{').count();
        let brace_close = text.matches('}').count();
        
        if brace_open != brace_close {
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 0 },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: None,
                code_description: None,
                source: Some("dyxel-lsp".to_string()),
                message: format!("括号不匹配：{{ 有 {} 个，}} 有 {} 个", brace_open, brace_close),
                related_information: None,
                tags: None,
                data: None,
            });
        }
        
        // 检查未知的组件名
        for (line_num, line) in text.lines().enumerate() {
            if let Some(_pos) = line.find("rsx!") {
                // 简单启发式：在 rsx! 后面的行中查找大写开头的标识符
                if let Some(caps) = self.find_unknown_components(line) {
                    for (name, col) in caps {
                        if !self.components.contains_key(&name) && name != "rsx" {
                            diagnostics.push(Diagnostic {
                                range: Range {
                                    start: Position { 
                                        line: line_num as u32, 
                                        character: col as u32 
                                    },
                                    end: Position { 
                                        line: line_num as u32, 
                                        character: (col + name.len()) as u32 
                                    },
                                },
                                severity: Some(DiagnosticSeverity::WARNING),
                                code: None,
                                code_description: None,
                                source: Some("dyxel-lsp".to_string()),
                                message: format!("未知的组件：{}，可能是变量引用", name),
                                related_information: None,
                                tags: None,
                                data: None,
                            });
                        }
                    }
                }
            }
        }
        
        diagnostics
    }

    fn find_unknown_components(&self, line: &str) -> Option<Vec<(String, usize)>> {
        let mut results = Vec::new();
        // 简单的正则匹配：找大写开头的单词
        for (_idx, word) in line.split_whitespace().enumerate() {
            if let Some(first) = word.chars().next() {
                if first.is_uppercase() && word.chars().all(|c| c.is_alphanumeric()) {
                    // 找到大写开头的词
                    if let Some(pos) = line.find(word) {
                        results.push((word.trim_end_matches('{').to_string(), pos));
                    }
                }
            }
        }
        if results.is_empty() { None } else { Some(results) }
    }

    /// 查找定义位置
    pub fn find_definition(&self, uri: &Url, position: Position) -> Vec<Location> {
        let mut locations = Vec::new();
        
        if let Some(doc) = self.documents.get(uri) {
            // 首先检查是否在 {state} 插值中
            if let Some(var_name) = self.get_interpolated_var_at_position(&doc.text, position) {
                // 查找变量定义
                if let Some(loc) = self.find_variable_definition(uri, &doc.text, &var_name) {
                    locations.push(loc);
                }
                return locations;
            }
            
            // 获取光标处的词
            if let Some(word) = self.get_word_at_position(&doc.text, position) {
                // 检查是否是已知组件
                if let Some(comp) = self.components.get(&word) {
                    // 返回组件定义位置
                    locations.push(Location {
                        uri: Url::parse(&format!("file://{}", comp.file_path)).unwrap_or_else(|_| uri.clone()),
                        range: Range {
                            start: Position { line: comp.line, character: 0 },
                            end: Position { line: comp.line + 1, character: 0 },
                        },
                    });
                }
            }
        }
        
        locations
    }
    
    /// 检查位置是否在字符串插值 {var} 中，返回变量名
    fn get_interpolated_var_at_position(&self, text: &str, position: Position) -> Option<String> {
        let lines: Vec<_> = text.lines().collect();
        if let Some(line) = lines.get(position.line as usize) {
            // 将 UTF-16 偏移转换为字符索引
            let col = utf16_to_char_idx(line, position.character as usize);
            
            // 查找包含位置的 {var} 模式
            let chars: Vec<_> = line.chars().collect();
            if col >= chars.len() {
                return None;
            }
            
            // 向前查找 {
            let mut start = col;
            while start > 0 && chars[start] != '{' {
                start -= 1;
            }
            
            // 向后查找 }
            let mut end = col;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            
            // 检查是否找到有效的 {var}
            if start < end && chars[start] == '{' && chars.get(end) == Some(&'}') {
                let var_name: String = chars[start+1..end].iter().collect();
                if !var_name.is_empty() && var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    return Some(var_name);
                }
            }
        }
        None
    }
    
    /// 查找变量定义位置
    fn find_variable_definition(&self, uri: &Url, text: &str, var_name: &str) -> Option<Location> {
        // 查找 let var_name = use_state(...) 模式
        let search_pattern = format!("let {}", var_name);
        let state_pattern = format!("{} = use_state", var_name);
        
        for (line_num, line) in text.lines().enumerate() {
            // 查找 let var_name
            if line.contains(&search_pattern) || line.contains(&state_pattern) {
                // 找到变量定义，计算列位置
                if let Some(col) = line.find(var_name) {
                    return Some(Location {
                        uri: uri.clone(),
                        range: Range {
                            start: Position { 
                                line: line_num as u32, 
                                character: col as u32 
                            },
                            end: Position { 
                                line: line_num as u32, 
                                character: (col + var_name.len()) as u32 
                            },
                        },
                    });
                }
            }
        }
        None
    }

    /// 获取插值 {} 中的部分输入
    fn get_partial_var_in_interpolation(&self, text: &str, position: Position) -> Option<String> {
        let lines: Vec<_> = text.lines().collect();
        if let Some(line) = lines.get(position.line as usize) {
            let col = utf16_to_char_idx(line, position.character as usize);
            let chars: Vec<_> = line.chars().collect();
            
            if col >= chars.len() {
                return None;
            }
            
            // 向前查找 {
            let mut start = col;
            while start > 0 && chars[start - 1] != '{' {
                start -= 1;
            }
            
            // 提取 { 和光标之间的字符作为部分输入
            let partial: String = chars[start..col].iter().collect();
            if !partial.is_empty() {
                return Some(partial);
            }
        }
        None
    }

    /// 自动补全
    pub fn complete(&self, uri: &Url, position: Position) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        
        // 获取当前正在输入的词
        let current_word = self.documents.get(uri)
            .and_then(|doc| self.get_word_at_position(&doc.text, position));
        
        // 检查是否在 {} 插值上下文中
        if let Some(doc) = self.documents.get(uri) {
            if self.is_in_interpolation(&doc.text, position) {
                // 在插值 {} 中，提供 state 变量补全
                let partial = self.get_partial_var_in_interpolation(&doc.text, position);
                let state_vars = self.find_state_variables(&doc.text);
                for var in state_vars {
                    // 如果有部分输入，进行过滤
                    if let Some(ref p) = partial {
                        if !var.name.starts_with(p) {
                            continue;
                        }
                    }
                    items.push(CompletionItem {
                        label: var.name.clone(),
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some(format!("State<{}>", var.ty)),
                        documentation: Some(Documentation::String(format!("State variable defined at line {}", var.line + 1))),
                        insert_text: Some(var.name.clone()),
                        ..Default::default()
                    });
                }
                return items;
            }
        }
        
        // 检查是否在属性上下文中
        let in_property_context = self.documents.get(uri)
            .and_then(|doc| self.get_completion_context(&doc.text, position));
        
        if let Some(CompletionContext::Property { component }) = in_property_context {
            // 在属性上下文中，只提供该组件的属性补全
            if let Some(comp) = self.components.get(&component) {
                // 获取已使用的属性名（用于过滤）
                let used_props = self.get_used_properties(uri, position, &component);
                
                for prop in &comp.properties {
                    // 如果属性已被使用，降低优先级但仍显示（可用于修改）
                    let is_used = used_props.contains(&prop.name);
                    
                    items.push(CompletionItem {
                        label: prop.name.clone(),
                        kind: Some(CompletionItemKind::PROPERTY),
                        detail: Some(format!("{} {}", 
                            prop.ty,
                            if prop.required { "(required)" } else { "" }
                        )),
                        documentation: Some(Documentation::String(prop.documentation.clone())),
                        insert_text: Some(format!("{}: ", prop.name)),
                        sort_text: Some(format!("{}{}", 
                            if is_used { "1" } else { "0" },
                            prop.name
                        )), // 已使用的属性排在后面
                        ..Default::default()
                    });
                }
                return items;
            }
        }
        
        // 不在属性上下文中，提供组件补全
        for (name, comp) in &self.components {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::CLASS),
                detail: Some(comp.module_path.clone()),
                documentation: Some(Documentation::String(comp.documentation.clone())),
                ..Default::default()
            });
        }
        
        items
    }
    
    /// 查找文档中定义的 state 变量
    fn find_state_variables(&self, text: &str) -> Vec<StateVar> {
        let mut vars = Vec::new();
        
        for (line_num, line) in text.lines().enumerate() {
            // 匹配 let xxx = use_state(...) 模式
            if let Some(start) = line.find("let ") {
                if let Some(end) = line.find(" = use_state") {
                    let var_name = line[start + 4..end].trim().to_string();
                    if !var_name.is_empty() {
                        // 尝试提取类型
                        let ty = self.extract_state_type(line);
                        vars.push(StateVar {
                            name: var_name,
                            ty,
                            line: line_num,
                        });
                    }
                }
            }
        }
        
        vars
    }
    
    /// 从 use_state 行提取类型
    fn extract_state_type(&self, line: &str) -> String {
        // 匹配 use_state(|| 0) 或 use_state(|| 0u32) 等
        if let Some(start) = line.find("use_state(|| ") {
            let rest = &line[start + 13..];
            if let Some(end) = rest.find(')') {
                let init = &rest[..end];
                // 简单类型推断
                if init.contains('"') {
                    return "String".to_string();
                } else if init.contains('.') {
                    return "f32".to_string();
                } else if init.ends_with("u32") || init.ends_with("u64") {
                    return "u32/u64".to_string();
                } else if init.ends_with("i32") || init.ends_with("i64") {
                    return "i32/i64".to_string();
                } else {
                    return "i32".to_string();
                }
            }
        }
        "unknown".to_string()
    }
    
    /// 检查位置是否在 {} 插值中
    fn is_in_interpolation(&self, text: &str, position: Position) -> bool {
        let lines: Vec<_> = text.lines().collect();
        if let Some(line) = lines.get(position.line as usize) {
            // 将 UTF-16 偏移转换为字符索引
            let col = utf16_to_char_idx(line, position.character as usize);
            let chars: Vec<_> = line.chars().collect();
            
            if col >= chars.len() {
                return false;
            }
            
            // 首先确保在字符串内部（被双引号包围）
            // 找到左边最近的未闭合的 "
            let mut in_string = false;
            let mut i = 0;
            while i < col {
                if chars[i] == '"' {
                    // 检查是否是转义的 \"
                    let mut backslash_count = 0;
                    let mut j = i;
                    while j > 0 && chars[j - 1] == '\\' {
                        backslash_count += 1;
                        j -= 1;
                    }
                    if backslash_count % 2 == 0 {
                        in_string = !in_string;
                    }
                }
                i += 1;
            }
            
            if !in_string {
                return false;
            }
            
            // 向前查找 {
            let mut brace_start = None;
            for i in (0..col).rev() {
                if let Some(c) = chars.get(i) {
                    if *c == '{' {
                        // 检查是否是转义 {{
                        if i > 0 && chars.get(i - 1) == Some(&'{') {
                            continue;
                        }
                        brace_start = Some(i);
                        break;
                    }
                    if *c == '}' {
                        // 已经在一个闭合的插值之后
                        return false;
                    }
                    if *c == '"' {
                        // 到达字符串边界
                        break;
                    }
                }
            }
            
            // 向后查找 }
            if brace_start.is_some() {
                for i in col..chars.len() {
                    if let Some(c) = chars.get(i) {
                        if *c == '}' {
                            return true;
                        }
                        if *c == '"' || *c == '{' {
                            return false;
                        }
                    }
                }
            }
        }
        false
    }

    /// 获取当前组件块中已经使用的属性名
    fn get_used_properties(&self, uri: &Url, position: Position, component: &str) -> Vec<String> {
        let mut used = Vec::new();
        
        if let Some(doc) = self.documents.get(uri) {
            let lines: Vec<_> = doc.text.lines().collect();
            let line_idx = position.line as usize;
            
            // 找到组件定义的行号（从当前行向上查找）
            let mut component_line = None;
            for i in (0..=line_idx).rev() {
                let line = lines[i];
                if line.contains(component) && line.contains('{') {
                    component_line = Some(i);
                    break;
                }
            }
            
            if let Some(start_line) = component_line {
                // 收集从组件定义到当前行的所有属性
                for i in start_line..=line_idx {
                    let line = lines[i];
                    
                    // 简单解析：找 property_name: 模式
                    if let Some(colon_pos) = line.find(':') {
                        let before_colon = &line[..colon_pos];
                        // 提取最后一个词作为属性名
                        if let Some(word) = before_colon.split_whitespace().last() {
                            let prop_name = word.trim();
                            if !prop_name.is_empty() && self.is_valid_property(component, prop_name) {
                                used.push(prop_name.to_string());
                            }
                        }
                    }
                }
            }
        }
        
        used
    }

    /// 检查是否是有效的属性名
    fn is_valid_property(&self, component: &str, property: &str) -> bool {
        if let Some(comp) = self.components.get(component) {
            comp.properties.iter().any(|p| p.name == property)
        } else {
            false
        }
    }

    /// 悬停文档
    pub fn hover(&self, uri: &Url, position: Position) -> Option<Hover> {
        if let Some(doc) = self.documents.get(uri) {
            if let Some(word) = self.get_word_at_position(&doc.text, position) {
                // 组件文档
                if let Some(comp) = self.components.get(&word) {
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!(
                                "## {}\n\n{}\n\n**模块**: `{}`\n\n**属性**:\n{}",
                                comp.name,
                                comp.documentation,
                                comp.module_path,
                                comp.properties.iter()
                                    .map(|p| format!("- `{}`: {} ({})", p.name, p.ty, if p.required { "必需" } else { "可选" }))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            ),
                        }),
                        range: None,
                    });
                }
            }
        }
        None
    }

    fn get_word_at_position(&self, text: &str, position: Position) -> Option<String> {
        let lines: Vec<_> = text.lines().collect();
        if let Some(line) = lines.get(position.line as usize) {
            // 简单的词提取
            let chars: Vec<_> = line.chars().collect();
            // 将 UTF-16 偏移转换为字符索引
            let col = utf16_to_char_idx(line, position.character as usize);
            
            if col >= chars.len() {
                return None;
            }
            
            // 找词的开始
            let mut start = col;
            while start > 0 && chars[start - 1].is_alphanumeric() {
                start -= 1;
            }
            
            // 找词的结束
            let mut end = col;
            while end < chars.len() && chars[end].is_alphanumeric() {
                end += 1;
            }
            
            Some(chars[start..end].iter().collect())
        } else {
            None
        }
    }

    fn get_completion_context(&self, text: &str, position: Position) -> Option<CompletionContext> {
        let lines: Vec<_> = text.lines().collect();
        let line_idx = position.line as usize;
        
        if line_idx >= lines.len() {
            return None;
        }
        
        // 将 UTF-16 偏移转换为字符索引
        let current_line = lines[line_idx];
        let char_col = utf16_to_char_idx(current_line, position.character as usize);
        
        // 步骤1: 计算大括号嵌套深度
        let mut brace_depth = 0i32;
        let mut last_open_brace_line = 0usize;
        
        for i in 0..=line_idx {
            let line = lines[i];
            
            if i == line_idx {
                // 使用字符索引处理当前行
                let chars: Vec<_> = line.chars().collect();
                let check_chars = &chars[..char_col.min(chars.len())];
                for c in check_chars {
                    match c {
                        '{' => {
                            brace_depth += 1;
                            last_open_brace_line = i;
                        }
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                }
            } else {
                for c in line.chars() {
                    match c {
                        '{' => {
                            brace_depth += 1;
                            last_open_brace_line = i;
                        }
                        '}' => brace_depth -= 1,
                        _ => {}
                    }
                }
            }
        }
        
        // 只有在 {} 内部才可能是属性上下文
        if brace_depth <= 0 {
            return None;
        }
        
        // 步骤2: 找到开启当前块的那个组件
        // 从 last_open_brace_line 向上查找组件名
        for i in (0..=last_open_brace_line).rev() {
            let line = lines[i].trim();
            
            // 跳过空行和注释
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            
            // 检查这一行是否有组件名
            for comp_name in self.components.keys() {
                // 查找组件名，确保是完整匹配
                if let Some(pos) = line.find(comp_name) {
                    // 检查组件名前是否是单词边界
                    let before = if pos == 0 {
                        true
                    } else {
                        let prev_char = line.chars().nth(pos - 1).unwrap_or(' ');
                        !prev_char.is_alphanumeric()
                    };
                    
                    // 检查后是否是空格或 {
                    let after_pos = pos + comp_name.len();
                    let after = if after_pos >= line.len() {
                        ""
                    } else {
                        &line[after_pos..]
                    };
                    
                    if before && (after.trim_start().is_empty() || after.trim_start().starts_with('{')) {
                        // 找到了！检查这一行或后面的行有 {
                        // 从这一行到 last_open_brace_line 检查是否有 {
                        for j in i..=last_open_brace_line {
                            if lines[j].contains('{') {
                                return Some(CompletionContext::Property {
                                    component: comp_name.clone(),
                                });
                            }
                        }
                    }
                }
            }
            
            // 如果这一行有单独的 }，可能跳出了当前块
            if line.contains('}') && !line.contains('{') && i < last_open_brace_line {
                // 继续查找，不要 break
            }
        }
        
        // 步骤3: 如果没找到，但确实在 {} 内，尝试通过已知属性推断
        // 检查当前行或附近是否有属性定义（xxx: yyy 模式）
        for i in (0..=line_idx).rev() {
            let line = lines[i];
            if line.contains(':') && !line.trim_start().starts_with("//") {
                // 找到了属性，向上查找组件
                for j in (0..=i).rev() {
                    let check_line = lines[j].trim();
                    for comp_name in self.components.keys() {
                        if check_line.contains(comp_name) && check_line.contains('{') {
                            // 确保不是子组件
                            let open_count = check_line.matches('{').count();
                            let close_count = check_line.matches('}').count();
                            if open_count > close_count {
                                return Some(CompletionContext::Property {
                                    component: comp_name.clone(),
                                });
                            }
                        }
                    }
                    // 遇到闭合括号停止
                    if check_line.contains('}') && !check_line.contains('{') {
                        break;
                    }
                }
                break;
            }
            // 遇到空块或闭合括号停止
            let line_trim = line.trim();
            if line_trim.contains("{}") || (line_trim.contains('}') && !line_trim.contains('{')) {
                break;
            }
        }
        
        None
    }
}

#[allow(dead_code)]
enum CompletionContext {
    Property { component: String },
    Value { property: String },
}

