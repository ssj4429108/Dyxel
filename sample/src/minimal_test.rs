// 最小化测试 - 只显示一个白色方块
use dyxel_app::prelude::*;

#[app]
pub fn MinimalTest() -> impl BaseView {
    rsx! {
        Column {
            width: "100%",
            height: "100%",
            background: Color::rgb(100, 150, 200), // 蓝色背景

            View {
                width: 200.0,
                height: 100.0,
                color: (255, 255, 255, 255), // 白色方块
                borderRadius: 8.0,
            }

            Text("如果看到白色方块，渲染正常") {
                fontSize: 16.0,
                textColor: (255, 255, 255, 255),
            }
        }
    }
}
