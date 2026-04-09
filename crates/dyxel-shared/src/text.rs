use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextSelection {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextState {
    pub text: String,
    pub selection: TextSelection,
    pub composing: Option<(usize, usize)>,
}

impl TextState {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            selection: TextSelection {
                start: text.len(),
                end: text.len(),
            },
            composing: None,
        }
    }
}
