// Prelude with IDE-friendly type aliases
pub use crate::{
    use_state, use_memo, use_effect, State, app,
    rsx, View, Text, Button, Column, Row,
    FlexDirection, JustifyContent, AlignItems,
    BaseView, set_text, force_layout,
};
pub use dyxel_shared::{SizeUnit, px, lp, PxExt, LpExt};

// IDE 提示文档 - 放在 prelude 方便查看
/// # RSX 属性参考
/// 
/// ## View 属性
/// - `width: "100%"` | `200.0` | `"auto"`
/// - `height: "100%"` | `200.0` | `"auto"`
/// - `color: (255, 0, 0)` - RGB 元组
/// - `flexDirection: FlexDirection::Column` | `::Row`
/// - `justifyContent: JustifyContent::Center`
/// - `alignItems: AlignItems::Center`
/// - `padding: (10.0, 20.0, 10.0, 20.0)` - top, right, bottom, left
/// - `margin: (10.0, 20.0, 10.0, 20.0)`
/// - `borderRadius: 8.0`
///
/// ## Text 属性
/// - `fontSize: 16.0`
/// - `fontWeight: 700`
/// - `textColor: (255, 255, 255, 255)` - RGBA
///
/// ## Button 事件
/// - `onTap: move |x, y| { ... }`
pub struct RsxDocs;
