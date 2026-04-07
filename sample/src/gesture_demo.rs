use dyxel_app::prelude::*;
use dyxel_view::TapGesture;

/// Gesture System V3 Demo
///
/// Demonstrates:
/// - Tap with configurable count (1-N)
/// - LongPress with phase detection
/// - Pan with phase detection (Began/Changed/Ended)
/// - Scale (pinch-to-zoom) with phase detection
/// - Rotation (two-finger rotate) with phase detection
/// - Simultaneous gestures (Pan + Scale + Rotation)
/// - Exclusive gesture composition (Tap vs DoubleTap)
#[app]
pub fn GestureV3Demo() -> impl BaseView {
    // State for tracking gesture events
    let tap_count = use_state(|| 0u32);
    let double_tap_count = use_state(|| 0u32);
    let triple_tap_count = use_state(|| 0u32);
    let long_press_count = use_state(|| 0u32);
    let pan_count = use_state(|| 0u32);

    // Pan tracking
    let pan_x = use_state(|| 0.0f32);
    let pan_y = use_state(|| 0.0f32);

    // Scale tracking
    let current_scale = use_state(|| 1.0f32);

    // Rotation tracking
    let current_rotation = use_state(|| 0.0f32);
    let rotation_delta = use_state(|| 0.0f32);

    // Simultaneous gesture tracking
    let sim_pan_count = use_state(|| 0u32);
    let sim_scale_count = use_state(|| 0u32);
    let sim_combined_active = use_state(|| false);

    // Event log
    let event_log = use_state(|| "Ready...".to_string());

    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (15u32, 15, 25, 255),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            padding: (10.0, 10.0, 10.0, 10.0),

            // ===== Title =====
            Text("Gesture System V3 Demo") {
                fontSize: 24.0,
                textColor: (255u8, 255, 255, 255),
                margin: (0.0, 0.0, 10.0, 0.0),
            }

            Text("Unified gesture API with phase detection") {
                fontSize: 12.0,
                textColor: (150u8, 150, 150, 255),
                margin: (0.0, 0.0, 10.0, 0.0),
            }

            // ===== Configurable Tap Counts =====
            View {
                width: "95%",
                height: 120.0,
                color: (30u32, 30, 45, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Configurable Tap Counts (Unified)") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,

                    // Single Tap
                    View {
                        width: 80.0,
                        height: 60.0,
                        color: (60u32, 180, 120, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let count = tap_count.clone();
                            let log = event_log.clone();
                            move |event| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("Single Tap #{} at ({:.0}, {:.0})", c, event.x, event.y));
                            }
                        },

                        Text("Tap (1)") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("{tap_count}") {
                            fontSize: 16.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }

                    // Double Tap
                    View {
                        width: 80.0,
                        height: 60.0,
                        color: (140u32, 100, 240, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onDoubleTap: {
                            let count = double_tap_count.clone();
                            let log = event_log.clone();
                            move |event| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("Double Tap #{} at ({:.0}, {:.0})", c, event.x, event.y));
                            }
                        },

                        Text("Tap (2)") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("{double_tap_count}") {
                            fontSize: 16.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }

                    // Triple Tap using gesture DSL
                    View {
                        width: 80.0,
                        height: 60.0,
                        color: (240u32, 180, 60, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        gesture: TapGesture::new().count(3).on_gesture_ended({
                            let count = triple_tap_count.clone();
                            let log = event_log.clone();
                            move |event| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("Triple Tap #{} at ({:.0}, {:.0})", c, event.x, event.y));
                            }
                        }),

                        Text("Tap (3)") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("{triple_tap_count}") {
                            fontSize: 16.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
            }

            // ===== Long Press with Phase =====
            View {
                width: "95%",
                height: 80.0,
                color: (45u32, 35, 55, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,

                Text("Long Press with Phase Detection") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                }

                View {
                    width: 200.0,
                    height: 50.0,
                    color: (180u32, 80, 100, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onLongPress: {
                        let count = long_press_count.clone();
                        let log = event_log.clone();
                        move |event| {
                            use dyxel_view::gesture::GesturePhase;
                            match event.phase {
                                GesturePhase::Began => {
                                    log.set("Long Press: Began".to_string());
                                }
                                GesturePhase::Ended => {
                                    let c = count.get() + 1;
                                    count.set(c);
                                    log.set(format!("Long Press #{}: Ended at ({:.0}, {:.0})", c, event.x, event.y));
                                }
                                _ => {}
                            }
                        }
                    },

                    Text("Hold Me ({long_press_count})") {
                        fontSize: 12.0,
                        textColor: (255u8, 255, 255, 255),
                    }
                }
            }

            // ===== Pan with Phase Detection =====
            View {
                width: "95%",
                height: 150.0,
                color: (35u32, 35, 50, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Pan with Phase Detection (Began/Changed/Ended)") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,

                    // Free Pan with phase
                    View {
                        width: 100.0,
                        height: 80.0,
                        color: (60u32, 140, 240, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onPan: {
                            let count = pan_count.clone();
                            let px = pan_x.clone();
                            let py = pan_y.clone();
                            let log = event_log.clone();
                            move |event| {
                                use dyxel_view::gesture::GesturePhase;
                                match event.phase {
                                    GesturePhase::Began => {
                                        log.set(format!("Pan: Began at ({:.0}, {:.0})", event.x, event.y));
                                    }
                                    GesturePhase::Changed => {
                                        let c = count.get() + 1;
                                        count.set(c);
                                        px.set(px.get() + event.delta_x);
                                        py.set(py.get() + event.delta_y);
                                        if c % 10 == 1 {
                                            log.set(format!("Pan: Changed delta({:.0}, {:.0})", event.delta_x, event.delta_y));
                                        }
                                    }
                                    GesturePhase::Ended => {
                                        log.set(format!("Pan: Ended at ({:.0}, {:.0})", event.x, event.y));
                                    }
                                    _ => {}
                                }
                            }
                        },

                        Text("Pan") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("{pan_count}") {
                            fontSize: 16.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }

                    // Pan Position Display
                    View {
                        width: 120.0,
                        height: 80.0,
                        color: (40u32, 40, 55, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,

                        Text("Delta") {
                            fontSize: 10.0,
                            textColor: (150u8, 150, 150, 255),
                        }
                        Text("({pan_x}, {pan_y})") {
                            fontSize: 12.0,
                            textColor: (200u8, 200, 255, 255),
                        }
                    }
                }
            }

            // ===== Scale (Pinch-to-Zoom) =====
            View {
                width: "95%",
                height: 120.0,
                color: (40u32, 45, 55, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Scale (Pinch-to-Zoom) with Phase") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    // Scale Area using onScale
                    View {
                        width: 150.0,
                        height: 80.0,
                        color: (100u32, 60, 180, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onScale: {
                            let scale = current_scale.clone();
                            let log = event_log.clone();
                            move |event| {
                                use dyxel_view::gesture::GesturePhase;
                                match event.phase {
                                    GesturePhase::Began => {
                                        log.set(format!("Scale: Began at {:.2}x", event.scale));
                                    }
                                    GesturePhase::Changed => {
                                        scale.set(event.scale);
                                        log.set(format!("Scale: Changed to {:.2}x", event.scale));
                                    }
                                    GesturePhase::Ended => {
                                        log.set(format!("Scale: Ended at {:.2}x", event.scale));
                                    }
                                    _ => {}
                                }
                            }
                        },

                        Text("Pinch Here") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("Scale: {current_scale}x") {
                            fontSize: 14.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
            }

            // ===== Rotation (Two-Finger Rotate) =====
            View {
                width: "95%",
                height: 120.0,
                color: (45u32, 40, 55, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Rotation (Two-Finger Rotate) with Phase") {
                    fontSize: 14.0,
                    textColor: (200u8, 200, 200, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,

                    // Rotation Area using on_rotation
                    View {
                        width: 150.0,
                        height: 80.0,
                        color: (80u32, 100, 160, 255),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        on_rotation: {
                            let rotation = current_rotation.clone();
                            let rot_delta = rotation_delta.clone();
                            let log = event_log.clone();
                            move |event| {
                                use dyxel_view::gesture::GesturePhase;
                                match event.phase {
                                    GesturePhase::Began => {
                                        log.set(format!("Rotation: Began at {:.1} deg", event.rotation.to_degrees()));
                                    }
                                    GesturePhase::Changed => {
                                        rotation.set(event.rotation);
                                        rot_delta.set(event.delta_rotation);
                                        log.set(format!("Rotation: {:.1} deg (delta: {:.1})",
                                            event.rotation.to_degrees(),
                                            event.delta_rotation.to_degrees()));
                                    }
                                    GesturePhase::Ended => {
                                        log.set(format!("Rotation: Ended at {:.1} deg", event.rotation.to_degrees()));
                                    }
                                    _ => {}
                                }
                            }
                        },

                        Text("Rotate Here") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        Text("Angle: {current_rotation} rad") {
                            fontSize: 14.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
            }

            // ===== Simultaneous Gestures (Pan + Scale) =====
            View {
                width: "95%",
                height: 140.0,
                color: (35u32, 50, 45, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Simultaneous Gestures (Pan + Scale)") {
                    fontSize: 14.0,
                    textColor: (200u8, 255, 200, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }

                Text("Use two fingers to pan AND scale at the same time") {
                    fontSize: 10.0,
                    textColor: (150u8, 200, 150, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                View {
                    width: "100%",
                    flexGrow: 1.0,
                    color: (50u32, 80, 60, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    on_pan: {
                        let count = sim_pan_count.clone();
                        let active = sim_combined_active.clone();
                        let log = event_log.clone();
                        move |event| {
                            use dyxel_view::gesture::GesturePhase;
                            match event.phase {
                                GesturePhase::Began => {
                                    active.set(true);
                                    log.set("Simultaneous: Pan began".to_string());
                                }
                                GesturePhase::Changed => {
                                    count.set(count.get() + 1);
                                }
                                GesturePhase::Ended => {
                                    active.set(false);
                                    log.set(format!("Simultaneous: Pan ended ({} updates)", count.get()));
                                }
                                _ => {}
                            }
                        }
                    },
                    on_scale: {
                        let count = sim_scale_count.clone();
                        let active = sim_combined_active.clone();
                        let log = event_log.clone();
                        move |event| {
                            use dyxel_view::gesture::GesturePhase;
                            match event.phase {
                                GesturePhase::Began => {
                                    active.set(true);
                                    log.set("Simultaneous: Scale began".to_string());
                                }
                                GesturePhase::Changed => {
                                    count.set(count.get() + 1);
                                    if count.get() % 5 == 0 {
                                        log.set(format!("Simultaneous: Scale {:.2}x", event.scale));
                                    }
                                }
                                GesturePhase::Ended => {
                                    active.set(false);
                                    log.set(format!("Simultaneous: Scale ended ({} updates)", count.get()));
                                }
                                _ => {}
                            }
                        }
                    },

                    Text(if sim_combined_active.get() { "Active!" } else { "Pan + Scale" }) {
                        fontSize: 14.0,
                        textColor: if sim_combined_active.get() {
                            (100u8, 255, 100, 255)
                        } else {
                            (200u8, 255, 200, 255)
                        },
                    }
                }
            }

            // ===== Exclusive Gesture Competition =====
            View {
                width: "95%",
                height: 220.0,
                color: (50u32, 40, 40, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),

                Text("Exclusive Gesture Competition") {
                    fontSize: 14.0,
                    textColor: (255u8, 200, 200, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }

                Text("Tap vs DoubleTap vs LongPress vs Pan") {
                    fontSize: 10.0,
                    textColor: (200u8, 150, 150, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }

                // Exclusive Gesture Area with all 4 gesture types
                View {
                    width: "100%",
                    height: 100.0,
                    color: (80u32, 60, 60, 255),
                    borderRadius: 6.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: {
                        let log = event_log.clone();
                        move |event| {
                            log.set(format!("Exclusive: Tap won! at ({:.0}, {:.0})", event.x, event.y));
                        }
                    },
                    onDoubleTap: {
                        let log = event_log.clone();
                        move |event| {
                            log.set(format!("Exclusive: DoubleTap won! at ({:.0}, {:.0})", event.x, event.y));
                        }
                    },
                    onLongPress: {
                        let log = event_log.clone();
                        move |event| {
                            if event.is_began() {
                                log.set(format!("Exclusive: LongPress began at ({:.0}, {:.0})", event.x, event.y));
                            } else if event.is_ended() {
                                log.set(format!("Exclusive: LongPress won! at ({:.0}, {:.0})", event.x, event.y));
                            }
                        }
                    },
                    onPan: {
                        let log = event_log.clone();
                        move |event| {
                            if event.phase == dyxel_view::gesture::GesturePhase::Changed {
                                if event.delta_x.abs() > 5.0 || event.delta_y.abs() > 5.0 {
                                    log.set(format!("Exclusive: Pan won! delta({:.0}, {:.0})", event.delta_x, event.delta_y));
                                }
                            }
                        }
                    },

                    Text("Tap quickly OR DoubleTap OR LongPress OR Pan") {
                        fontSize: 11.0,
                        textColor: (255u8, 200, 200, 255),
                    }
                }

                Text("LongPress Active: {long_press_count}") {
                    fontSize: 10.0,
                    textColor: (200u8, 150, 150, 255),
                    margin: (5.0, 0.0, 0.0, 0.0),
                }
            }

            // ===== All Gestures Overview =====
            View {
                width: "95%",
                height: 60.0,
                color: (30u32, 30, 40, 255),
                borderRadius: 8.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceAround,
                alignItems: AlignItems::Center,

                Text("Tap: {tap_count}") {
                    fontSize: 11.0,
                    textColor: (150u8, 255, 150, 255),
                }
                Text("Dbl: {double_tap_count}") {
                    fontSize: 11.0,
                    textColor: (150u8, 150, 255, 255),
                }
                Text("Tri: {triple_tap_count}") {
                    fontSize: 11.0,
                    textColor: (255u8, 200, 100, 255),
                }
                Text("Long: {long_press_count}") {
                    fontSize: 11.0,
                    textColor: (255u8, 150, 150, 255),
                }
                Text("Pan: {pan_count}") {
                    fontSize: 11.0,
                    textColor: (100u8, 200, 255, 255),
                }
                Text("Scale: {current_scale}x") {
                    fontSize: 11.0,
                    textColor: (200u8, 150, 255, 255),
                }
                Text("Rot: {current_rotation}rad") {
                    fontSize: 11.0,
                    textColor: (150u8, 200, 255, 255),
                }
            }

            // ===== Event Log =====
            View {
                width: "95%",
                flexGrow: 1.0,
                color: (25u32, 25, 35, 255),
                borderRadius: 8.0,
                padding: (10.0, 10.0, 10.0, 10.0),
                flexDirection: FlexDirection::Column,

                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceBetween,
                    margin: (0.0, 0.0, 8.0, 0.0),

                    Text("Event Log") {
                        fontSize: 14.0,
                        textColor: (200u8, 200, 200, 255),
                    }

                    // Clear button
                    View {
                        width: 50.0,
                        height: 24.0,
                        color: (100u32, 100, 100, 255),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let log = event_log.clone();
                            move |_| {
                                log.set("Cleared".to_string());
                            }
                        },

                        Text("Clear") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }

                Text("{event_log}") {
                    fontSize: 12.0,
                    textColor: (180u8, 180, 200, 255),
                }
            }
        }
    }
}
