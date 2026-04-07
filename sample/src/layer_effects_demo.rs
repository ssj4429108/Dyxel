// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layer Effects Demo - 图层效果演示

use dyxel_app::prelude::*;
use dyxel_view::{BaseView, FlexDirection, AlignItems, JustifyContent};

#[app]
pub fn LayerEffectsDemo() -> impl BaseView {
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (245u32, 247, 250, 255),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            justifyContent: JustifyContent::FlexStart,
            padding: (20.0, 20.0, 20.0, 20.0),

            // 标题
            Text {
                value: "Layer Effects Demo",
                fontSize: 24.0,
                textColor: (50u8, 50, 50, 255),
            }
            Text {
                value: "Vello Layer Architecture",
                fontSize: 12.0,
                textColor: (150u8, 150, 150, 255),
            }

            View { width: "100%", height: 20.0 }

            // 1. Opacity
            Text {
                value: "1. Opacity",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 100.0,
                color: (230u32, 235, 240, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                View {
                    width: 80.0,
                    height: 80.0,
                    color: (255u32, 100, 100, 255),
                    opacity: 1.0,
                    Text { value: "100%", fontSize: 14.0, textColor: (255u8, 255, 255, 255) }
                }
                View {
                    width: 80.0,
                    height: 80.0,
                    color: (100u32, 200, 100, 255),
                    opacity: 0.6,
                    Text { value: "60%", fontSize: 14.0, textColor: (255u8, 255, 255, 255) }
                }
                View {
                    width: 80.0,
                    height: 80.0,
                    color: (100u32, 100, 255, 255),
                    opacity: 0.3,
                    Text { value: "30%", fontSize: 14.0, textColor: (255u8, 255, 255, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 2. Shadow
            Text {
                value: "2. Shadow",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 120.0,
                color: (230u32, 235, 240, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    shadow: (4.0, 4.0, 8.0, 0x40000000u32),
                    Text { value: "Soft", fontSize: 12.0, textColor: (100u8, 100, 100, 255) }
                }
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    shadow: (8.0, 8.0, 16.0, 0x80000000u32),
                    Text { value: "Strong", fontSize: 12.0, textColor: (100u8, 100, 100, 255) }
                }
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (100u32, 150, 255, 255),
                    borderRadius: 12.0,
                    shadow: (4.0, 6.0, 12.0, 0x60448AFFu32),
                    Text { value: "Rounded", fontSize: 12.0, textColor: (255u8, 255, 255, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 3. Clip to Bounds - Fixed
            Text {
                value: "3. Clip to Bounds",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 140.0,
                color: (230u32, 235, 240, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                // No Clip - children visible outside
                View {
                    width: 120.0,
                    height: 100.0,
                    color: (255u32, 200, 200, 255),

                    View {
                        width: 50.0,
                        height: 50.0,
                        color: (255u32, 100, 100, 255),
                        position: (-25.0, -25.0),
                    }
                    View {
                        width: 50.0,
                        height: 50.0,
                        color: (255u32, 150, 50, 255),
                        position: (95.0, 75.0),
                    }
                    Text {
                        value: "No Clip",
                        fontSize: 11.0,
                        textColor: (150u8, 50, 50, 255),
                        position: (30.0, 40.0),
                    }
                }

                // Clipped - children clipped at bounds
                View {
                    width: 120.0,
                    height: 100.0,
                    color: (200u32, 255, 200, 255),
                    clip_to_bounds: true,

                    View {
                        width: 50.0,
                        height: 50.0,
                        color: (100u32, 200, 100, 255),
                        position: (-25.0, -25.0),
                    }
                    View {
                        width: 50.0,
                        height: 50.0,
                        color: (50u32, 150, 50, 255),
                        position: (95.0, 75.0),
                    }
                    Text {
                        value: "Clipped",
                        fontSize: 11.0,
                        textColor: (50u8, 150, 50, 255),
                        position: (25.0, 40.0),
                    }
                }

                // Rounded Clip
                View {
                    width: 120.0,
                    height: 100.0,
                    color: (200u32, 200, 255, 255),
                    borderRadius: 20.0,
                    clip_to_bounds: true,

                    View {
                        width: 140.0,
                        height: 120.0,
                        color: (100u32, 100, 255, 255),
                        position: (-10.0, -10.0),
                    }
                    Text {
                        value: "Rounded",
                        fontSize: 11.0,
                        textColor: (50u8, 50, 150, 255),
                        position: (25.0, 40.0),
                    }
                }
            }

            View { width: "100%", height: 20.0 }

            // 4. Blur
            Text {
                value: "4. Blur Effect",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 120.0,
                color: (230u32, 235, 240, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    blur: 0.0,
                    Text { value: "Sharp", fontSize: 14.0, textColor: (100u8, 100, 100, 255) }
                }
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    blur: 2.0,
                    Text { value: "Light", fontSize: 14.0, textColor: (100u8, 100, 100, 255) }
                }
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    blur: 5.0,
                    Text { value: "Medium", fontSize: 14.0, textColor: (100u8, 100, 100, 255) }
                }
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (255u32, 255, 255, 255),
                    blur: 10.0,
                    Text { value: "Heavy", fontSize: 14.0, textColor: (100u8, 100, 100, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 5. Combined Effects - Fixed layout
            Text {
                value: "5. Combined Effects",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 180.0,
                color: (100u32, 80, 120, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                // Card with shadow
                View {
                    width: 120.0,
                    height: 120.0,
                    color: (255u32, 255, 255, 255),
                    borderRadius: 12.0,
                    opacity: 0.95,
                    shadow: (0.0, 6.0, 16.0, 0x40000000u32),
                    flexDirection: FlexDirection::Column,
                    alignItems: AlignItems::Center,
                    justifyContent: JustifyContent::Center,

                    View {
                        width: 50.0,
                        height: 50.0,
                        color: (255u32, 150, 100, 255),
                        borderRadius: 25.0,
                    }
                    Text { value: "Card", fontSize: 12.0, textColor: (100u8, 100, 100, 255) }
                }

                // Frosted glass effect
                View {
                    width: 120.0,
                    height: 120.0,
                    color: (255u32, 255, 255, 180),
                    borderRadius: 12.0,
                    opacity: 0.7,
                    blur: 8.0,
                    shadow: (0.0, 4.0, 12.0, 0x30000000u32),
                    flexDirection: FlexDirection::Column,
                    alignItems: AlignItems::Center,
                    justifyContent: JustifyContent::Center,

                    Text { value: "Frosted", fontSize: 12.0, textColor: (80u8, 80, 80, 255) }
                    Text { value: "Glass", fontSize: 10.0, textColor: (120u8, 120, 120, 255) }
                }

                // All effects combined
                View {
                    width: 120.0,
                    height: 120.0,
                    color: (80u32, 140, 255, 255),
                    borderRadius: 16.0,
                    opacity: 0.9,
                    blur: 2.0,
                    shadow: (4.0, 6.0, 14.0, 0x50448AFFu32),
                    flexDirection: FlexDirection::Column,
                    alignItems: AlignItems::Center,
                    justifyContent: JustifyContent::Center,

                    View {
                        width: 60.0,
                        height: 60.0,
                        color: (255u32, 255, 255, 200),
                        borderRadius: 8.0,
                        opacity: 0.6,
                    }
                    Text { value: "All FX", fontSize: 12.0, textColor: (255u8, 255, 255, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 6. Performance Test
            Text {
                value: "6. Performance Test",
                fontSize: 16.0,
                textColor: (80u8, 80, 80, 255),
            }
            View {
                width: "100%",
                height: 100.0,
                color: (230u32, 235, 240, 255),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                View { width: 45.0, height: 45.0, color: (255u32, 100, 150, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 120, 170, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 140, 190, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 160, 210, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 180, 230, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 200, 250, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 220, 200, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
                View { width: 45.0, height: 45.0, color: (255u32, 240, 150, 255), borderRadius: 8.0, opacity: 0.8, shadow: (2.0, 4.0, 6.0, 0x40000000u32) }
            }

            View { width: "100%", height: 20.0 }

            Text {
                value: "Vello Native Layer Rendering",
                fontSize: 12.0,
                textColor: (100u8, 180, 100, 255),
            }
        }
    }
}
