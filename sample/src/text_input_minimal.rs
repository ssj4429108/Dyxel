// 测试 TextInput 布局
use dyxel_app::prelude::*;
use dyxel_view::{TextInput, TextRenderable};

#[app]
pub fn TextInputMinimal() -> impl BaseView {
    let mut col = Column::new()
        .width("100%")
        .height("100%")
        .color((249, 249, 255, 255))
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(20.0);

    let t1 = Text::new()
        .value("First Text")
        .font_size(24.0)
        .text_color((255, 0, 0, 255));

    // TextInput
    let input = TextInput::new()
        .text_value("Middle Input")
        .font_size(20.0)
        .text_color((0, 255, 0, 255));

    let t2 = Text::new()
        .value("Second Text")
        .font_size(24.0)
        .text_color((0, 0, 255, 255));

    col = col.child(t1);
    col = col.child(input);
    col = col.child(t2);

    col
}
