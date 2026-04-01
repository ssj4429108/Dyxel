use dyxel_app::prelude::*;

#[app]
pub fn GestureDemo() -> impl BaseView {
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (20u32, 20, 30, 255),
            
            View {
                width: 100.0,
                height: 60.0,
                color: (60u32, 180, 60, 255),
                
                Text("Tap") {
                    fontSize: 16.0,
                    textColor: (255, 255, 255, 255),
                }
            }
        }
    }
}
