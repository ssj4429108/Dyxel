// Final Demo - 完整计数器应用（rsx! 完整写法）
use dyxel_app::prelude::*;

#[app]
pub fn final_demo() -> impl BaseView {
    rsx! {
        View {

            width: "100%",
            height: "100%",
            color: (50u32, 50, 50, 255),
            flexDirection: FlexDirection::Column,
            justifyContent: JustifyContent::Center,
            alignItems: AlignItems::Center,

            Text("Hello Dyxel!") {
                fontSize: 36.0,
                textColor: (255, 255, 255, 255),
            }
            View {
                height: 20.0,
            }
            Text("rsx! 宏已修复") {
                fontSize: 24.0,
                textColor: (100, 255, 100, 255),
            }
        }
    }
}
