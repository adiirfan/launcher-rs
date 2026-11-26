//! Android glue shared by the launcher and the gallery. Everything here is
//! Android-only; the testable logic lives in launcher-core and launcher-draw.
#![cfg(target_os = "android")]

pub mod fonts;
pub mod gl_surface;
pub mod holo;
pub mod jni_util;
pub mod refresh;
pub mod render;
pub mod vsync;
pub mod sys;
pub mod update;
pub mod waker;
